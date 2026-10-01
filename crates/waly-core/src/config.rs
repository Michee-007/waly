//! `waly.toml` — configuration déclarée, UN fichier à côté de la base.
//!
//! Parité plateforme (mandat 2026-09-10, comparatif Hermes agent au
//! RESEARCH) : tout serveur OpenAI-compatible local (FLM, Ollama, llama.cpp,
//! LM Studio…) se déclare ici, sans poser de variables d'environnement.
//! Priorité GRAVÉE : env explicite › waly.toml › défaut/sonde — une variable
//! d'environnement posée n'est JAMAIS contredite par le fichier (elle reste
//! le geste le plus explicite, et les spawns desktop→voix en héritent).
//!
//! Sous-ensemble TOML volontaire (aucune dépendance neuve — piège 3 SAC,
//! budget) : sections `[nom]`, paires `cle = valeur` (chaîne "...", nombre,
//! booléen), commentaires `#`. Pas de tableaux ni de tables imbriquées : le
//! jour où une clé en a besoin, on prendra la crate `toml` — pas avant.
//!
//! Emplacement : `WALY_CONFIG` › `<racine>/data/waly.toml` (chemins.rs —
//! Windows : `C:\waly\data\waly.toml`). Le serveur LLM
//! reste sur le LOOPBACK (huis clos : le sceau ne laisse vivre que
//! 127.0.0.1) — l'absence de clé `hote` est un choix, pas un oubli.
//!
//! Clés servies aujourd'hui :
//!   [llm]         port = 42626, modele = "qwen3vl-it:4b"
//!   [utilisateur] nom = "Prenom"
//!   [stockage]    base = "C:\\chemin\\waly.db"

use std::collections::HashMap;
use std::sync::OnceLock;

/// Chemin du fichier de configuration (`WALY_CONFIG` pour surcharger).
pub fn chemin() -> String {
    std::env::var("WALY_CONFIG").unwrap_or_else(|_| crate::chemins::data_fichier("waly.toml"))
}

/// Table `section.cle → valeur`, lue UNE fois par processus (les tours se
/// re-paient chaque token : pas d'I/O fichier par tour). Fichier absent ou
/// illisible = table vide, silencieux : waly.toml est optionnel.
fn table() -> &'static HashMap<String, String> {
    static T: OnceLock<HashMap<String, String>> = OnceLock::new();
    T.get_or_init(|| std::fs::read_to_string(chemin()).map(|s| parse(&s)).unwrap_or_default())
}

/// Valeur de `[section] cle`, si le fichier la porte (insensible à la casse).
pub fn valeur(section: &str, cle: &str) -> Option<String> {
    table().get(&format!("{}.{}", section.to_lowercase(), cle.to_lowercase())).cloned()
}

/// Parse le sous-ensemble TOML décrit en tête de module. Pur et total :
/// toute ligne incomprise est IGNORÉE (un fichier de config ne doit jamais
/// empêcher Waly de démarrer).
pub fn parse(texte: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut section = String::new();
    for ligne in texte.trim_start_matches('\u{feff}').lines() {
        let l = ligne.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Some(nom) = l.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            section = nom.trim().to_lowercase();
            continue;
        }
        let Some((cle, brut)) = l.split_once('=') else { continue };
        let cle = cle.trim().to_lowercase();
        if cle.is_empty() || section.is_empty() {
            continue;
        }
        if let Some(v) = valeur_toml(brut.trim()) {
            out.insert(format!("{section}.{cle}"), v);
        }
    }
    out
}

/// Les sections filles de `prefixe` (`[mcp.demo]` → `demo` pour `"mcp"`),
/// triées, dédoublonnées — l'énumération des serveurs MCP déclarés.
pub fn sections_sous(prefixe: &str) -> Vec<String> {
    sections_sous_dans(table(), prefixe)
}

pub fn sections_sous_dans(t: &HashMap<String, String>, prefixe: &str) -> Vec<String> {
    let debut = format!("{}.", prefixe.to_lowercase());
    let mut noms: Vec<String> = t
        .keys()
        .filter_map(|k| k.strip_prefix(&debut))
        .filter_map(|reste| reste.rsplit_once('.').map(|(section, _)| section.to_string()))
        .collect();
    noms.sort();
    noms.dedup();
    noms
}

