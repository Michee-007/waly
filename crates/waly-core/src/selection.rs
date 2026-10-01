//! Sélection d'outils CONTEXTUELLE et ADAPTATIVE (2026-10-01, backlog R2).
//!
//! Mesuré le 2026-09-30 (Paramètres › Utilisation) : ~3 000 tokens d'entrée
//! par tour, dont le bloc d'outils, et un 4B qui s'égare quand il voit ~25
//! outils (14 appels pour créer une tâche, rappel posé deux fois). Mais
//! d'autres machines font tourner de plus gros modèles, ou un modèle par clé
//! API : eux doivent garder TOUS les outils. D'où :
//!
//! - **Mode** : `auto` (défaut) = sélection pour un petit modèle local
//!   (≤ [`SEUIL_MILLIARDS`]), tous les outils sinon ou si la taille est
//!   inconnue ; `tous` / `selection` forcés par le réglage `outils`.
//! - **Groupes** : chaque outil appartient à un groupe ; le `socle` est
//!   toujours là ; les autres s'ajoutent quand le message les appelle.
//! - **Cumulatif** : un groupe ajouté RESTE jusqu'à la reconstruction de la
//!   fenêtre. Le bloc d'outils précède l'historique dans le prompt : le
//!   changer à chaque tour casserait le cache de préfixe (préfill complet,
//!   ~25 s à 120 tok/s sur le moteur B). Une croissance = un seul défaut de
//!   cache, assumé.

use std::collections::BTreeSet;

/// Au-delà, le modèle est jugé assez solide pour voir tout le catalogue.
pub const SEUIL_MILLIARDS: f32 = 9.0;
pub const SOCLE: &str = "socle";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ModeOutils {
    Tous,
    Selection,
}

/// Mode effectif : réglage (`auto` | `tous` | `selection`) + taille du
/// modèle en milliards de paramètres (si connue).
pub fn mode(reglage: Option<&str>, milliards: Option<f32>) -> ModeOutils {
    match reglage {
        Some("tous") => ModeOutils::Tous,
        Some("selection") => ModeOutils::Selection,
        _ => match milliards {
            Some(b) if b <= SEUIL_MILLIARDS => ModeOutils::Selection,
            _ => ModeOutils::Tous,
        },
    }
}

/// `"4.0B"` → 4.0, `"270M"` → 0.27 (champ `details.parameter_size` d'Ollama).
pub fn milliards_depuis_taille(t: &str) -> Option<f32> {
    let t = t.trim().to_uppercase();
    if let Some(n) = t.strip_suffix('B') {
        return n.trim().parse().ok();
    }
    if let Some(n) = t.strip_suffix('M') {
        return n.trim().parse::<f32>().ok().map(|m| m / 1000.0);
    }
    None
}

/// Taille déduite du NOM quand le moteur ne la donne pas (FLM) :
/// `qwen3vl-it:4b` → 4, `llama3.1:70b` → 70, `gemma3:270m` → 0.27.
pub fn milliards_depuis_nom(nom: &str) -> Option<f32> {
    let tag = nom.rsplit(':').next()?.to_lowercase();
    for morceau in tag.split(|c: char| c == '-' || c == '_') {
        if let Some(n) = morceau.strip_suffix('b') {
            if let Ok(v) = n.parse::<f32>() {
                return Some(v);
            }
        }
        if let Some(n) = morceau.strip_suffix('m') {
            if let Ok(v) = n.parse::<f32>() {
                return Some(v / 1000.0);
            }
        }
    }
    None
}

/// Le groupe d'un outil. Inconnu → `socle` (un outil qu'on ne sait pas
/// classer reste visible : jamais d'amputation silencieuse).
pub fn groupe(outil: &str) -> String {
    let g = match outil {
        "heure" | "get_time" | "memoriser" | "chercher_memoire" | "oublier" | "echo" => SOCLE,
        // Montré seulement quand une demande attend VRAIMENT (le 4B l'appelait
        // à vide : « aucun id fourni », vécu 2026-10-01). L'hôte ajoute le groupe.
        crate::tools::RESOLVE_TOOL => "attentes",
        "creer_note" | "chercher_notes" | "lister_notes" => "notes",
        "creer_tache" | "lister_taches" | "maj_tache" => "taches",
        "poser_rappel" | "lister_rappels" | "annuler_rappel" => "rappels",
        "lire_fichier" | "ecrire_fichier" | "lister_fichiers" | "chercher_fichiers" => "fichiers",
        "regarder" => "vision",
        "lire_ecran" | "agir_ecran" | "regarder_ecran" => "ecran",
        "envoyer_message" => "messages",
        // Un seul groupe pour les connecteurs : leurs noms peuvent contenir
        // « _ » ou « - » (plugins), le découpage serait fragile.
        o if o.starts_with("mcp_") => "mcp",
        _ => SOCLE,
    };
    g.to_string()
}

/// Minuscules sans accents (comparaisons de mots-clés).
pub fn normaliser(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            '’' => '\'',
            c => c,
        })
        .collect()
}

const MOTS: &[(&str, &[&str])] = &[
    ("notes", &["note", "noter", "pense-bete", "carnet"]),
    ("taches", &["tache", "todo", "a faire", "choses a faire", "liste de travail"]),
    (
        "rappels",
        &["rappel", "rappelle", "previens", "alarme", "reveille", "souviens-moi", "n'oublie pas de me", "dans une minute", "dans une heure"],
    ),
    (
        "fichiers",
        &[
            "fichier", "document", "dossier", ".md", ".txt", ".docx", ".pdf", ".xlsx", ".pptx", ".csv",
            "rapport", "ecris", "redige", "enregistre", "sauvegarde", "word", "excel", "powerpoint",
            "tableur", "presentation", "diapo", "telecharg",
        ],
    ),
    ("vision", &["regarde", "vois", "camera", "photo", "montre-moi", "a quoi je ressemble"]),
    ("ecran", &["ecran", "fenetre", "clique", "tape dans", "appli", "application"]),
    ("messages", &["message", "envoie", "mail", "courriel", "ecris a"]),
];

