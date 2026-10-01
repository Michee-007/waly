//! Les MAINS sur les fichiers (Mains v1) — outils fichiers réels.
//!
//! Waly lit l'explorateur réel et crée/modifie de VRAIS fichiers, pas
//! seulement des notes en base. Règles de sûreté :
//! - LECTURE libre (rien ne quitte la machine — modèle local, huis clos) ;
//! - ÉCRITURE = outil SENSIBLE (`irreversible`) → passe TOUJOURS par une
//!   approbation humaine (gate de risque de tools.rs), limitée aux formats
//!   TEXTE (allowlist d'extensions — jamais d'exécutable) et hors des zones
//!   système ;
//! - un chemin RELATIF se résout dans le dossier documents de Waly
//!   (`%USERPROFILE%\Documents\Waly`, surcharge `WALY_DOCS`) — c'est là que
//!   naissent les documents montrés au Carnet.
//!
//! Formats : texte (md/txt/html/csv/json…) écrit tel quel, et documents
//! GÉNÉRÉS depuis le markdown selon l'extension — .docx, .xlsx, .pptx,
//! .pdf (module `documents`, Rust pur). L'indexation du disque est au
//! ROADMAP.

use std::path::{Path, PathBuf};

use crate::llm::ToolSpec;
use crate::safety::Capabilities;
use crate::tools::{Registry, Tool};

/// Extensions AUTORISÉES à l'écriture (texte seulement — jamais de binaire ni
/// d'exécutable ; l'allowlist prime sur tout).
const EXTENSIONS_ECRITURE: &[&str] = &[
    "md", "txt", "html", "htm", "css", "js", "ts", "json", "csv", "tsv",
    "xml", "yaml", "yml", "toml", "ini", "py", "rs", "sql", "tex", "svg",
    // Documents générés par Waly lui-même (jamais de .docm/.xlsm : pas de macros).
    "docx", "xlsx", "pptx", "pdf",
];

/// Politique de chemins — INJECTABLE (les tests posent leur racine tempdir).
#[derive(Clone)]
pub struct PolitiqueFichiers {
    /// Racine des chemins relatifs et du Carnet (« Documents Waly »).
    pub racine: PathBuf,
    /// Préfixes INTERDITS à l'écriture (zones système, données de Waly).
    pub interdits_ecriture: Vec<PathBuf>,
}

impl PolitiqueFichiers {
    /// Politique réelle de la machine : racine documents + zones système.
    pub fn defaut() -> Self {
        Self { racine: racine_documents(), interdits_ecriture: interdits_systeme() }
    }

    /// Résout l'entrée du modèle : absolue telle quelle, relative sous la
    /// racine. Refuse les `..` (échappement de racine par traversée).
    pub fn resoudre(&self, entree: &str) -> Result<PathBuf, String> {
        let entree = entree.trim();
        if entree.is_empty() {
            return Err("chemin vide".into());
        }
        if entree.contains("..") {
            return Err("chemin refuse (« .. » interdit)".into());
        }
        let p = Path::new(entree);
        Ok(if p.is_absolute() { p.to_path_buf() } else { self.racine.join(p) })
    }

    /// Un chemin est-il écrivable selon la politique ? (extension texte
    /// autorisée ET hors zones interdites.)
    pub fn ecriture_autorisee(&self, chemin: &Path) -> Result<(), String> {
        let ext = chemin
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase)
            .unwrap_or_default();
        if !EXTENSIONS_ECRITURE.contains(&ext.as_str()) {
            return Err(format!(
                "extension .{ext} refusee a l'ecriture (formats texte seulement : {})",
                EXTENSIONS_ECRITURE.join(", ")
            ));
        }
        // Comparaison sur une forme NORMALISÉE (casse, séparateurs) et par
        // composant entier. Vécu 2026-10-01 (CI Windows) : comparés en texte
        // brut, `C:/Windows/x.md` ne « commençait » pas par `C:\Windows` —
        // un chemin écrit avec des `/` contournait toutes les zones protégées.
        let cle = |p: &Path| {
            p.to_string_lossy().to_lowercase().replace('\\', "/").trim_end_matches('/').to_string()
        };
        let bas = cle(chemin);
        for zone in &self.interdits_ecriture {
            let z = cle(zone);
            if !z.is_empty() && (bas == z || bas.starts_with(&format!("{z}/"))) {
                return Err(format!("zone protegee ({}) : ecriture refusee", zone.display()));
            }
        }
        Ok(())
    }
}

