//! Passerelles de messagerie (lot 3 des « bientôt », 2026-10-01) : parler à
//! Waly depuis son téléphone. Première passerelle : Telegram (API des bots).
//!
//! Garde-fous, imposés par le code :
//! - **Waly reste scellé** : relève et envoi passent par la passerelle séparée
//!   (`exterieur::requete`), vers la seule adresse du service ; chaque message
//!   entrant ou sortant est inscrit au journal du sceau par l'appelant.
//! - **Un seul interlocuteur, appairé par code** : la passerelle ne répond
//!   qu'à la conversation PRIVÉE qui a envoyé le code affiché dans l'app.
//!   Tout le reste est ignoré sans réponse ; après [`ESSAIS_MAX`] codes faux,
//!   l'appairage se ferme (nouveau code à générer dans l'app).
//! - **Le jeton du bot** est chiffré par Windows (comme les clés des modèles
//!   extérieurs) et ne voyage que par l'entrée standard de la passerelle.
//! - Les réponses viennent du modèle LOCAL ; les actions qui demandent un
//!   accord restent en attente dans l'app (jamais approuvées à distance).
//!
//! Honnêteté : un message Telegram transite par les serveurs de Telegram —
//! c'est une sortie que l'utilisateur ouvre, visible dans Vie privée.

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::exterieur;

pub const TELEGRAM: &str = "https://api.telegram.org";
/// Codes faux tolérés avant de fermer l'appairage.
pub const ESSAIS_MAX: i64 = 5;
/// Longueur maximale d'un message Telegram (4096) avec de la marge.
const MESSAGE_MAX: usize = 3900;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Passerelle {
    pub id: i64,
    pub genre: String,
    pub base_url: String,
    /// Nom public du bot (rendu par le service à la connexion).
    pub bot: String,
    /// Code d'appairage à envoyer depuis le téléphone (vide une fois appairé).
    pub code: String,
    /// Conversation appairée (identifiant du service) et prénom affiché.
    pub interlocuteur: Option<i64>,
    pub nom: Option<String>,
    pub essais: i64,
    #[serde(skip)]
    pub curseur: i64,
}

fn table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS passerelles (
           id            INTEGER PRIMARY KEY,
           genre         TEXT NOT NULL,
           base_url      TEXT NOT NULL,
           jeton         BLOB NOT NULL,
           bot           TEXT NOT NULL DEFAULT '',
           code          TEXT NOT NULL DEFAULT '',
           interlocuteur INTEGER,
           nom           TEXT,
           essais        INTEGER NOT NULL DEFAULT 0,
           curseur       INTEGER NOT NULL DEFAULT 0,
           created_at    TEXT NOT NULL DEFAULT (datetime('now','localtime'))
         );",
    )
}

