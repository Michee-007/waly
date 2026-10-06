//! La Garde : ce que Waly TOUCHE, et ce qu'on lui a coupé.
//!
//! Deux choses, tenues au même endroit pour qu'elles ne puissent pas diverger :
//!
//! - le **registre** : chaque fois que Waly touche à une ressource de la
//!   machine (un fichier, sa mémoire, l'écran, la caméra, le micro), une ligne
//!   est écrite. On note QUOI (un chemin, un geste), jamais le contenu ;
//! - les **accès** : l'utilisateur peut couper une ressource. La coupure est
//!   vérifiée au point de passage unique des outils (`tools::Registry`) et à
//!   l'ouverture des sessions micro, caméra et écran. Une tentative coupée est
//!   refusée ET notée.
//!
//! Tout est local (table `registre`, réglages `garde_acces_*`). Ce module ne
//! concerne que Waly : pour les autres agents de la machine, voir
//! `agents_machine` (découverte) et `sceau` (réseau).

use rusqlite::Connection;
use serde::Serialize;

/// Une ressource de la machine que Waly peut toucher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Ressource {
    Fichiers,
    Memoire,
    Ecran,
    Camera,
    Micro,
}

impl Ressource {
    pub const TOUTES: [Ressource; 5] =
        [Ressource::Fichiers, Ressource::Memoire, Ressource::Ecran, Ressource::Camera, Ressource::Micro];

    /// Clé stable (base, interface).
    pub fn cle(self) -> &'static str {
        match self {
            Ressource::Fichiers => "fichiers",
            Ressource::Memoire => "memoire",
            Ressource::Ecran => "ecran",
            Ressource::Camera => "camera",
            Ressource::Micro => "micro",
        }
    }

    /// Nom affiché.
    pub fn nom(self) -> &'static str {
        match self {
            Ressource::Fichiers => "Fichiers",
            Ressource::Memoire => "Mémoire",
            Ressource::Ecran => "Écran",
            Ressource::Camera => "Caméra",
            Ressource::Micro => "Micro",
        }
    }

    pub fn depuis(cle: &str) -> Option<Ressource> {
        Ressource::TOUTES.into_iter().find(|r| r.cle() == cle)
    }
}

/// À quelle ressource touche un outil ? `None` : à aucune ressource gardée
/// (l'heure, les notes et tâches que Waly tient lui-même, un rappel).
pub fn ressource_de(outil: &str) -> Option<Ressource> {
    match outil {
        "lister_fichiers" | "lire_fichier" | "chercher_fichiers" | "ecrire_fichier" => Some(Ressource::Fichiers),
        "memoriser" | "chercher_memoire" | "oublier" => Some(Ressource::Memoire),
        "regarder_ecran" | "lire_ecran" | "agir_ecran" => Some(Ressource::Ecran),
        "regarder" => Some(Ressource::Camera),
        _ => None,
    }
}

/// Ce que le registre retient d'un appel d'outil : le geste et, pour un
/// fichier, son chemin. JAMAIS le contenu lu, écrit, vu ou mémorisé.
pub fn decrire(outil: &str, args: &serde_json::Value) -> String {
    let champ = |noms: &[&str]| -> Option<String> {
        noms.iter()
            .find_map(|n| args.get(*n).and_then(|v| v.as_str()))
            .map(|s| court(s, 140))
    };
    let chemin = || champ(&["chemin", "fichier", "dossier", "nom"]);
    match outil {
        "lire_fichier" => format!("a lu {}", chemin().unwrap_or_else(|| "un fichier".into())),
        "ecrire_fichier" => format!("a écrit {}", chemin().unwrap_or_else(|| "un fichier".into())),
        "lister_fichiers" => format!("a listé {}", chemin().unwrap_or_else(|| "un dossier".into())),
        "chercher_fichiers" => "a cherché dans tes fichiers".into(),
        "memoriser" => "a noté un souvenir".into(),
        "chercher_memoire" => "a consulté ses souvenirs".into(),
        "oublier" => "a effacé un souvenir".into(),
        "regarder_ecran" => "a regardé l'écran".into(),
        "lire_ecran" => "a lu le texte de l'écran".into(),
        "agir_ecran" => "a agi sur l'écran".into(),
        "regarder" => "a regardé par la caméra".into(),
        autre => format!("a utilisé {autre}"),
    }
}