/// `%USERPROFILE%\Documents\Waly` (Windows) ou `~/Documents/Waly`, surcharge
/// `WALY_DOCS`. N'est PAS créé ici (création paresseuse à la 1ʳᵉ écriture).
pub fn racine_documents() -> PathBuf {
    if let Ok(d) = std::env::var("WALY_DOCS") {
        if !d.trim().is_empty() {
            return PathBuf::from(d);
        }
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    Path::new(&home).join("Documents").join("Waly")
}

/// Zones système interdites à l'écriture (assemblées depuis l'environnement
/// réel ; sur un hôte non-Windows la plupart sont simplement absentes).
fn interdits_systeme() -> Vec<PathBuf> {
    let mut z: Vec<PathBuf> = Vec::new();
    for var in ["SystemRoot", "ProgramFiles", "ProgramFiles(x86)", "ProgramData"] {
        if let Ok(v) = std::env::var(var) {
            z.push(PathBuf::from(v));
        }
    }
    // Les données de Waly (base, souvenirs) et ses moteurs : jamais touchés
    // par un outil — seul le code du produit y écrit.
    z.push(crate::chemins::data());
    z.push(crate::chemins::engines());
    if let Ok(v) = std::env::var("LOCALAPPDATA") {
        z.push(PathBuf::from(v)); // profils navigateurs, app installée…
    }
    z
}

/// Enregistre les outils fichiers (Mains v1).
pub fn register_fichier_tools(registry: &mut Registry, politique: PolitiqueFichiers) {
    let p = std::rc::Rc::new(politique);
    registry.register(Box::new(ListerFichiers { p: p.clone() }));
    registry.register(Box::new(LireFichier { p: p.clone() }));
    registry.register(Box::new(ChercherFichiers { p: p.clone() }));
    registry.register(Box::new(EcrireFichier { p }));
}

fn s(args: &serde_json::Value, key: &str) -> Result<String, String> {
    args[key]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("champ manquant: {key}"))
}

fn obj_schema(properties: serde_json::Value, required: &[&str]) -> serde_json::Value {
    serde_json::json!({ "type": "object", "properties": properties, "required": required })
}

// ── lister_fichiers ─────────────────────────────────────────────────────────

struct ListerFichiers {
    p: std::rc::Rc<PolitiqueFichiers>,
}
impl Tool for ListerFichiers {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "lister_fichiers".into(),
            description: "Liste un dossier reel du disque (defaut : Documents Waly).".into(),
            parameters: obj_schema(
                serde_json::json!({"dossier": {"type": "string"}}),
                &[],
            ),
        }
    }
    fn run(&self, args: &serde_json::Value) -> Result<String, String> {
        let dossier = match s(args, "dossier") {
            Ok(d) => self.p.resoudre(&d)?,
            Err(_) => self.p.racine.clone(),
        };
        let mut entrees = match std::fs::read_dir(&dossier) {
            Ok(rd) => rd.flatten().collect::<Vec<_>>(),
            Err(e) => return Ok(format!("dossier illisible ({}) : {e}", dossier.display())),
        };
        entrees.sort_by_key(|e| e.file_name());
        let mut lignes = vec![format!("{} :", dossier.display())];
        for e in entrees.iter().take(60) {
            let nom = e.file_name().to_string_lossy().to_string();
            match e.file_type() {
                Ok(t) if t.is_dir() => lignes.push(format!("[dossier] {nom}")),
                _ => {
                    let taille = e.metadata().map(|m| m.len()).unwrap_or(0);
                    lignes.push(format!("{nom} ({taille} octets)"));
                }
            }
        }
        if entrees.len() > 60 {
            lignes.push(format!("… et {} autres entrees", entrees.len() - 60));
        }
        if entrees.is_empty() {
            lignes.push("(vide)".into());
        }
        Ok(lignes.join("\n"))
    }
}