/// Une valeur scalaire : chaîne littérale `'...'` (AUCUN échappement — la
/// forme recommandée pour les chemins Windows), chaîne `"..."` (échappements
/// `\\ \" \n \t` ; un `\x` inconnu est gardé tel quel, par tolérance), sinon
/// nombre ou booléen (commentaire de fin de ligne coupé).
fn valeur_toml(brut: &str) -> Option<String> {
    if let Some(reste) = brut.strip_prefix('\'') {
        let fin = reste.find('\'')?;
        return Some(reste[..fin].to_string());
    }
    if let Some(reste) = brut.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = reste.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => return Some(out),
                '\\' => match chars.next()? {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    '\\' => out.push('\\'),
                    '"' => out.push('"'),
                    autre => {
                        out.push('\\');
                        out.push(autre);
                    }
                },
                c => out.push(c),
            }
        }
        return None; // chaîne non refermée : ligne ignorée
    }
    let sans_comm = brut.split('#').next().unwrap_or("").trim();
    if sans_comm.is_empty() {
        return None;
    }
    let scalaire = sans_comm.parse::<i64>().is_ok()
        || sans_comm.parse::<f64>().is_ok()
        || sans_comm == "true"
        || sans_comm == "false";
    scalaire.then(|| sans_comm.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sections_et_types() {
        let t = parse(
            "# commentaire\n[llm]\nport = 11434\nmodele = \"qwen3vl-it:4b\"\n\n[Utilisateur]\nnom = \"Michée\" # inline\nactif = true\n",
        );
        assert_eq!(t.get("llm.port").map(String::as_str), Some("11434"));
        assert_eq!(t.get("llm.modele").map(String::as_str), Some("qwen3vl-it:4b"));
        assert_eq!(t.get("utilisateur.nom").map(String::as_str), Some("Michée"));
        assert_eq!(t.get("utilisateur.actif").map(String::as_str), Some("true"));
    }

    #[test]
    fn chaine_quotee_protege_le_diese_et_les_accents() {
        let t = parse("[a]\nx = \"été # pas un commentaire\"\n");
        assert_eq!(t.get("a.x").map(String::as_str), Some("été # pas un commentaire"));
    }

    #[test]
    fn lignes_illisibles_ignorees_sans_erreur() {
        let t = parse("\u{feff}[llm]\nport 42626\n= vide\n[cassé\nmodele = \"ok\"\nnu = valeurnue\n");
        // `port 42626` (pas de =), `[cassé` (section non refermée : la
        // précédente reste active), `nu` (valeur non quotée non scalaire).
        assert_eq!(t.len(), 1);
        assert_eq!(t.get("llm.modele").map(String::as_str), Some("ok"));
    }

    #[test]
    fn cle_hors_section_ignoree() {
        assert!(parse("port = 42626\n").is_empty());
    }

    #[test]
    fn chemins_windows_echappes_ou_litteraux() {
        let t = parse(
            "[s]\na = \"C:\\\\waly\\\\data\\\\waly.db\"\nb = 'C:\\Users\\Jane Doe\\x.py'\nc = \"dit \\\"oui\\\"\"\nd = \"C:\\waly\"\ne = \"non refermee\n",
        );
        assert_eq!(t.get("s.a").map(String::as_str), Some("C:\\waly\\data\\waly.db"));
        assert_eq!(t.get("s.b").map(String::as_str), Some("C:\\Users\\Jane Doe\\x.py"));
        assert_eq!(t.get("s.c").map(String::as_str), Some("dit \"oui\""));
        // `\w` inconnu : gardé tel quel (tolérance).
        assert_eq!(t.get("s.d").map(String::as_str), Some("C:\\waly"));
        assert!(!t.contains_key("s.e"));
    }

    #[test]
    fn sections_filles_enumerees() {
        let t = parse(
            "[mcp.demo]\ncommande = 'python'\n[mcp.fichiers]\ncommande = 'npx.cmd'\narg1 = 'x'\n[llm]\nport = 1\n",
        );
        assert_eq!(sections_sous_dans(&t, "mcp"), vec!["demo".to_string(), "fichiers".to_string()]);
        assert!(sections_sous_dans(&t, "absent").is_empty());
    }
}
