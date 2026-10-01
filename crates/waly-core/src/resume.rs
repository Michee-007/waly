//! Résumé des longues conversations (lot 2 des « bientôt », 2026-09-30).
//!
//! La fenêtre de contexte ne garde que les derniers messages (rebuild) : sans
//! résumé, Waly OUBLIE le début d'une longue conversation. Au repos, les
//! messages sortis de la fenêtre sont condensés (un tour LLM court, sans
//! outils) en un résumé CUMULATIF stocké par session ; il entre au système
//! STABLE au rebuild suivant — jamais par tour (discipline append-only R4.5).

use rusqlite::Connection;

use crate::llm::{LlmClient, Msg, Turn};
use crate::store;

/// Messages gardés en clair par la fenêtre du desktop (`rebuild_window`).
pub const FENETRE: u32 = 8;
/// Nombre minimal de messages hors fenêtre avant de lancer un résumé.
pub const SEUIL: usize = 6;
/// Longueur max d'un message dans le transcript à résumer (caractères).
const MESSAGE_MAX: usize = 600;

const PROMPT_RESUME: &str = "Tu tiens la memoire d'une conversation entre un \
utilisateur et son assistant Waly. On te donne l'ancien resume (peut etre vide) \
et les messages suivants. Ecris le NOUVEAU resume cumulatif, en francais, \
150 mots maximum : faits appris sur l'utilisateur, decisions prises, travail \
fait, questions encore ouvertes. Style telegraphique, a la 3e personne \
(« l'utilisateur », « Waly »). Reponds UNIQUEMENT le resume.";

/// Transcript borné envoyé au résumé.
pub fn transcript(ancien: Option<&str>, messages: &[(i64, String, String)]) -> String {
    let mut t = format!("Ancien resume : {}\n\nMessages suivants :", ancien.unwrap_or("(aucun)"));
    for (_, role, contenu) in messages {
        let qui = if role == "user" { "Utilisateur" } else { "Waly" };
        // L'en-tête [horodatage | conscience] des tours dictés n'a rien à faire ici.
        let c = contenu.trim_start();
        let c = c.strip_prefix('[').and_then(|r| r.split_once("] ")).map(|(_, x)| x).unwrap_or(c);
        t.push_str(&format!("\n{qui} : {}", crate::prompt::compacte(c, MESSAGE_MAX)));
    }
    t
}

/// Résume ce qui est sorti de la fenêtre de `session`, s'il y a assez de
/// matière. `Ok(true)` si le résumé a été mis à jour.
pub fn resumer_session(llm: &LlmClient, conn: &Connection, session: i64) -> Result<bool, String> {
    let msgs = store::messages_a_resumer(conn, session, FENETRE, SEUIL).map_err(|e| e.to_string())?;
    let Some(&(dernier, _, _)) = msgs.last() else { return Ok(false) };
    let ancien = store::resume_session(conn, session).map(|(r, _)| r);
    let messages = [Msg::System(PROMPT_RESUME.into()), Msg::User(transcript(ancien.as_deref(), &msgs))];
    let texte = match llm.chat(&messages, &[])? {
        Turn::Text(t) => t,
        Turn::ToolCalls(_) => return Ok(false),
    };
    let texte = texte.trim();
    if texte.chars().count() < 20 {
        return Ok(false);
    }
    store::set_resume_session(conn, session, texte, dernier).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Bloc du système STABLE : le résumé du début de la conversation. Vide
/// sans résumé.
pub fn bloc_prompt(conn: &Connection, session: i64) -> String {
    match store::resume_session(conn, session) {
        Some((r, _)) if !r.is_empty() => format!(
            "\nDEBUT DE CETTE CONVERSATION (resume des messages plus anciens que ceux que tu vois) :\n{r}"
        ),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seuls_les_messages_hors_fenetre_sont_a_resumer() {
        let conn = store::open(":memory:").unwrap();
        let s = store::create_session(&conn, "Longue", "chat").unwrap();
        for i in 0..(FENETRE as usize + SEUIL - 1) {
            store::append_message_in(&conn, s, if i % 2 == 0 { "user" } else { "assistant" }, &format!("m{i}")).unwrap();
        }
        // SEUIL - 1 messages hors fenêtre : pas encore.
        assert!(store::messages_a_resumer(&conn, s, FENETRE, SEUIL).unwrap().is_empty());
        store::append_message_in(&conn, s, "assistant", "encore").unwrap();
        let a = store::messages_a_resumer(&conn, s, FENETRE, SEUIL).unwrap();
        assert_eq!(a.len(), SEUIL);
        assert_eq!(a[0].2, "m0");
        // Une fois résumés, ils ne reviennent plus.
        store::set_resume_session(&conn, s, "resume", a.last().unwrap().0).unwrap();
        assert!(store::messages_a_resumer(&conn, s, FENETRE, SEUIL).unwrap().is_empty());
        assert!(bloc_prompt(&conn, s).contains("resume"));
    }

    #[test]
    fn transcript_retire_l_en_tete_et_borne() {
        let t = transcript(None, &[(1, "user".into(), "[lundi 10:00 | en appel] bonjour".into())]);
        assert!(t.contains("Utilisateur : bonjour"));
        assert!(t.contains("(aucun)"));
    }
}