fn court(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

/// Ce que Waly dit au modèle quand un accès est coupé : il doit l'annoncer
/// simplement, sans chercher un autre chemin.
pub fn refus(r: Ressource) -> String {
    format!(
        "l'acces a « {} » est coupe par l'utilisateur dans la Garde. Dis-le-lui simplement et ne cherche pas a contourner.",
        r.nom()
    )
}

/// Table du registre (appelée à l'ouverture de la base).
pub fn migrer(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS registre (
           id        INTEGER PRIMARY KEY,
           agent     TEXT NOT NULL DEFAULT 'Waly',
           ressource TEXT NOT NULL,
           detail    TEXT NOT NULL DEFAULT '',
           refuse    INTEGER NOT NULL DEFAULT 0,
           at        TEXT NOT NULL DEFAULT (datetime('now','localtime'))
         );
         CREATE INDEX IF NOT EXISTS idx_registre_at ON registre(at);",
    )
}

fn cle_acces(r: Ressource) -> String {
    format!("garde_acces_{}", r.cle())
}

/// L'accès à cette ressource est-il permis ? Permis par défaut ; seule une
/// coupure explicite le retire.
pub fn permis(conn: &Connection, r: Ressource) -> bool {
    crate::store::reglage(conn, &cle_acces(r)).as_deref() != Some("0")
}

/// Coupe ou rend un accès, et l'inscrit au registre.
pub fn regler(conn: &Connection, r: Ressource, permettre: bool) -> rusqlite::Result<()> {
    crate::store::set_reglage(conn, &cle_acces(r), if permettre { "" } else { "0" })?;
    conn.execute(
        "INSERT INTO registre (agent, ressource, detail) VALUES ('Toi', ?1, ?2)",
        rusqlite::params![
            r.cle(),
            if permettre { format!("as rendu à Waly l'accès à {}", r.nom()) } else { format!("as coupé à Waly l'accès à {}", r.nom()) }
        ],
    )?;
    Ok(())
}

/// Inscrit un geste de Waly. `refuse` : la tentative a été refusée (accès coupé).
pub fn noter(conn: &Connection, r: Ressource, detail: &str, refuse: bool) {
    let _ = conn.execute(
        "INSERT INTO registre (ressource, detail, refuse) VALUES (?1, ?2, ?3)",
        rusqlite::params![r.cle(), detail, refuse as i64],
    );
}

/// Ce que le service a VU d'un autre programme (Garde, étape 3). `genre` et
/// `objet` viennent de l'observation ; rendu : (ressource, phrase).
pub fn dire_observation(genre: &str, objet: &str) -> (&'static str, String) {
    match genre {
        "ouvert" => ("fichiers", format!("a ouvert {objet}")),
        "cree" => ("fichiers", format!("a créé {objet}")),
        "ecrit" => ("fichiers", format!("a écrit {objet}")),
        "supprime" => ("fichiers", format!("a supprimé {objet}")),
        "renomme" => ("fichiers", format!("a renommé {objet}")),
        "lance" => ("programmes", format!("a lancé {objet}")),
        "connecte" => ("internet", format!("s'est connecté à {objet}")),
        autre => ("fichiers", format!("{autre} {objet}")),
    }
}

/// Inscrit le geste d'un AUTRE programme. Le même geste répété dans la
/// journée ne fait qu'une ligne, remontée en tête.
pub fn noter_agent(conn: &Connection, agent: &str, ressource: &str, detail: &str) {
    // Vécu 2026-10-06 : on ne changeait que la date de la ligne. Le fil, lu
    // par numéro, la laissait enfouie sous des centaines de lignes plus
    // récentes : un geste refait n'apparaissait plus. La ligne du jour est
    // donc retirée puis réinscrite : elle reprend la tête pour de bon.
    let _ = conn.execute(
        "DELETE FROM registre WHERE agent = ?1 AND detail = ?2 AND date(at) = date('now','localtime')",
        rusqlite::params![agent, detail],
    );
    let _ = conn.execute(
        "INSERT INTO registre (agent, ressource, detail) VALUES (?1, ?2, ?3)",
        rusqlite::params![agent, ressource, detail],
    );
}

