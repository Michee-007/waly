//! Rattrapage des appels d'outils ÉCRITS EN TEXTE.
//!
//! Les petits modèles locaux (llama3.2-3b en secours Ollama, vécu
//! 2026-09-03) « imitent » parfois un appel d'outil : au lieu de l'émettre
//! par l'API, ils écrivent le JSON dans leur réponse — l'utilisateur voit
//! `{"name": "creer_fichier", "parameters": {...}}` et rien ne se passe.
//! On reconnaît ces réponses et on les rejoue comme de VRAIS appels : ils
//! passent par le MÊME dispatch (murs financiers, validation, gate de
//! risque, approbation humaine) — aucun raccourci de sûreté.
//!
//! Garde contre les faux positifs : la réponse ENTIÈRE doit être l'appel
//! (une fois retirées les clôtures ```json et les balises <tool_call>),
//! jamais un JSON cité au milieu d'une phrase.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::llm::ToolCall;

/// Reconnaît une réponse texte qui n'est QU'UN appel d'outil écrit. Formes :
/// - `{"name": "x", "parameters"|"arguments": {...}}` (et la variante
///   `{"function": {"name": ..., "arguments": ...}}`). Un nom INCONNU est
///   accepté s'il a une forme d'identifiant : le dispatch répondra « outil
///   inconnu » avec les voisins, et le modèle se corrigera (vécu :
///   `creer_fichier` au lieu de `ecrire_fichier`) ;
/// - `x {...}` (nom puis arguments) : nom CONNU seulement (forme plus
///   ambiguë).
/// Les arguments doivent être un objet JSON ; la lecture tolère les sauts de
/// ligne bruts dans les chaînes, fréquents chez les petits modèles.
pub fn appel_ecrit_en_texte(texte: &str, connu: impl Fn(&str) -> bool) -> Option<ToolCall> {
    let t = nettoyer(texte);
    if t.starts_with('{') && t.ends_with('}') {
        let v = parse_tolerant(t)?;
        let obj = v.as_object()?;
        let (nom, args) = match obj.get("function").and_then(|f| f.as_object()) {
            Some(f) => (f.get("name")?.as_str()?, f.get("arguments").or_else(|| f.get("parameters"))?),
            None => (obj.get("name")?.as_str()?, obj.get("parameters").or_else(|| obj.get("arguments"))?),
        };
        if !est_identifiant(nom) {
            return None;
        }
        return Some(appel(nom, objet_args(args)?));
    }
    // Forme « nom {args} ».
    let i = t.find('{')?;
    let nom = t[..i].trim();
    if !t.ends_with('}') || !est_identifiant(nom) || !connu(nom) {
        return None;
    }
    let args = parse_tolerant(&t[i..])?;
    if !args.is_object() {
        return None;
    }
    Some(appel(nom, args))
}

/// Retire les emballages courants : clôtures de code et balises d'appel.
fn nettoyer(texte: &str) -> &str {
    let mut t = texte.trim();
    // Deux passes : les emballages peuvent s'imbriquer dans les deux sens.
    for _ in 0..2 {
        if let Some(r) = t.strip_prefix("```json").or_else(|| t.strip_prefix("```")) {
            t = r.trim();
        }
        if let Some(r) = t.strip_suffix("```") {
            t = r.trim();
        }
        if let Some(r) = t.strip_prefix("<tool_call>") {
            t = r.trim();
        }
        if let Some(r) = t.strip_suffix("</tool_call>") {
            t = r.trim();
        }
    }
    t
}

fn parse_tolerant(s: &str) -> Option<serde_json::Value> {
    serde_json::from_str(s)
        .ok()
        .or_else(|| serde_json::from_str(&echapper_controles(s)).ok())
}

