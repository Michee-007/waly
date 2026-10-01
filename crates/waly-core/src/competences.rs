//! Compétences apprises — la boucle d'auto-amélioration (2026-09-10).
//!
//! Le constat du comparatif Hermes agent (Nous Research) : un agent qui
//! DISTILLE une capacité réutilisable après chaque tâche réussie devient
//! meilleur à l'usage. Version Waly, 100 % locale et huis clos :
//!
//! 1. Une MISSION passe à `fini` avec au moins un outil réellement exécuté →
//!    un tour LLM court distille `{titre, declencheur, recette}` (étapes,
//!    outils, pièges — jamais de données personnelles, le prompt l'exige).
//! 2. Stockée dans `competences` (store.rs), indexée dans sqlite-vec quand
//!    l'embedder est chargé (`try_index` — même mécanique que la mémoire).
//! 3. À la mission suivante : la compétence PERTINENTE (sémantique si
//!    embedder, mot-clé sinon — le desktop v1 vit sans embedder) est
//!    injectée dans l'EN-TÊTE FRAIS du message — discipline append-only
//!    R4.5 : jamais dans le système ni l'historique.
//!
//! Doublons : re-distiller la même compétence la RENFORCE (touch) au lieu
//! d'en créer une copie (clé = slug du titre, ou fort recouvrement de
//! déclencheur).

use rusqlite::Connection;

use crate::llm::{LlmClient, Msg, Turn};
use crate::native_tools::SharedEmbedder;
use crate::store;

/// Prompt de distillation : un JSON strict, court, impersonnel.
const PROMPT_DISTILLATION: &str = "Tu viens d'achever une mission avec succes. \
Distille UNE competence reutilisable pour des missions semblables. Reponds \
UNIQUEMENT ce JSON, rien d'autre : {\"titre\": \"nom court de la competence\", \
\"declencheur\": \"les demandes types qui l'appellent, en une phrase\", \
\"recette\": \"etapes numerotees : quoi faire, quels outils appeler, quels pieges eviter\"}. \
Concret et bref (recette 90 mots max). Ne cite AUCUNE donnee personnelle \
(noms, chemins prives, contenus) : la competence decrit la METHODE, pas le cas.";

#[derive(Debug, Clone, PartialEq)]
pub struct Competence {
    pub key: String,
    pub titre: String,
    pub declencheur: String,
    pub recette: String,
}