pub fn lister(conn: &Connection) -> Vec<Passerelle> {
    if table(conn).is_err() {
        return Vec::new();
    }
    let Ok(mut st) = conn.prepare(
        "SELECT id, genre, base_url, bot, code, interlocuteur, nom, essais, curseur FROM passerelles ORDER BY id",
    ) else {
        return Vec::new();
    };
    st.query_map([], |r| {
        Ok(Passerelle {
            id: r.get(0)?,
            genre: r.get(1)?,
            base_url: r.get(2)?,
            bot: r.get(3)?,
            code: r.get(4)?,
            interlocuteur: r.get(5)?,
            nom: r.get(6)?,
            essais: r.get(7)?,
            curseur: r.get(8)?,
        })
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

/// Jeton d'un bot Telegram : `<chiffres>:<lettres, chiffres, _ ->`.
pub fn jeton_admis(jeton: &str) -> bool {
    match jeton.split_once(':') {
        Some((a, b)) => {
            (3..=20).contains(&a.len())
                && a.chars().all(|c| c.is_ascii_digit())
                && (10..=100).contains(&b.len())
                && b.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        }
        None => false,
    }
}

/// Code d'appairage à 8 chiffres. Aléa du système (clés de hachage tirées par
/// l'OS à chaque `RandomState`), sans dépendance.
pub fn nouveau_code() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    format!("{:08}", h.finish() % 100_000_000)
}

fn jeton(conn: &Connection, id: i64) -> Result<String, String> {
    let chiffre: Vec<u8> = conn
        .query_row("SELECT jeton FROM passerelles WHERE id=?1", params![id], |r| r.get(0))
        .map_err(|_| "passerelle introuvable".to_string())?;
    String::from_utf8(exterieur::deproteger(&chiffre)?).map_err(|_| "jeton illisible".to_string())
}

fn appel(base: &str, jeton: &str, methode: &str, corps: Option<&serde_json::Value>, max_s: u32) -> Result<serde_json::Value, String> {
    let url = format!("{base}/bot{jeton}/{methode}");
    let corps = corps.map(|c| c.to_string());
    let methode = if corps.is_some() { "POST" } else { "GET" };
    let (code, brut) = exterieur::requete(methode, &url, &["Content-Type: application/json".to_string()], corps.as_deref(), max_s)?;
    let v: serde_json::Value = serde_json::from_str(&brut).map_err(|_| format!("réponse illisible du service ({code})"))?;
    if v["ok"].as_bool() != Some(true) {
        let d = v["description"].as_str().unwrap_or("refus");
        return Err(match v["error_code"].as_i64() {
            Some(401) | Some(404) => format!("jeton refusé par le service ({d})"),
            _ => format!("le service a répondu : {d}"),
        });
    }
    Ok(v["result"].clone())
}

/// Connecte un bot Telegram : vérifie le jeton auprès du service (une vraie
/// sortie), le chiffre, et prépare un code d'appairage. `base_url` vide =
/// Telegram ; autre adresse = banc local.
pub fn connecter(conn: &Connection, base_url: &str, jeton: &str) -> Result<Passerelle, String> {
    let base = if base_url.trim().is_empty() { TELEGRAM } else { base_url.trim().trim_end_matches('/') };
    exterieur::url_admise(base)?;
    let jeton = jeton.trim();
    if !jeton_admis(jeton) {
        return Err("jeton invalide : attendu le jeton donné par BotFather (123456:ABC…)".into());
    }
    let moi = appel(base, jeton, "getMe", None, 20)?;
    let bot = moi["username"].as_str().unwrap_or("bot").to_string();
    let chiffre = exterieur::proteger(jeton.as_bytes())?;
    table(conn).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO passerelles(genre, base_url, jeton, bot, code) VALUES ('telegram', ?1, ?2, ?3, ?4)",
        params![base, chiffre, bot, nouveau_code()],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();
    lister(conn).into_iter().find(|p| p.id == id).ok_or_else(|| "passerelle introuvable".into())
}

/// Retire une passerelle et son jeton.
pub fn retirer(conn: &Connection, id: i64) -> Result<bool, String> {
    table(conn).map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM passerelles WHERE id=?1", params![id]).map(|n| n > 0).map_err(|e| e.to_string())
}

/// Oublie l'interlocuteur et rouvre l'appairage avec un NOUVEAU code.
pub fn reappairer(conn: &Connection, id: i64) -> Result<String, String> {
    let code = nouveau_code();
    conn.execute(
        "UPDATE passerelles SET interlocuteur=NULL, nom=NULL, essais=0, code=?2 WHERE id=?1",
        params![id, code],
    )
    .map_err(|e| e.to_string())?;
    Ok(code)
}

/// Un message reçu du service.
#[derive(Debug, Clone, PartialEq)]
pub struct Recu {
    pub update_id: i64,
    pub chat_id: i64,
    pub prive: bool,
    pub prenom: String,
    pub texte: String,
}

/// Messages texte d'une réponse `getUpdates` (le reste — photos, groupes
/// sans texte, modifications — est ignoré mais fait avancer le curseur).
pub fn recus_telegram(resultat: &serde_json::Value) -> (Vec<Recu>, Option<i64>) {
    let mut dernier = None;
    let mut out = Vec::new();
    for u in resultat.as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let Some(id) = u["update_id"].as_i64() else { continue };
        dernier = Some(dernier.map_or(id, |d: i64| d.max(id)));
        let m = &u["message"];
        let (Some(chat), Some(texte)) = (m["chat"]["id"].as_i64(), m["text"].as_str()) else { continue };
        out.push(Recu {
            update_id: id,
            chat_id: chat,
            prive: m["chat"]["type"] == "private",
            prenom: m["from"]["first_name"].as_str().unwrap_or("").chars().take(40).collect(),
            texte: texte.to_string(),
        });
    }
    (out, dernier)
}

#[derive(Debug, PartialEq)]
pub enum Decision {
    /// Bon code depuis une conversation privée : l'appairer.
    Appairer,
    /// Message de l'interlocuteur appairé : Waly répond.
    Repondre,
    /// Code faux pendant l'appairage : compter l'essai, ne rien répondre.
    CodeFaux,
    /// Tout le reste : ignoré sans réponse (la raison va au journal).
    Ignorer(&'static str),
}

/// Décision PURE pour un message reçu.
pub fn decider(p: &Passerelle, r: &Recu) -> Decision {
    if !r.prive {
        return Decision::Ignorer("pas une conversation privée");
    }
    match p.interlocuteur {
        Some(i) if i == r.chat_id => Decision::Repondre,
        Some(_) => Decision::Ignorer("inconnu (un interlocuteur est déjà appairé)"),
        None if p.essais >= ESSAIS_MAX || p.code.is_empty() => Decision::Ignorer("appairage fermé"),
        None => {
            // `/start 12345678` (lien profond) ou le code seul.
            let t = r.texte.trim();
            let t = t.strip_prefix("/start").map(str::trim).unwrap_or(t);
            if t == p.code { Decision::Appairer } else { Decision::CodeFaux }
        }
    }
}

/// Découpe une réponse en messages que le service accepte, aux fins de ligne
/// si possible.
pub fn decouper(texte: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut reste: Vec<char> = texte.trim().chars().collect();
    while reste.len() > MESSAGE_MAX {
        let coupe = reste[..MESSAGE_MAX].iter().rposition(|c| *c == '\n').filter(|i| *i > MESSAGE_MAX / 2).unwrap_or(MESSAGE_MAX);
        out.push(reste[..coupe].iter().collect::<String>().trim().to_string());
        reste = reste[coupe..].to_vec();
    }
    let fin: String = reste.iter().collect::<String>().trim().to_string();
    if !fin.is_empty() {
        out.push(fin);
    }
    out
}

/// Relève les nouveaux messages (attente longue de `attente_s` côté service)
/// et avance le curseur — un message n'est jamais traité deux fois.
pub fn relever(conn: &Connection, p: &Passerelle, attente_s: u32) -> Result<Vec<Recu>, String> {
    let jeton = jeton(conn, p.id)?;
    let corps = serde_json::json!({"offset": p.curseur, "timeout": attente_s, "allowed_updates": ["message"]});
    let res = appel(&p.base_url, &jeton, "getUpdates", Some(&corps), attente_s + 15)?;
    let (recus, dernier) = recus_telegram(&res);
    if let Some(d) = dernier {
        conn.execute("UPDATE passerelles SET curseur=?2 WHERE id=?1", params![p.id, d + 1]).map_err(|e| e.to_string())?;
    }
    Ok(recus)
}

/// Envoie un texte à une conversation (découpé si nécessaire).
pub fn envoyer(conn: &Connection, p: &Passerelle, chat_id: i64, texte: &str) -> Result<(), String> {
    let jeton = jeton(conn, p.id)?;
    for morceau in decouper(texte) {
        appel(&p.base_url, &jeton, "sendMessage", Some(&serde_json::json!({"chat_id": chat_id, "text": morceau})), 30)?;
    }
    Ok(())
}

/// Enregistre l'appairage (le code est consommé).
pub fn appairer(conn: &Connection, id: i64, chat_id: i64, prenom: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE passerelles SET interlocuteur=?2, nom=?3, code='', essais=0 WHERE id=?1 AND interlocuteur IS NULL",
        params![id, chat_id, prenom],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Compte un code faux ; rend le nombre d'essais.
pub fn code_faux(conn: &Connection, id: i64) -> i64 {
    let _ = conn.execute("UPDATE passerelles SET essais=essais+1 WHERE id=?1", params![id]);
    conn.query_row("SELECT essais FROM passerelles WHERE id=?1", params![id], |r| r.get(0)).unwrap_or(ESSAIS_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(interlocuteur: Option<i64>, essais: i64) -> Passerelle {
        Passerelle {
            id: 1,
            genre: "telegram".into(),
            base_url: TELEGRAM.into(),
            bot: "waly_bot".into(),
            code: "12345678".into(),
            interlocuteur,
            nom: None,
            essais,
            curseur: 0,
        }
    }

    fn recu(chat: i64, prive: bool, texte: &str) -> Recu {
        Recu { update_id: 1, chat_id: chat, prive, prenom: "A".into(), texte: texte.into() }
    }

    #[test]
    fn appairage_par_code_puis_un_seul_interlocuteur() {
        assert_eq!(decider(&p(None, 0), &recu(42, true, "12345678")), Decision::Appairer);
        assert_eq!(decider(&p(None, 0), &recu(42, true, "/start 12345678")), Decision::Appairer);
        assert_eq!(decider(&p(None, 0), &recu(42, true, "bonjour")), Decision::CodeFaux);
        // Un groupe ne s'appaire jamais, même avec le bon code.
        assert!(matches!(decider(&p(None, 0), &recu(-100, false, "12345678")), Decision::Ignorer(_)));
        // Trop d'essais : l'appairage est fermé, même pour le bon code.
        assert!(matches!(decider(&p(None, ESSAIS_MAX), &recu(42, true, "12345678")), Decision::Ignorer(_)));
        // Appairé : seul l'interlocuteur est servi.
        assert_eq!(decider(&p(Some(42), 0), &recu(42, true, "quelle heure ?")), Decision::Repondre);
        assert!(matches!(decider(&p(Some(42), 0), &recu(99, true, "12345678")), Decision::Ignorer(_)));
    }

    #[test]
    fn lecture_des_messages_et_du_curseur() {
        let v: serde_json::Value = serde_json::from_str(
            r#"[{"update_id":10,"message":{"message_id":1,"from":{"id":42,"is_bot":false,"first_name":"Michée"},"chat":{"id":42,"type":"private"},"date":1,"text":"Bonjour"}},
                {"update_id":11,"message":{"message_id":2,"from":{"id":7,"first_name":"X"},"chat":{"id":-5,"type":"group"},"date":1,"text":"salut"}},
                {"update_id":12,"message":{"message_id":3,"chat":{"id":42,"type":"private"},"photo":[]}}]"#,
        )
        .unwrap();
        let (r, dernier) = recus_telegram(&v);
        assert_eq!(dernier, Some(12), "une photo fait avancer le curseur sans être traitée");
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].chat_id, r[0].prive, r[0].prenom.as_str(), r[0].texte.as_str()), (42, true, "Michée", "Bonjour"));
        assert!(!r[1].prive);
        assert_eq!(recus_telegram(&serde_json::json!([])), (vec![], None));
    }

    #[test]
    fn jetons_codes_et_decoupe() {
        assert!(jeton_admis("123456789:AAH-abc_DEF1234567890"));
        for j in ["", "abc", "123:court", "12a:AAAAAAAAAAAAAAA", "123456:AAAA AAAAAAAAAAA", "123456:AAAAAAAAAAAA\"x"] {
            assert!(!jeton_admis(j), "{j}");
        }
        let (a, b) = (nouveau_code(), nouveau_code());
        assert!(a.len() == 8 && a.chars().all(|c| c.is_ascii_digit()));
        assert_ne!(a, b, "deux codes de suite diffèrent");
        assert_eq!(decouper("  court  "), vec!["court".to_string()]);
        let long = format!("{}\n{}", "a".repeat(3000), "b".repeat(3000));
        let m = decouper(&long);
        assert_eq!(m.len(), 2);
        assert!(m[0].chars().all(|c| c == 'a') && m[1].chars().all(|c| c == 'b'));
        assert!(decouper("").is_empty());
    }
}
