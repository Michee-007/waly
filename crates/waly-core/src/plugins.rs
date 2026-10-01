//! Plugins (lot 2 des « bientôt », 2026-09-30) : un DOSSIER qui apporte des
//! compétences et des connecteurs MCP prêts à l'emploi, installé par
//! l'utilisateur — jamais en arrière-plan, jamais depuis le réseau (un dépôt
//! en ligne demande une sortie : lot 3).
//!
//! Manifeste `waly-plugin.toml` (même sous-ensemble TOML que waly.toml) :
//! ```toml
//! [plugin]
//! nom = "Aide à la rédaction"
//! description = "Relire, résumer, reformuler"
//! version = "1.0.0"
//!
//! [competence.relire]
//! titre = "Relire un texte"
//! declencheur = "quand on demande de relire ou corriger un texte"
//! recette = "1. ... 2. ..."
//!
//! [mcp.outil]            # optionnel : un programme local (stdio)
//! commande = "node"
//! arg1 = "serveur.js"    # relatif au dossier du plugin
//! ```
//!
//! Règles gravées :
//! - installation = COPIE dans `data/plugins/<slug>` (la source peut
//!   disparaître) ; le système de fichiers est la vérité (pas de table) :
//!   un fichier `.desactive` éteint le plugin ;
//! - un connecteur de plugin exige TOUJOURS une approbation par appel
//!   (`confiance` du manifeste ignorée : c'est à l'utilisateur de faire
//!   confiance, pas au plugin de se l'accorder) ;
//! - ses compétences entrent au catalogue sous la clé `plugin:<slug>:<nom>`
//!   et en sortent à la désactivation/désinstallation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::mcp::ConfigServeur;
use crate::store;

pub const MANIFESTE: &str = "waly-plugin.toml";
const MARQUE_DESACTIVE: &str = ".desactive";
/// Garde-fous de copie : un plugin est un petit dossier de texte/scripts.
const TAILLE_MAX: u64 = 50 * 1024 * 1024;
const FICHIERS_MAX: usize = 2000;
const PROFONDEUR_MAX: usize = 8;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CompetencePlugin {
    pub nom: String,
    pub titre: String,
    pub declencheur: String,
    pub recette: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ConnecteurPlugin {
    pub nom: String,
    /// La commande telle que le manifeste la déclare + ses arguments : ce
    /// qui SERA LANCÉ — montré à l'utilisateur avant installation.
    pub commande: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Plugin {
    pub slug: String,
    pub nom: String,
    pub description: String,
    pub version: String,
    pub dossier: String,
    pub actif: bool,
    pub competences: Vec<CompetencePlugin>,
    pub connecteurs: Vec<ConnecteurPlugin>,
}

/// Dossier des plugins installés (`data/plugins`, surcharge WALY_PLUGINS).
pub fn racine() -> PathBuf {
    std::env::var("WALY_PLUGINS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| crate::chemins::data().join("plugins"))
}

/// Identifiant de dossier sûr : minuscules, chiffres, tirets.
pub fn slug(nom: &str) -> String {
    let s: String = nom
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c if c.is_ascii_alphanumeric() => c,
            _ => '-',
        })
        .collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    s.chars().take(48).collect()
}

/// Lit le manifeste d'un dossier (pure hors I/O du fichier). Erreur si le
/// manifeste manque ou n'a pas de `[plugin] nom`.
pub fn lire(dossier: &Path) -> Result<Plugin, String> {
    let texte = std::fs::read_to_string(dossier.join(MANIFESTE))
        .map_err(|_| format!("pas de {MANIFESTE} dans ce dossier"))?;
    depuis_texte(&texte, dossier)
}

pub fn depuis_texte(texte: &str, dossier: &Path) -> Result<Plugin, String> {
    let t: HashMap<String, String> = crate::config::parse(texte);
    let g = |k: &str| t.get(k).cloned().unwrap_or_default();
    let nom = g("plugin.nom");
    if nom.trim().is_empty() {
        return Err("manifeste sans [plugin] nom".into());
    }
    let mut competences = Vec::new();
    for c in crate::config::sections_sous_dans(&t, "competence") {
        let k = |cle: &str| g(&format!("competence.{c}.{cle}"));
        if k("titre").is_empty() || k("recette").is_empty() {
            continue;
        }
        competences.push(CompetencePlugin {
            nom: c.clone(),
            titre: k("titre"),
            declencheur: k("declencheur"),
            recette: k("recette"),
        });
    }
    let mut connecteurs = Vec::new();
    for m in crate::config::sections_sous_dans(&t, "mcp") {
        let section = format!("mcp.{m}");
        if let Some(cfg) = crate::mcp::config_depuis(&m, |cle| t.get(&format!("{section}.{cle}")).cloned()) {
            connecteurs.push(ConnecteurPlugin { nom: m.clone(), commande: cfg.commande, args: cfg.args });
        }
    }
    Ok(Plugin {
        slug: slug(&nom),
        nom,
        description: g("plugin.description"),
        version: g("plugin.version"),
        dossier: dossier.to_string_lossy().into_owned(),
        actif: !dossier.join(MARQUE_DESACTIVE).exists(),
        competences,
        connecteurs,
    })
}

/// Plugins installés (dossiers de `racine()` qui portent un manifeste).
pub fn installes() -> Vec<Plugin> {
    let Ok(rd) = std::fs::read_dir(racine()) else { return Vec::new() };
    let mut v: Vec<Plugin> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| lire(&e.path()).ok())
        .collect();
    v.sort_by(|a, b| a.nom.to_lowercase().cmp(&b.nom.to_lowercase()));
    v
}