/// Slug d'un titre → clé stable (minuscules, alphanumérique, tirets).
pub fn slug(titre: &str) -> String {
    let mut s = String::new();
    for c in titre.to_lowercase().chars() {
        if c.is_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    s.trim_matches('-').chars().take(60).collect()
}

/// Extrait la compétence du texte du modèle : premier bloc `{...}` JSON
/// valide portant les trois champs (tolère prose et clôtures ``` autour).
pub fn extraire(texte: &str) -> Option<Competence> {
    let debut = texte.find('{')?;
    let fin = texte.rfind('}')?;
    if fin <= debut {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&texte[debut..=fin]).ok()?;
    let champ = |n: &str| {
        v.get(n)
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let titre = crate::prompt::compacte(&champ("titre")?, 80);
    let declencheur = crate::prompt::compacte(&champ("declencheur")?, 240);
    let recette = crate::prompt::compacte(&champ("recette")?, 700);
    let key = slug(&titre);
    if key.len() < 3 {
        return None;
    }
    Some(Competence { key, titre, declencheur, recette })
}

/// Mots porteurs d'un texte (≥ 4 caractères, minuscules, dédoublonnés) —
/// l'appariement mot-clé du chemin SANS embedder.
pub fn mots_cles(texte: &str) -> Vec<String> {
    let mut vus = Vec::new();
    for mot in texte.to_lowercase().split(|c: char| !c.is_alphanumeric()) {
        if mot.chars().count() >= 4 && !vus.iter().any(|v| v == mot) {
            vus.push(mot.to_string());
        }
    }
    vus
}

/// Nombre de mots porteurs de `source` présents dans `message`.
fn recouvrement(source: &str, message: &str) -> usize {
    let msg = message.to_lowercase();
    mots_cles(source).iter().filter(|m| msg.contains(m.as_str())).count()
}

/// La compétence la plus pertinente pour une demande de mission, ou rien.
/// Sémantique (plancher [`store::SEMANTIC_MIN_COS`]) si l'embedder est
/// chargé ; sinon mot-clé : ≥ 2 mots porteurs du déclencheur+titre présents
/// dans la demande. Marque l'usage (touch) — la vie de la compétence.
pub fn pertinente(
    conn: &Connection,
    emb: &SharedEmbedder,
    message: &str,
) -> Option<store::CompetenceRow> {
    let choisie = pertinente_semantique(conn, emb, message)
        .or_else(|| pertinente_mots(conn, message))?;
    store::touch_competence(conn, &choisie.key).ok();
    Some(choisie)
}

fn pertinente_semantique(
    conn: &Connection,
    emb: &SharedEmbedder,
    message: &str,
) -> Option<store::CompetenceRow> {
    let emb = emb.as_ref()?;
    let vec = emb.borrow_mut().embed_query(message).ok()?;
    let json = crate::embed::to_vec_json(&vec);
    let voisins = store::semantic_neighbors(conn, "competence", &json, 3).ok()?;
    let (key, _) = voisins.into_iter().find(|(_, cos)| *cos >= store::SEMANTIC_MIN_COS)?;
    store::get_competence(conn, &key).ok().flatten()
}

fn pertinente_mots(conn: &Connection, message: &str) -> Option<store::CompetenceRow> {
    let toutes = store::list_competences(conn).ok()?;
    toutes
        .into_iter()
        .map(|c| (recouvrement(&format!("{} {}", c.titre, c.declencheur), message), c))
        .filter(|(hits, _)| *hits >= 2)
        .max_by_key(|(hits, _)| *hits)
        .map(|(_, c)| c)
}

/// Fragment d'en-tête frais (append-only R4.5 : le frais va en queue du
/// message, jamais dans le système).
pub fn fragment_frais(c: &store::CompetenceRow) -> String {
    format!(
        " | COMPETENCE APPRISE ({}) : {} — applique-la si elle convient",
        c.titre, c.recette
    )
}

/// Intègre une compétence distillée : renforce l'existante (même clé ou
/// déclencheur très recouvrant) au lieu de dupliquer, sinon crée + indexe.
/// Retourne le titre si une compétence NEUVE est née.
pub fn integrer(
    conn: &Connection,
    emb: &SharedEmbedder,
    c: &Competence,
) -> rusqlite::Result<Option<String>> {
    if store::get_competence(conn, &c.key)?.is_some() {
        store::touch_competence(conn, &c.key)?;
        return Ok(None);
    }
    for existante in store::list_competences(conn)? {
        if recouvrement(&existante.declencheur, &c.declencheur) >= 3 {
            store::touch_competence(conn, &existante.key)?;
            return Ok(None);
        }
    }
    store::upsert_competence(conn, &c.key, &c.titre, &c.declencheur, &c.recette)?;
    if let Some(e) = emb.as_ref() {
        if let Ok(vec) = e.borrow_mut().embed_passage(&format!("{} {}", c.titre, c.declencheur)) {
            store::index_entity(conn, "competence", &c.key, &crate::embed::to_vec_json(&vec))?;
        }
    }
    Ok(Some(c.titre.clone()))
}

/// Le résumé de mission envoyé à la distillation (borné : chaque token du
/// tour de distillation se paie aussi).
pub fn transcript(demande: &str, outils: &[String], issue: &str) -> String {
    format!(
        "Mission accomplie.\nDemande : {}\nOutils executes : {}\nIssue : {}",
        crate::prompt::compacte(demande, 400),
        outils.join(", "),
        crate::prompt::compacte(issue, 900),
    )
}

/// La boucle complète après une mission réussie : distille (un tour LLM
/// court, sans outils) puis intègre. `Ok(None)` = rien de neuf (réponse
/// illisible, ou compétence déjà connue — renforcée).
pub fn apprendre(
    llm: &LlmClient,
    conn: &Connection,
    emb: &SharedEmbedder,
    demande: &str,
    outils: &[String],
    issue: &str,
) -> Result<Option<String>, String> {
    distiller(llm, conn, emb, PROMPT_DISTILLATION, transcript(demande, outils, issue))
}

/// Un tour LLM court sans outils → JSON de compétence → intégration.
fn distiller(
    llm: &LlmClient,
    conn: &Connection,
    emb: &SharedEmbedder,
    prompt: &str,
    transcript: String,
) -> Result<Option<String>, String> {
    let messages = [Msg::System(prompt.into()), Msg::User(transcript)];
    let texte = match llm.chat(&messages, &[])? {
        Turn::Text(t) => t,
        Turn::ToolCalls(_) => return Ok(None),
    };
    let Some(c) = extraire(&texte) else { return Ok(None) };
    integrer(conn, emb, &c).map_err(|e| format!("competences: {e}"))
}

// ── S'améliorer à l'usage (lot 2, 2026-09-30) ──────────────────────────────
// Une mission RÉUSSIE qui a suivi une compétence la relit : si l'exécution
// réelle apporte une étape manquante ou un piège, la recette est réécrite et
// l'ancienne ARCHIVÉE (store::reviser_competence) — jamais de perte, retour
// possible depuis Personnaliser.

const PROMPT_AMELIORATION: &str = "Tu as suivi une COMPETENCE pour reussir une \
mission. Compare sa recette a ce qui s'est REELLEMENT passe. Si l'execution \
revele une etape manquante, un meilleur ordre ou un piege, reecris la recette \
(etapes numerotees, 90 mots max, aucune donnee personnelle). Sinon, garde-la. \
Reponds UNIQUEMENT ce JSON : {\"change\": true|false, \"recette\": \"...\", \
\"raison\": \"ce qui change, en quelques mots\"}.";

/// Relit une compétence après une mission réussie qui l'a utilisée.
/// `Ok(Some(raison))` si la recette a été réécrite (nouvelle version).
pub fn ameliorer(
    llm: &LlmClient,
    conn: &Connection,
    key: &str,
    demande: &str,
    outils: &[String],
    issue: &str,
) -> Result<Option<String>, String> {
    let Some(c) = store::get_competence(conn, key).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let transcript = format!(
        "Competence « {} » — recette actuelle :\n{}\n\n{}",
        c.titre,
        c.recette,
        transcript(demande, outils, issue)
    );
    let messages = [Msg::System(PROMPT_AMELIORATION.into()), Msg::User(transcript)];
    let texte = match llm.chat(&messages, &[])? {
        Turn::Text(t) => t,
        Turn::ToolCalls(_) => return Ok(None),
    };
    Ok(revision(&c.recette, &texte).and_then(|(recette, raison)| {
        store::reviser_competence(conn, key, &recette, &raison).ok().filter(|&ok| ok).map(|_| raison)
    }))
}

/// Décision pure : `Some((recette, raison))` seulement si le modèle dit
/// `change` ET que la recette diffère vraiment (espaces/casse ignorés) et
/// reste plausible (≥ 20 caractères).
pub fn revision(actuelle: &str, reponse: &str) -> Option<(String, String)> {
    let debut = reponse.find('{')?;
    let fin = reponse.rfind('}')?;
    let v: serde_json::Value = serde_json::from_str(&reponse[debut..=fin]).ok()?;
    if v["change"].as_bool() != Some(true) {
        return None;
    }
    let recette = v["recette"].as_str()?.trim().to_string();
    let norme = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    if recette.chars().count() < 20 || norme(&recette) == norme(actuelle) {
        return None;
    }
    let raison = v["raison"].as_str().unwrap_or("améliorée après usage").trim().to_string();
    Some((recette, if raison.is_empty() { "améliorée après usage".into() } else { raison }))
}

// ── Apprendre en regardant (B3, 2026-09-11) ─────────────────────────────────
// L'utilisateur MONTRE une tâche (session « regarde-moi » qu'il ouvre et ferme
// lui-même, étapes en texte, jamais de pixels — waly_sight::demo). La recette
// distillée doit être rejouable par les mains d'écran, sous approbation.

const PROMPT_DEMONSTRATION: &str = "L'utilisateur vient de te MONTRER une tache sur \
son ecran ; voici les etapes observees (texte seulement). Distille UNE competence \
reutilisable. Reponds UNIQUEMENT ce JSON, rien d'autre : {\"titre\": \"nom court de la \
tache\", \"declencheur\": \"les demandes types qui l'appellent, en une phrase\", \
\"recette\": \"etapes numerotees a rejouer : lire_ecran, puis pour chaque geste \
agir_ecran sur l'element vise (role et nom), dans l'ordre\"}. Generalise ce qui est \
propre au cas (noms de fichiers, textes saisis) en <a demander a l'utilisateur>. \
Recette 90 mots max. Aucune donnee personnelle.";

/// Les étapes observées, bornées (chaque token de la distillation se paie).
pub fn transcript_demonstration(etapes: &[String]) -> String {
    let mut s = String::from("Demonstration de l'utilisateur :\n");
    for (i, e) in etapes.iter().take(60).enumerate() {
        s.push_str(&format!("{}. {}\n", i + 1, crate::prompt::compacte(e, 160)));
    }
    if etapes.len() > 60 {
        s.push_str(&format!("… (+{} etapes)\n", etapes.len() - 60));
    }
    s
}

/// Distille une démonstration en compétence (`Ok(None)` : rien de neuf).
pub fn apprendre_demonstration(
    llm: &LlmClient,
    conn: &Connection,
    emb: &SharedEmbedder,
    etapes: &[String],
) -> Result<Option<String>, String> {
    if etapes.is_empty() {
        return Ok(None);
    }
    distiller(llm, conn, emb, PROMPT_DEMONSTRATION, transcript_demonstration(etapes))
}

#[cfg(test)]
mod tests_demo {
    use super::*;

    #[test]
    fn transcript_de_demonstration_numerote_et_borne() {
        let etapes: Vec<String> = (1..=70).map(|i| format!("clic bouton « B{i} »")).collect();
        let t = transcript_demonstration(&etapes);
        assert!(t.contains("1. clic bouton « B1 »"), "{t}");
        assert!(t.contains("60. clic bouton « B60 »"), "{t}");
        assert!(!t.contains("B61 »"), "{t}");
        assert!(t.contains("(+10 etapes)"), "{t}");
    }
}

#[cfg(test)]
mod tests_amelioration {
    use super::*;

    #[test]
    fn revision_seulement_si_change_vraiment() {
        let a = "1. ouvrir le fichier 2. ecrire le resume";
        let r = r#"ok {"change": true, "recette": "1. lister les taches 2. ouvrir le fichier 3. ecrire le resume", "raison": "etape de listage"} fin"#;
        assert_eq!(
            revision(a, r),
            Some(("1. lister les taches 2. ouvrir le fichier 3. ecrire le resume".into(), "etape de listage".into()))
        );
        // Le modèle dit « change » mais recopie la même recette : rien.
        let r = r#"{"change": true, "recette": "1. Ouvrir le fichier   2. ecrire le resume", "raison": "x"}"#;
        assert_eq!(revision(a, r), None);
        assert_eq!(revision(a, r#"{"change": false, "recette": "autre chose de long ici", "raison": ""}"#), None);
        assert_eq!(revision(a, r#"{"change": true, "recette": "trop court", "raison": ""}"#), None);
        assert_eq!(revision(a, "pas de json"), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Connection {
        store::open(":memory:").unwrap()
    }

    #[test]
    fn slug_stable_et_borne() {
        assert_eq!(slug("Créer un rapport hebdo !"), "créer-un-rapport-hebdo");
        assert_eq!(slug("   "), "");
    }

    #[test]
    fn extraire_json_pur_ou_enrobe() {
        let brut = r#"{"titre":"Rapport hebdo","declencheur":"créer un rapport ou un plan de semaine","recette":"1. ecrire_fichier…"}"#;
        let c = extraire(brut).unwrap();
        assert_eq!(c.key, "rapport-hebdo");
        let enrobe = format!("Voici la compétence :\n```json\n{brut}\n```\nVoilà !");
        assert_eq!(extraire(&enrobe).unwrap(), c);
    }

    #[test]
    fn extraire_refuse_incomplet_ou_illisible() {
        assert!(extraire("pas de json ici").is_none());
        assert!(extraire(r#"{"titre":"X"}"#).is_none());
        assert!(extraire(r#"{"titre":"","declencheur":"d","recette":"r"}"#).is_none());
    }

    #[test]
    fn pertinente_mots_cles_choisit_la_meilleure_et_compte_l_usage() {
        let conn = base();
        store::upsert_competence(
            &conn,
            "rapport-hebdo",
            "Rapport hebdo",
            "créer un rapport ou un plan de la semaine dans un fichier",
            "1. ecrire_fichier rapport.md",
        )
        .unwrap();
        store::upsert_competence(
            &conn,
            "tri-notes",
            "Tri des notes",
            "ranger et organiser les notes par thème",
            "1. chercher_notes 2. creer_note",
        )
        .unwrap();
        let c =
            pertinente(&conn, &None, "fais-moi un rapport avec le plan de la semaine").unwrap();
        assert_eq!(c.key, "rapport-hebdo");
        assert_eq!(store::get_competence(&conn, "rapport-hebdo").unwrap().unwrap().uses, 1);
        // Un seul mot commun (« notes » absent) : pas de faux positif.
        assert!(pertinente(&conn, &None, "quelle heure est-il ce matin ?").is_none());
    }

    #[test]
    fn integrer_renforce_au_lieu_de_dupliquer() {
        let conn = base();
        let c = Competence {
            key: "rapport-hebdo".into(),
            titre: "Rapport hebdo".into(),
            declencheur: "créer un rapport ou un plan de la semaine".into(),
            recette: "1. ecrire_fichier".into(),
        };
        assert_eq!(integrer(&conn, &None, &c).unwrap(), Some("Rapport hebdo".into()));
        // Même clé → renforcée, pas dupliquée.
        assert_eq!(integrer(&conn, &None, &c).unwrap(), None);
        // Titre différent mais déclencheur très recouvrant → renforce aussi.
        let c2 = Competence {
            key: "plan-semaine".into(),
            titre: "Plan semaine".into(),
            declencheur: "créer un plan de la semaine dans un rapport".into(),
            recette: "1. ecrire_fichier".into(),
        };
        assert_eq!(integrer(&conn, &None, &c2).unwrap(), None);
        assert_eq!(store::list_competences(&conn).unwrap().len(), 1);
        assert_eq!(store::get_competence(&conn, "rapport-hebdo").unwrap().unwrap().uses, 2);
    }

    #[test]
    fn fragment_frais_porte_titre_et_recette() {
        let c = store::CompetenceRow {
            key: "k".into(),
            titre: "Rapport hebdo".into(),
            declencheur: "d".into(),
            recette: "1. faire".into(),
            uses: 0,
        };
        let f = fragment_frais(&c);
        assert!(f.starts_with(" | COMPETENCE APPRISE"));
        assert!(f.contains("Rapport hebdo") && f.contains("1. faire"));
    }
}