/// Combien de gestes de cet agent aujourd'hui, par ressource.
pub fn vus_par(conn: &Connection, agent: &str, ressource: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM registre WHERE agent = ?1 AND ressource = ?2 AND date(at) = date('now','localtime')",
        rusqlite::params![agent, ressource],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

/// Vérifie l'accès ET tient le registre, d'un seul geste : `Ok` si permis (la
/// touche est notée), `Err(message)` si coupé (le refus est noté).
pub fn passer(conn: &Connection, r: Ressource, detail: &str) -> Result<(), String> {
    if permis(conn, r) {
        noter(conn, r, detail, false);
        Ok(())
    } else {
        noter(conn, r, &format!("{detail} : refusé, accès coupé"), true);
        Err(refus(r))
    }
}

/// Une ligne du registre.
#[derive(Debug, Clone, Serialize)]
pub struct Ligne {
    pub at: String,
    pub agent: String,
    pub ressource: String,
    pub detail: String,
    pub refuse: bool,
}

/// Les lignes les plus récentes d'abord.
pub fn lignes(conn: &Connection, limite: i64) -> rusqlite::Result<Vec<Ligne>> {
    let mut st = conn.prepare(
        "SELECT at, agent, ressource, detail, refuse FROM registre ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = st.query_map([limite], |r| {
        Ok(Ligne { at: r.get(0)?, agent: r.get(1)?, ressource: r.get(2)?, detail: r.get(3)?, refuse: r.get::<_, i64>(4)? != 0 })
    })?;
    rows.collect()
}