fn copier(src: &Path, dst: &Path, profondeur: usize, n: &mut usize, taille: &mut u64) -> Result<(), String> {
    if profondeur > PROFONDEUR_MAX {
        return Err("dossier trop profond".into());
    }
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for e in std::fs::read_dir(src).map_err(|e| e.to_string())?.flatten() {
        let meta = std::fs::symlink_metadata(e.path()).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            continue; // jamais suivre un lien hors du dossier
        }
        let cible = dst.join(e.file_name());
        if meta.is_dir() {
            let nom = e.file_name().to_string_lossy().to_lowercase();
            if nom == ".git" || nom == "node_modules" {
                continue;
            }
            copier(&e.path(), &cible, profondeur + 1, n, taille)?;
        } else {
            *n += 1;
            *taille += meta.len();
            if *n > FICHIERS_MAX || *taille > TAILLE_MAX {
                return Err("plugin trop volumineux (2 000 fichiers / 50 Mo max)".into());
            }
            std::fs::copy(e.path(), &cible).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Installe (ou met à jour) depuis un dossier : copie, puis compétences au
/// catalogue. Les connecteurs se branchent au prochain démarrage de Waly.
pub fn installer(conn: &Connection, source: &Path) -> Result<Plugin, String> {
    let p = lire(source)?;
    if p.slug.is_empty() {
        return Err("nom de plugin invalide".into());
    }
    let dest = racine().join(&p.slug);
    if dest.exists() {
        retirer_competences(conn, &p.slug);
        std::fs::remove_dir_all(&dest).map_err(|e| format!("mise à jour impossible : {e}"))?;
    }
    let (mut n, mut taille) = (0usize, 0u64);
    if let Err(e) = copier(source, &dest, 0, &mut n, &mut taille) {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(e);
    }
    let installe = lire(&dest)?;
    ajouter_competences(conn, &installe);
    Ok(installe)
}

fn cle(slug: &str, nom: &str) -> String {
    format!("plugin:{slug}:{nom}")
}

fn ajouter_competences(conn: &Connection, p: &Plugin) {
    for c in &p.competences {
        store::upsert_competence(conn, &cle(&p.slug, &c.nom), &c.titre, &c.declencheur, &c.recette).ok();
    }
}

fn retirer_competences(conn: &Connection, slug: &str) {
    if let Ok(l) = store::list_competences(conn) {
        let prefixe = format!("plugin:{slug}:");
        for c in l.iter().filter(|c| c.key.starts_with(&prefixe)) {
            store::delete_competence(conn, &c.key).ok();
        }
    }
}

fn dossier_installe(slug: &str) -> Result<PathBuf, String> {
    // Le slug vient de l'UI : on le re-normalise (pas de « .. » possible).
    let d = racine().join(self::slug(slug));
    if !d.join(MANIFESTE).exists() {
        return Err("plugin introuvable".into());
    }
    Ok(d)
}

/// Active / désactive : compétences ajoutées ou retirées tout de suite,
/// connecteurs au prochain démarrage.
pub fn activer(conn: &Connection, slug: &str, actif: bool) -> Result<(), String> {
    let d = dossier_installe(slug)?;
    let marque = d.join(MARQUE_DESACTIVE);
    if actif {
        let _ = std::fs::remove_file(&marque);
        ajouter_competences(conn, &lire(&d)?);
    } else {
        std::fs::write(&marque, b"").map_err(|e| e.to_string())?;
        retirer_competences(conn, &self::slug(slug));
    }
    Ok(())
}

pub fn desinstaller(conn: &Connection, slug: &str) -> Result<(), String> {
    let d = dossier_installe(slug)?;
    retirer_competences(conn, &self::slug(slug));
    std::fs::remove_dir_all(&d).map_err(|e| e.to_string())
}

/// Connecteurs MCP des plugins ACTIFS, prêts à brancher : arguments relatifs
/// résolus dans le dossier du plugin, commande relative aussi si le fichier
/// existe là, et approbation FORCÉE à chaque appel.
pub fn serveurs() -> Vec<ConfigServeur> {
    let mut v = Vec::new();
    for p in installes().into_iter().filter(|p| p.actif) {
        let d = PathBuf::from(&p.dossier);
        let resoudre = |s: &str| {
            let c = d.join(s);
            if Path::new(s).is_relative() && c.exists() { c.to_string_lossy().into_owned() } else { s.to_string() }
        };
        let texte = std::fs::read_to_string(d.join(MANIFESTE)).unwrap_or_default();
        let t = crate::config::parse(&texte);
        for m in crate::config::sections_sous_dans(&t, "mcp") {
            let section = format!("mcp.{m}");
            if let Some(mut cfg) = crate::mcp::config_depuis(&m, |cle| t.get(&format!("{section}.{cle}")).cloned()) {
                cfg.nom = format!("{}-{m}", p.slug);
                cfg.commande = resoudre(&cfg.commande);
                cfg.args = cfg.args.iter().map(|a| resoudre(a)).collect();
                cfg.lecture = false;
                v.push(cfg);
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXEMPLE: &str = r#"
[plugin]
nom = "Aide à la rédaction"
description = "Relire et résumer"
version = "1.0.0"

[competence.relire]
titre = "Relire un texte"
declencheur = "quand on demande de relire"
recette = "1. lire 2. corriger"

[competence.vide]
titre = "Sans recette"

[mcp.outil]
commande = "node"
arg1 = "serveur.js"
confiance = "lecture"
"#;

    #[test]
    fn manifeste_lu_competences_valides_et_programmes_montres() {
        let p = depuis_texte(EXEMPLE, Path::new("/tmp/x")).unwrap();
        assert_eq!(p.slug, "aide-a-la-redaction");
        assert_eq!(p.competences.len(), 1);
        assert_eq!(p.connecteurs[0].commande, "node");
        assert_eq!(p.connecteurs[0].args, vec!["serveur.js".to_string()]);
        assert!(depuis_texte("[plugin]\ndescription = \"x\"", Path::new("/tmp")).is_err());
    }

    #[test]
    fn installer_activer_desinstaller_sans_trace() {
        let base = std::env::temp_dir().join(format!("waly-plugins-test-{}", std::process::id()));
        let src = base.join("source");
        std::fs::create_dir_all(src.join("sous")).unwrap();
        std::fs::write(src.join(MANIFESTE), EXEMPLE).unwrap();
        std::fs::write(src.join("sous").join("serveur.js"), "// rien").unwrap();
        std::env::set_var("WALY_PLUGINS", base.join("installes"));
        let conn = store::open(":memory:").unwrap();

        let p = installer(&conn, &src).unwrap();
        assert!(Path::new(&p.dossier).join("sous").join("serveur.js").exists());
        assert!(store::get_competence(&conn, "plugin:aide-a-la-redaction:relire").unwrap().is_some());
        let s = serveurs();
        assert_eq!(s.len(), 1);
        assert!(!s[0].lecture, "un plugin ne s'accorde jamais la confiance");
        assert_eq!(s[0].nom, "aide-a-la-redaction-outil");

        activer(&conn, &p.slug, false).unwrap();
        assert!(serveurs().is_empty());
        assert!(store::get_competence(&conn, "plugin:aide-a-la-redaction:relire").unwrap().is_none());
        activer(&conn, &p.slug, true).unwrap();
        assert!(store::get_competence(&conn, "plugin:aide-a-la-redaction:relire").unwrap().is_some());

        desinstaller(&conn, &p.slug).unwrap();
        assert!(installes().is_empty());
        assert!(store::list_competences(&conn).unwrap().is_empty());
        assert!(desinstaller(&conn, "../../etc").is_err());
        std::fs::remove_dir_all(&base).ok();
        std::env::remove_var("WALY_PLUGINS");
    }
}