/// Échappe les caractères de contrôle BRUTS à l'intérieur des chaînes JSON
/// (un saut de ligne littéral dans `"contenu": "..."` rend le JSON invalide).
fn echapper_controles(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    let (mut dans_chaine, mut echappe) = (false, false);
    for c in s.chars() {
        if !dans_chaine {
            if c == '"' {
                dans_chaine = true;
            }
            out.push(c);
            continue;
        }
        if echappe {
            echappe = false;
            out.push(c);
            continue;
        }
        match c {
            '\\' => {
                echappe = true;
                out.push(c);
            }
            '"' => {
                dans_chaine = false;
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Arguments : un objet, ou une chaîne contenant un objet JSON.
fn objet_args(v: &serde_json::Value) -> Option<serde_json::Value> {
    match v {
        serde_json::Value::Object(_) => Some(v.clone()),
        serde_json::Value::String(s) => parse_tolerant(s).filter(|a| a.is_object()),
        _ => None,
    }
}

/// Forme d'un nom d'outil : snake_case ASCII minuscule, ≤ 64 caractères.
fn est_identifiant(nom: &str) -> bool {
    let mut cs = nom.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && nom.len() <= 64
        && cs.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn appel(nom: &str, args: serde_json::Value) -> ToolCall {
    // Identifiant unique par processus : le résultat d'outil s'y rattache.
    static N: AtomicUsize = AtomicUsize::new(0);
    ToolCall {
        id: format!("rattrape-{}", N.fetch_add(1, Ordering::Relaxed)),
        name: nom.to_string(),
        arguments: args.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connus(n: &str) -> bool {
        ["heure", "ecrire_fichier", "resoudre_attentes", "lire_fichier"].contains(&n)
    }

    fn args(c: &ToolCall) -> serde_json::Value {
        serde_json::from_str(&c.arguments).unwrap()
    }

    #[test]
    fn cas_reel_du_3_septembre_sauts_de_ligne_bruts_et_nom_invente() {
        // Réponse réelle de llama3.2 (capture de Michée) : sauts de ligne
        // littéraux dans la chaîne, nom halluciné.
        let t = "{\"name\": \"creer_fichier\", \"parameters\": {\"chemin\": \"rapport.md\", \
                 \"contenu\": \"<# Plan de la semaine #>\nVendredi | [L'item du vendredi]\nSamedi\"}}";
        let c = appel_ecrit_en_texte(t, connus).expect("doit etre rattrape");
        assert_eq!(c.name, "creer_fichier"); // le dispatch dira « inconnu » + voisins
        assert_eq!(args(&c)["chemin"], "rapport.md");
        assert!(args(&c)["contenu"].as_str().unwrap().contains("\nVendredi"));
    }

    #[test]
    fn emballages_cloture_et_balise() {
        let c = appel_ecrit_en_texte("```json\n{\"name\":\"heure\",\"arguments\":{}}\n```", connus);
        assert_eq!(c.unwrap().name, "heure");
        let c = appel_ecrit_en_texte("<tool_call>{\"name\":\"heure\",\"arguments\":{}}</tool_call>", connus);
        assert_eq!(c.unwrap().name, "heure");
    }

    #[test]
    fn variante_function_et_arguments_en_chaine() {
        let t = r#"{"function":{"name":"lire_fichier","arguments":"{\"chemin\":\"a.md\"}"}}"#;
        let c = appel_ecrit_en_texte(t, connus).unwrap();
        assert_eq!(c.name, "lire_fichier");
        assert_eq!(args(&c)["chemin"], "a.md");
    }

    #[test]
    fn forme_nom_puis_arguments_si_connu() {
        let c = appel_ecrit_en_texte(r#"resoudre_attentes {"confirmes":[1]}"#, connus).unwrap();
        assert_eq!(c.name, "resoudre_attentes");
        assert_eq!(args(&c)["confirmes"][0], 1);
        // Nom inconnu dans cette forme ambiguë : laissé en texte.
        assert!(appel_ecrit_en_texte(r#"bonjour {"a":1}"#, connus).is_none());
    }

    #[test]
    fn json_casse_reste_du_texte() {
        // Autre fuite réelle (capture de Michée) : arguments invalides.
        let t = r#"resoudre_attentes {"confirmes":[id"Ecrire un fichier"]}"#;
        assert!(appel_ecrit_en_texte(t, connus).is_none());
    }

    #[test]
    fn pas_de_faux_positif_sur_la_prose() {
        assert!(appel_ecrit_en_texte("Bonjour !", connus).is_none());
        assert!(appel_ecrit_en_texte(
            r#"Voici un exemple : {"name":"heure","parameters":{}}"#,
            connus
        )
        .is_none());
        // Un « nom » qui n'a pas une forme d'identifiant.
        assert!(appel_ecrit_en_texte(r#"{"name":"Mon Nom","parameters":{}}"#, connus).is_none());
        // Un objet JSON quelconque sans forme d'appel.
        assert!(appel_ecrit_en_texte(r#"{"ville":"Paris"}"#, connus).is_none());
    }

    #[test]
    fn identifiants_uniques() {
        let a = appel_ecrit_en_texte(r#"{"name":"heure","arguments":{}}"#, connus).unwrap();
        let b = appel_ecrit_en_texte(r#"{"name":"heure","arguments":{}}"#, connus).unwrap();
        assert_ne!(a.id, b.id);
    }
}