// ── lire_fichier ────────────────────────────────────────────────────────────

const LECTURE_MAX: usize = 15_000;

struct LireFichier {
    p: std::rc::Rc<PolitiqueFichiers>,
}
impl Tool for LireFichier {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "lire_fichier".into(),
            description: "Lit un fichier texte reel du disque.".into(),
            parameters: obj_schema(
                serde_json::json!({"chemin": {"type": "string"}}),
                &["chemin"],
            ),
        }
    }
    fn run(&self, args: &serde_json::Value) -> Result<String, String> {
        let chemin = self.p.resoudre(&s(args, "chemin")?)?;
        // Documents Office (zip, donc « binaires ») : leur TEXTE via l'aperçu
        // (fichiers de référence des projets, lot 2).
        let ext = chemin.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if ["docx", "xlsx", "pptx"].contains(&ext.as_str()) {
            let t = crate::apercu::texte(&chemin)?;
            if t.chars().count() > LECTURE_MAX {
                let coupe: String = t.chars().take(LECTURE_MAX).collect();
                return Ok(format!("{coupe}\n… (tronque)"));
            }
            return Ok(t);
        }
        let octets = std::fs::read(&chemin)
            .map_err(|e| format!("lecture impossible ({}) : {e}", chemin.display()))?;
        if octets.iter().take(4096).filter(|&&b| b == 0).count() > 0 {
            return Ok(format!(
                "{} est un fichier binaire ({} octets) — lecture texte impossible",
                chemin.display(),
                octets.len()
            ));
        }
        let texte = String::from_utf8_lossy(&octets);
        if texte.chars().count() > LECTURE_MAX {
            let coupe: String = texte.chars().take(LECTURE_MAX).collect();
            return Ok(format!("{coupe}\n… (tronque, fichier de {} octets)", octets.len()));
        }
        Ok(texte.into_owned())
    }
}

// ── chercher_fichiers ───────────────────────────────────────────────────────

struct ChercherFichiers {
    p: std::rc::Rc<PolitiqueFichiers>,
}
impl Tool for ChercherFichiers {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "chercher_fichiers".into(),
            description: "Cherche des fichiers par nom sous un dossier (defaut : Documents Waly)."
                .into(),
            parameters: obj_schema(
                serde_json::json!({
                    "motif": {"type": "string"},
                    "dossier": {"type": "string"},
                }),
                &["motif"],
            ),
        }
    }
    fn run(&self, args: &serde_json::Value) -> Result<String, String> {
        let motif = s(args, "motif")?.to_lowercase();
        if motif.trim().is_empty() {
            return Err("motif vide".into());
        }
        let racine = match s(args, "dossier") {
            Ok(d) => self.p.resoudre(&d)?,
            Err(_) => self.p.racine.clone(),
        };
        let mut resultats = Vec::new();
        let mut visites = 0usize;
        chercher_rec(&racine, &motif, 0, &mut visites, &mut resultats);
        if resultats.is_empty() {
            return Ok(format!("aucun fichier contenant « {motif} » sous {}", racine.display()));
        }
        Ok(resultats.join("\n"))
    }
}

/// Parcours borné : profondeur ≤ 6, ≤ 4000 dossiers visités, ≤ 40 résultats —
/// jamais un scan sans fin du disque dans un tour de conversation.
fn chercher_rec(
    dossier: &Path,
    motif: &str,
    prof: usize,
    visites: &mut usize,
    resultats: &mut Vec<String>,
) {
    if prof > 6 || *visites > 4000 || resultats.len() >= 40 {
        return;
    }
    *visites += 1;
    let Ok(rd) = std::fs::read_dir(dossier) else { return };
    for e in rd.flatten() {
        if resultats.len() >= 40 {
            return;
        }
        let nom = e.file_name().to_string_lossy().to_string();
        // Les dossiers cachés/techniques n'apportent que du bruit.
        if nom.starts_with('.') || nom == "node_modules" || nom == "target" {
            continue;
        }
        let chemin = e.path();
        if nom.to_lowercase().contains(motif) {
            resultats.push(chemin.display().to_string());
        }
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            chercher_rec(&chemin, motif, prof + 1, visites, resultats);
        }
    }
}

