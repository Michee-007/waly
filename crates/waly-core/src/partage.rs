//! Partage entre deux Waly (lot 3, 2026-10-01) : envoyer une conversation à
//! un contact, n'importe où dans le monde, sans compte.
//!
//! Décision Michée (01/10) : pas de « réseau local seulement » — les deux
//! machines ne se voient pas. Elles passent donc par un **relais** (programme
//! `waly-relais`, auto-hébergeable) qui ne fait que garder des enveloppes
//! chiffrées dans une boîte, le temps que le destinataire les relève. Les deux
//! Waly ne font que des connexions SORTANTES vers le relais : aucun port
//! entrant, aucune règle de pare-feu.
//!
//! - **Identité** : une paire de clés X25519 par installation, créée sur la
//!   machine. Le « code Waly » qu'on donne à un contact EST la clé publique
//!   (+ la boîte et l'adresse du relais) : pas de compte, pas d'annuaire.
//! - **Chiffrement** : `crypto_box` de NaCl (X25519 + XSalsa20-Poly1305),
//!   authentifié dans les deux sens — construction standard, rien d'inventé.
//!   Le relais ne voit que du chiffré, et l'expéditeur est prouvé : une
//!   enveloppe qui ne vient pas d'un contact connu est jetée.
//! - **Le destinataire décide** : une conversation reçue attend son accord ;
//!   elle n'entre dans l'app qu'acceptée.
//! - **Waly reste scellé** : dépôt et relève passent par la passerelle
//!   séparée (`exterieur::requete`), et s'inscrivent au journal du sceau.
//!
//! Limites assumées (v1) : pas de confidentialité persistante (une clé volée
//! plus tard ouvre les enveloppes encore au relais) ; le relais voit QUI
//! dépose dans QUELLE boîte et quand (jamais le contenu) ; coffre de clés
//! Windows seulement.

use crypto_box::aead::{Aead, AeadCore, OsRng};
use crypto_box::{PublicKey, SalsaBox, SecretKey};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::exterieur;

/// Taille maximale d'une conversation partagée (texte clair).
pub const CLAIR_MAX: usize = 300_000;
const PREFIXE: &str = "waly1-";

// --- Codages (pas de la cryptographie) --------------------------------------

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn dehex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

const B32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