/// (choses touchées, refus) par Waly aujourd'hui.
pub fn comptes_du_jour(conn: &Connection) -> (i64, i64) {
    conn.query_row(
        "SELECT COALESCE(SUM(refuse = 0), 0), COALESCE(SUM(refuse = 1), 0) FROM registre
         WHERE agent = 'Waly' AND date(at) = date('now','localtime')",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap_or((0, 0))
}

/// Dernier geste de Waly sur chaque ressource (pour le graphe).
pub fn dernier_par_ressource(conn: &Connection, r: Ressource) -> Option<Ligne> {
    conn.query_row(
        "SELECT at, agent, ressource, detail, refuse FROM registre
         WHERE agent = 'Waly' AND ressource = ?1 ORDER BY id DESC LIMIT 1",
        [r.cle()],
        |x| Ok(Ligne { at: x.get(0)?, agent: x.get(1)?, ressource: x.get(2)?, detail: x.get(3)?, refuse: x.get::<_, i64>(4)? != 0 }),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE reglages (cle TEXT PRIMARY KEY, valeur TEXT NOT NULL, updated_at TEXT NOT NULL DEFAULT (datetime('now')));",
        )
        .unwrap();
        migrer(&c).unwrap();
        c
    }

    #[test]
    fn un_geste_refait_remonte_en_tete_du_fil_meme_sous_des_centaines_de_lignes() {
        let c = base();
        noter_agent(&c, "powershell.exe", "fichiers", r"a ouvert C:\waly\README.md");
        for i in 0..400 {
            noter_agent(&c, "chrome.exe", "fichiers", &format!("a écrit C:/cache/{i}"));
        }
        assert!(!lignes(&c, 300).unwrap().iter().any(|l| l.detail.contains("README")), "enfoui : hors des 300 dernières");
        // Le même geste, refait : une seule ligne, et en tête.
        noter_agent(&c, "powershell.exe", "fichiers", r"a ouvert C:\waly\README.md");
        let l = lignes(&c, 300).unwrap();
        assert!(l[0].detail.contains("README"), "{}", l[0].detail);
        assert_eq!(vus_par(&c, "powershell.exe", "fichiers"), 1, "toujours une seule ligne pour ce geste");
    }

    #[test]
    fn chaque_outil_qui_touche_la_machine_a_sa_ressource() {
        assert_eq!(ressource_de("lire_fichier"), Some(Ressource::Fichiers));
        assert_eq!(ressource_de("ecrire_fichier"), Some(Ressource::Fichiers));
        assert_eq!(ressource_de("chercher_memoire"), Some(Ressource::Memoire));
        assert_eq!(ressource_de("regarder"), Some(Ressource::Camera));
        assert_eq!(ressource_de("regarder_ecran"), Some(Ressource::Ecran));
        assert_eq!(ressource_de("agir_ecran"), Some(Ressource::Ecran));
        // Ce que Waly tient lui-même n'est pas une ressource gardée.
        assert_eq!(ressource_de("heure"), None);
        assert_eq!(ressource_de("creer_note"), None);
        assert_eq!(ressource_de("outil_invente"), None);
    }

    #[test]
    fn le_registre_note_le_chemin_jamais_le_contenu() {
        let a = serde_json::json!({"chemin": "C:\\docs\\budget.xlsx", "contenu": "SECRET 42"});
        let d = decrire("ecrire_fichier", &a);
        assert_eq!(d, "a écrit C:\\docs\\budget.xlsx");
        assert!(!d.contains("SECRET"));
        let m = serde_json::json!({"cle": "sante", "valeur": "allergie aux noix"});
        let d = decrire("memoriser", &m);
        assert!(!d.contains("allergie") && !d.contains("sante"), "{d}");
        assert_eq!(decrire("lire_fichier", &serde_json::json!({})), "a lu un fichier");
    }

    #[test]
    fn permis_par_defaut_coupe_sur_demande_et_rendu() {
        let c = base();
        for r in Ressource::TOUTES {
            assert!(permis(&c, r), "{r:?} doit être permis par défaut");
        }
        regler(&c, Ressource::Ecran, false).unwrap();
        assert!(!permis(&c, Ressource::Ecran));
        assert!(permis(&c, Ressource::Micro), "couper l'écran ne coupe pas le micro");
        regler(&c, Ressource::Ecran, true).unwrap();
        assert!(permis(&c, Ressource::Ecran));
    }

    #[test]
    fn passer_note_la_touche_ou_le_refus() {
        let c = base();
        assert!(passer(&c, Ressource::Fichiers, "a lu a.txt").is_ok());
        regler(&c, Ressource::Fichiers, false).unwrap();
        let e = passer(&c, Ressource::Fichiers, "a lu b.txt").unwrap_err();
        assert!(e.contains("Fichiers"), "{e}");
        let l = lignes(&c, 10).unwrap();
        // Le plus récent d'abord : le refus, la coupure (par « Toi »), la touche.
        assert!(l[0].refuse && l[0].detail.contains("b.txt"));
        assert_eq!(l[1].agent, "Toi");
        assert!(!l[2].refuse && l[2].detail == "a lu a.txt");
        assert_eq!(comptes_du_jour(&c), (1, 1));
        assert_eq!(dernier_par_ressource(&c, Ressource::Fichiers).unwrap().detail, l[0].detail);
        assert!(dernier_par_ressource(&c, Ressource::Camera).is_none());
    }

    #[test]
    fn un_autre_agent_a_son_registre_sans_doublon() {
        let c = base();
        let (r, d) = dire_observation("ouvert", r"C:\docs\a.txt");
        assert_eq!((r, d.as_str()), ("fichiers", r"a ouvert C:\docs\a.txt"));
        assert_eq!(dire_observation("connecte", "1.1.1.1:443"), ("internet", "s'est connecté à 1.1.1.1:443".to_string()));
        assert_eq!(dire_observation("lance", r"C:\x\ping.exe").0, "programmes");
        noter_agent(&c, "Ollama", r, &d);
        noter_agent(&c, "Ollama", r, &d); // le même geste : une seule ligne
        noter_agent(&c, "Ollama", "internet", "s'est connecté à 1.1.1.1:443");
        assert_eq!(vus_par(&c, "Ollama", "fichiers"), 1);
        assert_eq!(vus_par(&c, "Ollama", "internet"), 1);
        assert_eq!(vus_par(&c, "Hermes", "fichiers"), 0);
        // Les gestes des autres ne comptent pas dans ceux de Waly.
        assert_eq!(comptes_du_jour(&c), (0, 0));
    }

    #[test]
    fn les_cles_font_l_aller_retour() {
        for r in Ressource::TOUTES {
            assert_eq!(Ressource::depuis(r.cle()), Some(r));
        }
        assert_eq!(Ressource::depuis("internet"), None);
    }
}