// ── ecrire_fichier (SENSIBLE → approbation humaine) ─────────────────────────

struct EcrireFichier {
    p: std::rc::Rc<PolitiqueFichiers>,
}
impl Tool for EcrireFichier {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "ecrire_fichier".into(),
            description: "Cree ou remplace un fichier reel. Texte (md, html, csv, txt...) ou \
                          document Word/Excel/PowerPoint/PDF selon l'extension (.docx, .xlsx, \
                          .pptx, .pdf) : contenu en markdown ; xlsx = tableau markdown ou CSV ; \
                          pptx = diapos separees par ---. Chemin relatif = dossier Documents Waly."
                .into(),
            parameters: obj_schema(
                serde_json::json!({
                    "chemin": {"type": "string"},
                    "contenu": {"type": "string"},
                }),
                &["chemin", "contenu"],
            ),
        }
    }
    fn capabilities(&self) -> Capabilities {
        // Remplacer un fichier existant ne se défait pas → confirmation
        // humaine AVANT chaque écriture (gate de risque, cartes HITL).
        Capabilities { writes_memory: true, irreversible: true, ..Default::default() }
    }
    fn run(&self, args: &serde_json::Value) -> Result<String, String> {
        let chemin = self.p.resoudre(&s(args, "chemin")?)?;
        self.p.ecriture_autorisee(&chemin)?;
        let contenu = s(args, "contenu")?;
        if let Some(parent) = chemin.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("creation du dossier ({}) : {e}", parent.display()))?;
        }
        let existait = chemin.exists();
        // Document Office/PDF : généré depuis le markdown ; sinon texte brut.
        let ext = chemin.extension().and_then(|e| e.to_str()).unwrap_or("");
        let octets = crate::documents::generer(ext, &contenu).unwrap_or_else(|| contenu.into_bytes());
        std::fs::write(&chemin, &octets)
            .map_err(|e| format!("ecriture impossible ({}) : {e}", chemin.display()))?;
        Ok(format!(
            "{} : {} ({} octets)",
            if existait { "remplace" } else { "cree" },
            chemin.display(),
            octets.len()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::Risk;

    fn politique_test(racine: &Path) -> PolitiqueFichiers {
        PolitiqueFichiers {
            racine: racine.to_path_buf(),
            interdits_ecriture: vec![racine.join("zone-protegee")],
        }
    }

    fn tempdir(nom: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("waly-fichiers-{nom}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn relatif_sous_racine_et_traversee_refusee() {
        let p = politique_test(Path::new("/racine"));
        assert_eq!(p.resoudre("rapport.md").unwrap(), Path::new("/racine").join("rapport.md"));
        assert!(p.resoudre("../hors").is_err());
        assert!(p.resoudre("").is_err());
    }

    #[test]
    fn ecriture_allowlist_et_zones() {
        let p = politique_test(Path::new("/racine"));
        assert!(p.ecriture_autorisee(Path::new("/racine/doc.md")).is_ok());
        assert!(p.ecriture_autorisee(Path::new("/racine/data.csv")).is_ok());
        // Extensions exécutables/binaires : refus, quel que soit le chemin.
        assert!(p.ecriture_autorisee(Path::new("/racine/x.exe")).is_err());
        assert!(p.ecriture_autorisee(Path::new("/racine/x.ps1")).is_err());
        assert!(p.ecriture_autorisee(Path::new("/racine/sans-extension")).is_err());
        // Zone protégée : refus même en .md.
        assert!(p.ecriture_autorisee(Path::new("/racine/zone-protegee/n.md")).is_err());
        // Séparateurs et casse mêlés (Windows) : la zone tient quand même.
        let w = PolitiqueFichiers {
            racine: PathBuf::from(r"C:\Users\x\Documents\Waly"),
            interdits_ecriture: vec![PathBuf::from(r"C:\Windows"), PathBuf::from(r"C:\waly\data")],
        };
        for chemin in [r"C:\Windows\System32\n.md", "C:/Windows/System32/n.md", "c:/windows/n.md", r"C:\waly/data\n.md"] {
            assert!(w.ecriture_autorisee(Path::new(chemin)).is_err(), "{chemin}");
        }
        // Un dossier voisin au nom plus long n'est PAS la zone.
        assert!(w.ecriture_autorisee(Path::new(r"C:\WindowsNotes\n.md")).is_ok());
        assert!(w.ecriture_autorisee(Path::new(r"C:\Users\x\Documents\Waly\n.md")).is_ok());
    }

    #[test]
    fn ecrire_est_sensible_lire_ne_l_est_pas() {
        let p = std::rc::Rc::new(politique_test(Path::new("/racine")));
        assert_eq!(EcrireFichier { p: p.clone() }.capabilities().risk(), Risk::Sensitive);
        assert_eq!(LireFichier { p: p.clone() }.capabilities().risk(), Risk::Read);
        assert_eq!(ListerFichiers { p }.capabilities().risk(), Risk::Read);
    }

    #[test]
    fn cycle_ecrire_lister_lire_chercher() {
        let racine = tempdir("cycle");
        let p = std::rc::Rc::new(politique_test(&racine));
        let ecrire = EcrireFichier { p: p.clone() };
        let out = ecrire
            .run(&serde_json::json!({"chemin": "notes/rapport.md", "contenu": "# Bonjour"}))
            .unwrap();
        assert!(out.starts_with("cree"), "{out}");
        // Remplacement signalé comme tel.
        let out2 = ecrire
            .run(&serde_json::json!({"chemin": "notes/rapport.md", "contenu": "# Bonsoir"}))
            .unwrap();
        assert!(out2.starts_with("remplace"), "{out2}");

        let lu = LireFichier { p: p.clone() }
            .run(&serde_json::json!({"chemin": "notes/rapport.md"}))
            .unwrap();
        assert_eq!(lu, "# Bonsoir");

        let liste = ListerFichiers { p: p.clone() }
            .run(&serde_json::json!({"dossier": "notes"}))
            .unwrap();
        assert!(liste.contains("rapport.md"), "{liste}");

        let trouve = ChercherFichiers { p }
            .run(&serde_json::json!({"motif": "rapport"}))
            .unwrap();
        assert!(trouve.contains("rapport.md"), "{trouve}");
        let _ = std::fs::remove_dir_all(&racine);
    }

    #[test]
    fn documents_generes_selon_l_extension() {
        let racine = tempdir("documents");
        let ecrire = EcrireFichier { p: std::rc::Rc::new(politique_test(&racine)) };
        for (nom, signature) in [
            ("plan.docx", &b"PK\x03\x04"[..]),
            ("budget.xlsx", &b"PK\x03\x04"[..]),
            ("revue.pptx", &b"PK\x03\x04"[..]),
            ("note.pdf", &b"%PDF-"[..]),
            ("note.md", &b"# Plan"[..]),
        ] {
            let out = ecrire
                .run(&serde_json::json!({"chemin": nom, "contenu": "# Plan\n- un"}))
                .unwrap();
            assert!(out.starts_with("cree"), "{out}");
            let octets = std::fs::read(racine.join(nom)).unwrap();
            assert!(octets.starts_with(signature), "{nom}");
            // L'issue annonce la VRAIE taille écrite (carte de fichier de l'UI).
            assert!(out.contains(&format!("({} octets)", octets.len())), "{out}");
        }
        let _ = std::fs::remove_dir_all(&racine);
    }
}