/// Groupes appelés par un message (mots-clés) ; `connecteurs` = noms des
/// serveurs MCP branchés, appelés par leur nom ou par « connecteur ».
pub fn groupes_pour(message: &str, connecteurs: &[String]) -> BTreeSet<String> {
    let m = normaliser(message);
    let mut g = BTreeSet::new();
    for (groupe, mots) in MOTS {
        if mots.iter().any(|k| m.contains(k)) {
            g.insert(groupe.to_string());
        }
    }
    for c in connecteurs {
        if m.contains(&normaliser(c)) || m.contains("connecteur") {
            g.insert("mcp".to_string());
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_adaptatif_au_cerveau() {
        assert_eq!(mode(None, Some(4.0)), ModeOutils::Selection);
        assert_eq!(mode(Some("auto"), Some(32.0)), ModeOutils::Tous);
        assert_eq!(mode(None, None), ModeOutils::Tous, "modèle inconnu (cloud) : tout");
        assert_eq!(mode(Some("tous"), Some(4.0)), ModeOutils::Tous);
        assert_eq!(mode(Some("selection"), Some(70.0)), ModeOutils::Selection);
    }

    #[test]
    fn tailles_lues() {
        assert_eq!(milliards_depuis_taille("4.0B"), Some(4.0));
        assert_eq!(milliards_depuis_taille("270M"), Some(0.27));
        assert_eq!(milliards_depuis_taille("?"), None);
        assert_eq!(milliards_depuis_nom("qwen3vl-it:4b"), Some(4.0));
        assert_eq!(milliards_depuis_nom("llama3.1:70b-instruct-q4"), Some(70.0));
        assert_eq!(milliards_depuis_nom("qwen3:4b-instruct-2507-q4_K_M"), Some(4.0));
        assert_eq!(milliards_depuis_nom("mon-modele"), None);
    }

    #[test]
    fn groupes_et_mots_cles() {
        assert_eq!(groupe("poser_rappel"), "rappels");
        assert_eq!(groupe("mcp_documents_list_directory"), "mcp");
        assert_eq!(groupe("outil_futur"), SOCLE);
        let g = groupes_pour("Rappelle-moi demain de relire le Rapport.docx", &[]);
        assert!(g.contains("rappels") && g.contains("fichiers"));
        assert!(groupes_pour("Pourquoi le ciel est-il bleu ?", &[]).is_empty());
        let g = groupes_pour("liste mes documents via le connecteur", &["documents".into()]);
        assert!(g.contains("mcp"));
        assert!(groupes_pour("Ajoute une tâche « budget »", &[]).contains("taches"));
    }
}

#[cfg(test)]
mod tests_registre {
    use super::*;
    use crate::tools::Registry;

    #[test]
    fn le_registre_filtre_puis_elargit_sans_jamais_retirer() {
        let conn = std::rc::Rc::new(crate::store::open(":memory:").unwrap());
        let mut r = Registry::new();
        crate::native_tools::register_core_tools(&mut r, conn, None);
        let tous = r.specs().len();
        r.selectionner(Some(BTreeSet::new()));
        let socle: Vec<String> = r.specs().into_iter().map(|s| s.name).collect();
        assert!(socle.len() < tous, "le socle est plus court que le catalogue");
        assert!(socle.iter().all(|n| groupe(n) == SOCLE), "{socle:?}");
        assert!(socle.iter().any(|n| n == "memoriser"));
        assert!(r.elargir(["rappels".to_string()]));
        assert!(!r.elargir(["rappels".to_string()]), "déjà là : pas de changement");
        assert!(r.specs().iter().any(|s| s.name == "poser_rappel"));
        r.selectionner(None);
        assert_eq!(r.specs().len(), tous);
        assert!(!r.elargir(["notes".to_string()]), "mode tous : rien à élargir");
    }
}

#[cfg(test)]
mod tests_garde {
    use crate::chat::garde_appels;
    use crate::llm::ToolCall;

    fn appel(id: &str, name: &str, args: &str) -> ToolCall {
        ToolCall { id: id.into(), name: name.into(), arguments: args.into() }
    }

    #[test]
    fn doublons_et_rafales_bloques() {
        let mut vus = std::collections::HashSet::new();
        assert!(garde_appels(&mut vus, &appel("1", "poser_rappel", r#"{"titre":"eau","quand":"10:25"}"#)).is_none());
        // Même appel, clés dans un autre ordre : doublon.
        assert!(garde_appels(&mut vus, &appel("2", "poser_rappel", r#"{"quand":"10:25","titre":"eau"}"#)).is_some());
        assert!(garde_appels(&mut vus, &appel("3", "lister_rappels", "{}")).is_none());
        assert!(garde_appels(&mut vus, &appel("4", "heure", "{}")).is_none());
        assert!(garde_appels(&mut vus, &appel("5", "memoriser", r#"{"cle":"a"}"#)).is_none());
        assert!(garde_appels(&mut vus, &appel("6", "chercher_memoire", "{}")).is_some(), "5e appel distinct : plafond");
    }
}