fn b32(octets: &[u8]) -> String {
    let (mut out, mut acc, mut bits) = (String::new(), 0u32, 0u32);
    for &o in octets {
        acc = (acc << 8) | o as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(B32[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(B32[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn deb32(s: &str) -> Option<Vec<u8>> {
    let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0u32);
    for c in s.bytes() {
        let v = B32.iter().position(|&x| x == c.to_ascii_lowercase())? as u32;
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

// --- Le code Waly : clé publique + boîte + relais ---------------------------

/// Ce qu'un code désigne : à qui chiffrer, et où déposer.
#[derive(Debug, Clone, PartialEq)]
pub struct Adresse {
    pub cle: [u8; 32],
    /// Identifiant de la boîte au relais (32 caractères hexadécimaux).
    pub boite: String,
    pub relais: String,
}

/// `waly1-<base32(clé ‖ boîte ‖ contrôle)>@<relais>` — le contrôle (CRC32)
/// attrape les fautes de recopie ; ce n'est pas un secret.
pub fn code(a: &Adresse) -> String {
    let mut brut = a.cle.to_vec();
    brut.extend(dehex(&a.boite).unwrap_or_default());
    let crc = crc32fast::hash(&brut);
    brut.extend(crc.to_be_bytes());
    format!("{PREFIXE}{}@{}", b32(&brut), a.relais)
}

pub fn lire_code(texte: &str) -> Result<Adresse, String> {
    let t: String = texte.chars().filter(|c| !c.is_whitespace()).collect();
    let reste = t.strip_prefix(PREFIXE).ok_or("ce n'est pas un code Waly (il commence par waly1-)")?;
    let (corps, relais) = reste.split_once('@').ok_or("code incomplet : il manque l'adresse du relais")?;
    let brut = deb32(corps).filter(|b| b.len() >= 52).ok_or("code illisible")?;
    let (donnees, crc) = brut[..52].split_at(48);
    if crc32fast::hash(donnees).to_be_bytes() != crc {
        return Err("code mal recopié (contrôle faux)".into());
    }
    let relais = relais.trim_end_matches('/').to_string();
    exterieur::url_admise(&relais)?;
    let mut cle = [0u8; 32];
    cle.copy_from_slice(&donnees[..32]);
    Ok(Adresse { cle, boite: hex(&donnees[32..48]), relais })
}

// --- Enveloppes (pur, testable partout) --------------------------------------

#[derive(Serialize, Deserialize)]
struct Enveloppe {
    v: u8,
    /// Clé publique de l'expéditeur (hex).
    de: String,
    /// Nonce (hex, 24 octets, tiré au hasard à chaque enveloppe).
    n: String,
    /// Chiffré + sceau d'authenticité (hex).
    c: String,
}

/// Chiffre `clair` pour `dest`, de la part du porteur de `sk`.
pub fn sceller(sk: &[u8; 32], dest: &[u8; 32], clair: &[u8]) -> Result<String, String> {
    if clair.len() > CLAIR_MAX {
        return Err(format!("trop long à partager ({} Ko, limite {} Ko)", clair.len() / 1000, CLAIR_MAX / 1000));
    }
    let sk = SecretKey::from(*sk);
    let boite = SalsaBox::new(&PublicKey::from(*dest), &sk);
    let nonce = SalsaBox::generate_nonce(&mut OsRng);
    let chiffre = boite.encrypt(&nonce, clair).map_err(|_| "chiffrement impossible".to_string())?;
    serde_json::to_string(&Enveloppe { v: 1, de: hex(sk.public_key().as_bytes()), n: hex(&nonce), c: hex(&chiffre) })
        .map_err(|e| e.to_string())
}

/// Ouvre une enveloppe. `connu` dit si l'expéditeur annoncé est un contact :
/// sinon elle est refusée AVANT tout déchiffrement. Le déchiffrement réussi
/// prouve que l'enveloppe vient bien de lui et n'a pas été modifiée.
pub fn ouvrir(sk: &[u8; 32], enveloppe: &str, connu: impl Fn(&[u8; 32]) -> bool) -> Result<([u8; 32], Vec<u8>), String> {
    let e: Enveloppe = serde_json::from_str(enveloppe).map_err(|_| "enveloppe illisible".to_string())?;
    if e.v != 1 {
        return Err("version d'enveloppe inconnue".into());
    }
    let de: [u8; 32] = dehex(&e.de).and_then(|d| d.try_into().ok()).ok_or("expéditeur illisible")?;
    if !connu(&de) {
        return Err("expéditeur inconnu (pas dans tes contacts)".into());
    }
    let nonce = dehex(&e.n).filter(|n| n.len() == 24).ok_or("enveloppe illisible")?;
    let chiffre = dehex(&e.c).ok_or("enveloppe illisible")?;
    let boite = SalsaBox::new(&PublicKey::from(de), &SecretKey::from(*sk));
    let clair = boite
        .decrypt(crypto_box::aead::generic_array::GenericArray::from_slice(&nonce), chiffre.as_slice())
        .map_err(|_| "enveloppe falsifiée ou qui ne t'est pas destinée".to_string())?;
    Ok((de, clair))
}

/// Ce qui voyage dans l'enveloppe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Charge {
    pub genre: String,
    pub titre: String,
    /// Le nom que l'expéditeur se donne (affichage ; l'identité, c'est la clé).
    pub de: String,
    pub messages: Vec<(String, String)>,
}

// --- Identité et contacts (stockage) -----------------------------------------

fn tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS partage_identite (
           id      INTEGER PRIMARY KEY CHECK(id = 1),
           cle     BLOB NOT NULL,
           secrete BLOB NOT NULL,
           boite   TEXT NOT NULL,
           jeton   BLOB NOT NULL,
           relais  TEXT NOT NULL DEFAULT '',
           nom     TEXT NOT NULL DEFAULT ''
         );
         CREATE TABLE IF NOT EXISTS partage_contacts (
           id         INTEGER PRIMARY KEY,
           nom        TEXT NOT NULL,
           cle        BLOB NOT NULL UNIQUE,
           boite      TEXT NOT NULL,
           relais     TEXT NOT NULL,
           created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
         );
         CREATE TABLE IF NOT EXISTS partage_recus (
           id         INTEGER PRIMARY KEY,
           contact_id INTEGER NOT NULL,
           titre      TEXT NOT NULL,
           charge     TEXT NOT NULL,
           etat       TEXT NOT NULL DEFAULT 'attente',
           recu_at    TEXT NOT NULL DEFAULT (datetime('now','localtime'))
         );",
    )
}

/// L'identité de cette installation (la clé secrète ne sort jamais d'ici).
pub struct Identite {
    pub cle: [u8; 32],
    secrete: [u8; 32],
    pub boite: String,
    jeton: String,
    pub relais: String,
    pub nom: String,
}

fn alea<const N: usize>() -> [u8; N] {
    use crypto_box::aead::rand_core::RngCore;
    let mut b = [0u8; N];
    OsRng.fill_bytes(&mut b);
    b
}

/// L'identité, créée à la première demande (clés tirées sur la machine,
/// secrets chiffrés par Windows avant d'entrer en base).
pub fn identite(conn: &Connection) -> Result<Identite, String> {
    tables(conn).map_err(|e| e.to_string())?;
    let existe: Option<(Vec<u8>, Vec<u8>, String, Vec<u8>, String, String)> = conn
        .query_row("SELECT cle, secrete, boite, jeton, relais, nom FROM partage_identite WHERE id=1", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
        })
        .ok();
    if let Some((cle, secrete, boite, jeton, relais, nom)) = existe {
        let secrete: [u8; 32] = exterieur::deproteger(&secrete)?.try_into().map_err(|_| "clé illisible".to_string())?;
        return Ok(Identite {
            cle: cle.try_into().map_err(|_| "clé illisible".to_string())?,
            secrete,
            boite,
            jeton: String::from_utf8(exterieur::deproteger(&jeton)?).map_err(|_| "jeton illisible".to_string())?,
            relais,
            nom,
        });
    }
    let sk = SecretKey::generate(&mut OsRng);
    let i = Identite {
        cle: *sk.public_key().as_bytes(),
        secrete: sk.to_bytes(),
        boite: hex(&alea::<16>()),
        jeton: hex(&alea::<32>()),
        relais: String::new(),
        nom: String::new(),
    };
    conn.execute(
        "INSERT INTO partage_identite(id, cle, secrete, boite, jeton) VALUES (1, ?1, ?2, ?3, ?4)",
        params![i.cle.to_vec(), exterieur::proteger(&i.secrete)?, i.boite, exterieur::proteger(i.jeton.as_bytes())?],
    )
    .map_err(|e| e.to_string())?;
    Ok(i)
}

/// Un relais est-il réglé ? (Sans créer d'identité : la relève de fond ne
/// tourne que pour qui a ouvert le partage.)
pub fn actif(conn: &Connection) -> bool {
    conn.query_row("SELECT relais <> '' FROM partage_identite WHERE id=1", [], |r| r.get(0)).unwrap_or(false)
}

/// Pose le relais où cette installation reçoit, et le nom qu'elle se donne.
pub fn regler(conn: &Connection, relais: &str, nom: &str) -> Result<(), String> {
    identite(conn)?;
    let relais = relais.trim().trim_end_matches('/');
    if !relais.is_empty() {
        exterieur::url_admise(relais)?;
    }
    let nom: String = nom.trim().chars().take(40).collect();
    conn.execute("UPDATE partage_identite SET relais=?1, nom=?2 WHERE id=1", params![relais, nom]).map_err(|e| e.to_string())?;
    Ok(())
}

/// Le code à donner à un contact (exige un relais : sans lui, personne ne
/// peut t'écrire).
pub fn mon_code(conn: &Connection) -> Result<String, String> {
    let i = identite(conn)?;
    if i.relais.is_empty() {
        return Err("indique d'abord l'adresse de ton relais".into());
    }
    Ok(code(&Adresse { cle: i.cle, boite: i.boite, relais: i.relais }))
}

#[derive(Debug, Clone, Serialize)]
pub struct Contact {
    pub id: i64,
    pub nom: String,
    #[serde(skip)]
    pub cle: [u8; 32],
    #[serde(skip)]
    pub boite: String,
    pub relais: String,
    /// Empreinte courte de sa clé (à comparer de vive voix si l'on veut).
    pub empreinte: String,
}

pub fn contacts(conn: &Connection) -> Vec<Contact> {
    if tables(conn).is_err() {
        return Vec::new();
    }
    let Ok(mut st) = conn.prepare("SELECT id, nom, cle, boite, relais FROM partage_contacts ORDER BY nom") else {
        return Vec::new();
    };
    st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Vec<u8>>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?)))
        .map(|rows| {
            rows.filter_map(Result::ok)
                .filter_map(|(id, nom, cle, boite, relais)| {
                    let cle: [u8; 32] = cle.try_into().ok()?;
                    Some(Contact { id, nom, empreinte: b32(&cle[..5]), cle, boite, relais })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Ajoute un contact par son code (ou met à jour sa boîte s'il existe déjà).
pub fn ajouter_contact(conn: &Connection, nom: &str, code_contact: &str) -> Result<i64, String> {
    let a = lire_code(code_contact)?;
    let moi = identite(conn)?;
    if a.cle == moi.cle {
        return Err("c'est ton propre code".into());
    }
    let nom: String = nom.trim().chars().take(40).collect();
    if nom.is_empty() {
        return Err("donne un nom à ce contact".into());
    }
    conn.execute(
        "INSERT INTO partage_contacts(nom, cle, boite, relais) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(cle) DO UPDATE SET nom=?1, boite=?3, relais=?4",
        params![nom, a.cle.to_vec(), a.boite, a.relais],
    )
    .map_err(|e| e.to_string())?;
    conn.query_row("SELECT id FROM partage_contacts WHERE cle=?1", params![a.cle.to_vec()], |r| r.get(0)).map_err(|e| e.to_string())
}

pub fn retirer_contact(conn: &Connection, id: i64) -> Result<bool, String> {
    tables(conn).map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM partage_contacts WHERE id=?1", params![id]).map(|n| n > 0).map_err(|e| e.to_string())
}

// --- Relais : dépôt, relève ---------------------------------------------------

/// Envoie une conversation à un contact : chiffrée ici, déposée dans SA
/// boîte, sur SON relais. Rend le nombre d'octets partis.
pub fn envoyer_conversation(conn: &Connection, contact_id: i64, titre: &str, messages: &[(String, String)]) -> Result<usize, String> {
    let moi = identite(conn)?;
    let c = contacts(conn).into_iter().find(|c| c.id == contact_id).ok_or("contact introuvable")?;
    let charge = Charge { genre: "conversation".into(), titre: titre.chars().take(120).collect(), de: moi.nom.clone(), messages: messages.to_vec() };
    let clair = serde_json::to_vec(&charge).map_err(|e| e.to_string())?;
    let enveloppe = sceller(&moi.secrete, &c.cle, &clair)?;
    let (statut, _) = exterieur::requete("POST", &format!("{}/v1/boites/{}", c.relais, c.boite), &["Content-Type: application/json".to_string()], Some(&enveloppe), 60)?;
    match statut.as_str() {
        "200" | "201" => Ok(enveloppe.len()),
        "404" => Err(format!("{} n'a pas encore ouvert sa boîte sur ce relais (son Waly doit avoir tourné une fois)", c.nom)),
        "413" => Err("trop volumineux pour le relais".into()),
        "429" | "507" => Err(format!("la boîte de {} est pleine", c.nom)),
        s => Err(format!("le relais a répondu {s}")),
    }
}

/// Une conversation reçue, en attente de la décision de l'utilisateur.
#[derive(Debug, Clone, Serialize)]
pub struct Recu {
    pub id: i64,
    pub contact: String,
    pub titre: String,
    pub messages: usize,
    pub recu_at: String,
}

/// Relève la boîte (attente longue côté relais) : chaque enveloppe est
/// ouverte ; celles d'un contact connu attendent l'accord de l'utilisateur,
/// les autres sont jetées. Rend (reçues, jetées). Ouvre la boîte au relais
/// au premier appel.
pub fn relever(conn: &Connection, attente_s: u32) -> Result<(Vec<Recu>, usize), String> {
    let moi = identite(conn)?;
    if moi.relais.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let base = format!("{}/v1/boites/{}", moi.relais, moi.boite);
    let auth = format!("Authorization: Bearer {}", moi.jeton);
    let (statut, corps) = exterieur::requete("GET", &base, &[auth.clone(), format!("X-Attente: {attente_s}")], None, attente_s + 15)?;
    if statut != "200" {
        return Err(match statut.as_str() {
            "403" => "le relais refuse cette boîte (jeton différent : boîte déjà prise ?)".into(),
            s => format!("le relais a répondu {s}"),
        });
    }
    let v: serde_json::Value = serde_json::from_str(&corps).map_err(|_| "réponse illisible du relais".to_string())?;
    let connus = contacts(conn);
    let (mut recus, mut jetes) = (Vec::new(), 0);
    for m in v["messages"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let (Some(id), Some(env)) = (m["id"].as_u64(), m["corps"].as_str()) else { continue };
        let ouvert = ouvrir(&moi.secrete, env, |de| connus.iter().any(|c| &c.cle == de))
            .and_then(|(de, clair)| serde_json::from_slice::<Charge>(&clair).map(|c| (de, c)).map_err(|_| "contenu illisible".to_string()));
        match ouvert {
            Ok((de, charge)) if charge.genre == "conversation" => {
                let contact = connus.iter().find(|c| c.cle == de).ok_or("contact introuvable")?;
                conn.execute(
                    "INSERT INTO partage_recus(contact_id, titre, charge) VALUES (?1, ?2, ?3)",
                    params![contact.id, charge.titre, serde_json::to_string(&charge).unwrap_or_default()],
                )
                .map_err(|e| e.to_string())?;
                recus.push(Recu {
                    id: conn.last_insert_rowid(),
                    contact: contact.nom.clone(),
                    titre: charge.titre.clone(),
                    messages: charge.messages.len(),
                    recu_at: String::new(),
                });
            }
            _ => jetes += 1,
        }
        // Traitée (gardée ou jetée) : elle quitte le relais.
        let _ = exterieur::requete("DELETE", &format!("{base}/{id}"), &[auth.clone()], None, 20);
    }
    Ok((recus, jetes))
}

/// Conversations reçues en attente d'accord.
pub fn recus(conn: &Connection) -> Vec<Recu> {
    if tables(conn).is_err() {
        return Vec::new();
    }
    let Ok(mut st) = conn.prepare(
        "SELECT r.id, COALESCE(c.nom, 'contact retiré'), r.titre, r.charge, r.recu_at
         FROM partage_recus r LEFT JOIN partage_contacts c ON c.id = r.contact_id
         WHERE r.etat = 'attente' ORDER BY r.id DESC",
    ) else {
        return Vec::new();
    };
    st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?)))
        .map(|rows| {
            rows.filter_map(Result::ok)
                .map(|(id, contact, titre, charge, recu_at)| Recu {
                    id,
                    contact,
                    titre,
                    messages: serde_json::from_str::<Charge>(&charge).map(|c| c.messages.len()).unwrap_or(0),
                    recu_at,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Décision de l'utilisateur. Acceptée : la conversation entre dans l'app
/// (nouvelle conversation, titre « reçue de … ») ; rend son id. Refusée :
/// elle est effacée.
pub fn decider(conn: &Connection, id: i64, accepter: bool) -> Result<Option<i64>, String> {
    let (contact, charge): (String, String) = conn
        .query_row(
            "SELECT COALESCE(c.nom, 'contact retiré'), r.charge FROM partage_recus r
             LEFT JOIN partage_contacts c ON c.id = r.contact_id WHERE r.id=?1 AND r.etat='attente'",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "partage introuvable".to_string())?;
    if !accepter {
        conn.execute("DELETE FROM partage_recus WHERE id=?1", params![id]).map_err(|e| e.to_string())?;
        return Ok(None);
    }
    let charge: Charge = serde_json::from_str(&charge).map_err(|_| "contenu illisible".to_string())?;
    let titre: String = format!("{} · reçue de {contact}", charge.titre).chars().take(120).collect();
    let session = crate::store::create_session(conn, &titre, "chat").map_err(|e| e.to_string())?;
    for (role, texte) in &charge.messages {
        let role = if role == "user" { "user" } else { "assistant" };
        crate::store::append_message_in(conn, session, role, texte).map_err(|e| e.to_string())?;
    }
    conn.execute("DELETE FROM partage_recus WHERE id=?1", params![id]).map_err(|e| e.to_string())?;
    Ok(Some(session))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cles() -> ([u8; 32], [u8; 32]) {
        let sk = SecretKey::generate(&mut OsRng);
        (sk.to_bytes(), *sk.public_key().as_bytes())
    }

    #[test]
    fn enveloppe_aller_retour_et_refus() {
        let (sk_a, pk_a) = cles();
        let (sk_b, pk_b) = cles();
        let (sk_c, _) = cles();
        let env = sceller(&sk_a, &pk_b, "bonjour à toi".as_bytes()).unwrap();
        assert!(!env.contains("bonjour"), "le relais ne voit que du chiffré");
        let (de, clair) = ouvrir(&sk_b, &env, |k| *k == pk_a).unwrap();
        assert_eq!((de, clair.as_slice()), (pk_a, "bonjour à toi".as_bytes()));
        // Expéditeur hors contacts : refusé avant tout déchiffrement.
        assert!(ouvrir(&sk_b, &env, |_| false).unwrap_err().contains("inconnu"));
        // Pas le destinataire : indéchiffrable.
        assert!(ouvrir(&sk_c, &env, |_| true).is_err());
        // Un octet modifié en route : rejeté (authenticité).
        let mut e: Enveloppe = serde_json::from_str(&env).unwrap();
        let dernier = if e.c.ends_with('0') { "1" } else { "0" };
        e.c.replace_range(e.c.len() - 1.., dernier);
        assert!(ouvrir(&sk_b, &serde_json::to_string(&e).unwrap(), |_| true).is_err());
        // Usurpation : C annonce la clé de A sans avoir sa clé secrète.
        let faux = sceller(&sk_c, &pk_b, b"je suis A").unwrap().replace(&hex(SecretKey::from(sk_c).public_key().as_bytes()), &hex(&pk_a));
        assert!(ouvrir(&sk_b, &faux, |k| *k == pk_a).is_err());
        // Deux enveloppes du même texte diffèrent (nonce tiré à chaque fois).
        assert_ne!(env, sceller(&sk_a, &pk_b, "bonjour à toi".as_bytes()).unwrap());
        assert!(sceller(&sk_a, &pk_b, &vec![0u8; CLAIR_MAX + 1]).is_err());
    }

    #[test]
    fn code_aller_retour_et_fautes_de_recopie() {
        let (_, pk) = cles();
        let a = Adresse { cle: pk, boite: hex(&[7u8; 16]), relais: "https://relais.exemple.fr".into() };
        let c = code(&a);
        assert!(c.starts_with("waly1-") && c.ends_with("@https://relais.exemple.fr"));
        assert_eq!(lire_code(&c).unwrap(), a);
        // Espaces et retours à la ligne d'un copier-coller : tolérés.
        assert_eq!(lire_code(&format!("  {}\n{} ", &c[..20], &c[20..])).unwrap(), a);
        // Une lettre changée : le contrôle l'attrape.
        let i = 10;
        let autre = if c.as_bytes()[i] == b'a' { 'b' } else { 'a' };
        let faux = format!("{}{autre}{}", &c[..i], &c[i + 1..]);
        assert!(lire_code(&faux).unwrap_err().contains("recopié"));
        assert!(lire_code("bonjour").is_err());
        assert!(lire_code(&c.replace("https://relais.exemple.fr", "http://relais.exemple.fr")).is_err(), "http vers internet refusé");
    }

    #[test]
    fn codages() {
        assert_eq!(dehex(&hex(&[0, 1, 254, 255])).unwrap(), vec![0, 1, 254, 255]);
        assert!(dehex("abc").is_none() && dehex("zz").is_none());
        for n in [0usize, 1, 5, 32, 52] {
            let b: Vec<u8> = (0..n as u8).collect();
            assert_eq!(&deb32(&b32(&b)).unwrap()[..n], b.as_slice());
        }
    }
}
