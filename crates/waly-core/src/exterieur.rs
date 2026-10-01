//! Modèles extérieurs avec la clé de l'utilisateur + routeur (lot 3 des
//! « bientôt », 2026-10-01 — R8 « BYOK » du RFC 2026-07-20).
//!
//! Règles (ADR 2026-09-14, imposées ICI par le code, jamais confiées au
//! modèle) :
//! 1. **Waly reste scellé.** L'appel sort par un PROCESSUS PASSERELLE séparé
//!    (`curl` du système : signé, TLS du système, hors du périmètre scellé),
//!    vers la seule destination que l'utilisateur a déclarée. Chaque appel
//!    est inscrit au journal du sceau par l'appelant.
//! 2. **Un modèle extérieur ne reçoit ni la mémoire, ni les fichiers, ni
//!    l'écran, ni les outils** : seulement le texte de la conversation
//!    affichée. Les fichiers joints sont retirés ([`sans_fichiers`]) sauf
//!    autorisation explicite pour CE message.
//! 3. **Le routeur choisit un modèle ; il ne décide pas de ce qui sort** :
//!    tout ce qui touche aux outils, à la mémoire, aux fichiers, à l'écran ou
//!    aux images reste local ([`router`]).
//!
//! La clé est chiffrée par Windows pour le compte de l'utilisateur (DPAPI,
//! `crypt32.dll` — DLL système, piège 3) ; elle ne quitte la base qu'en
//! mémoire, vers l'entrée standard de la passerelle (jamais en argument de
//! ligne de commande, jamais sur disque en clair).
//!
//! Reste à faire (demande une mise à jour du service élevé) : sceller la
//! passerelle elle-même PAR DESTINATION (filtres WFP adresse/port).

use std::io::{BufRead, BufReader, Write};

use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocole {
    /// API Messages d'Anthropic (`POST /v1/messages`, `x-api-key`).
    Anthropic,
    /// Format OpenAI `chat/completions` (Mistral, OpenAI, serveur personnel).
    OpenAi,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Fournisseur {
    pub id: &'static str,
    pub nom: &'static str,
    /// Vide = l'utilisateur donne l'adresse (serveur personnel).
    pub base_url: &'static str,
    pub protocole: Protocole,
    /// Suggestions ; l'utilisateur peut saisir un autre nom.
    pub modeles: &'static [&'static str],
}

pub const FOURNISSEURS: &[Fournisseur] = &[
    Fournisseur {
        id: "anthropic",
        nom: "Claude (Anthropic)",
        base_url: "https://api.anthropic.com/v1",
        protocole: Protocole::Anthropic,
        modeles: &["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-4-5"],
    },
    Fournisseur {
        id: "mistral",
        nom: "Mistral",
        base_url: "https://api.mistral.ai/v1",
        protocole: Protocole::OpenAi,
        modeles: &["mistral-large-latest", "mistral-medium-latest", "mistral-small-latest"],
    },
    Fournisseur {
        id: "openai",
        nom: "OpenAI",
        base_url: "https://api.openai.com/v1",
        protocole: Protocole::OpenAi,
        modeles: &[],
    },
    Fournisseur {
        id: "perso",
        nom: "Ton propre serveur (format OpenAI)",
        base_url: "",
        protocole: Protocole::OpenAi,
        modeles: &[],
    },
];

pub fn fournisseur(id: &str) -> Option<&'static Fournisseur> {
    FOURNISSEURS.iter().find(|f| f.id == id)
}

/// Un modèle extérieur déclaré par l'utilisateur (la clé n'y figure pas).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Config {
    pub id: i64,
    pub fournisseur: String,
    pub base_url: String,
    pub modele: String,
}

impl Config {
    pub fn protocole(&self) -> Protocole {
        fournisseur(&self.fournisseur).map(|f| f.protocole).unwrap_or(Protocole::OpenAi)
    }

    /// Hôte de destination (ce que l'utilisateur voit dans Vie privée).
    pub fn hote(&self) -> String {
        hote(&self.base_url).unwrap_or_default()
    }
}

// --- Validation : rien d'autre ne part vers la passerelle -------------------

fn hote(url: &str) -> Option<String> {
    let reste = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let autorite = reste.split('/').next()?;
    let h = autorite.rsplit_once(':').map_or(autorite, |(h, p)| {
        if p.chars().all(|c| c.is_ascii_digit()) { h } else { autorite }
    });
    (!h.is_empty()).then(|| h.to_ascii_lowercase())
}

fn hote_prive(h: &str) -> bool {
    if h == "localhost" || h.ends_with(".local") || h.ends_with(".lan") {
        return true;
    }
    let o: Vec<u8> = h.split('.').filter_map(|x| x.parse().ok()).collect();
    o.len() == 4
        && h.split('.').count() == 4
        && (o[0] == 127 || o[0] == 10 || (o[0] == 192 && o[1] == 168) || (o[0] == 172 && (16..=31).contains(&o[1])))
}

/// Adresse admise : `https://…` partout ; `http://…` seulement vers la
/// machine elle-même ou le réseau local (serveur personnel). Jamais
/// d'identifiants dans l'adresse, jamais de caractère qui s'échapperait de la
/// configuration de la passerelle.
pub fn url_admise(url: &str) -> Result<(), String> {
    let sure = url.len() <= 300
        && url.chars().all(|c| c.is_ascii_alphanumeric() || "._:/-~%".contains(c));
    if !sure || url.contains('@') {
        return Err("adresse invalide (caractères non admis)".into());
    }
    let h = hote(url).ok_or("adresse invalide : attendu https://hôte/…")?;
    if url.starts_with("http://") && !hote_prive(&h) {
        return Err("une adresse http:// n'est admise que vers cette machine ou ton réseau local — utilise https://".into());
    }
    Ok(())
}

pub fn modele_admis(nom: &str) -> bool {
    !nom.is_empty() && nom.len() <= 120 && nom.chars().all(|c| c.is_ascii_alphanumeric() || "._:/-".contains(c))
}

/// Une clé d'API plausible : ASCII sans espace ni guillemet (elle voyage
/// dans la configuration de la passerelle).
pub fn cle_admise(cle: &str) -> bool {
    (1..=400).contains(&cle.len()) && cle.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

// --- Stockage (clé chiffrée par le système) ---------------------------------

fn table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS modeles_exterieurs (
           id          INTEGER PRIMARY KEY,
           fournisseur TEXT NOT NULL,
           base_url    TEXT NOT NULL,
           modele      TEXT NOT NULL,
           cle         BLOB NOT NULL,
           created_at  TEXT NOT NULL DEFAULT (datetime('now','localtime'))
         );",
    )
}

/// Déclare un modèle extérieur. La clé est chiffrée AVANT d'entrer en base.
pub fn ajouter(conn: &Connection, fournisseur_id: &str, base_url: &str, modele: &str, cle: &str) -> Result<i64, String> {
    let f = fournisseur(fournisseur_id).ok_or("fournisseur inconnu")?;
    let base = if f.base_url.is_empty() { base_url.trim().trim_end_matches('/') } else { f.base_url };
    url_admise(base)?;
    if !modele_admis(modele.trim()) {
        return Err("nom de modèle invalide".into());
    }
    if !cle_admise(cle.trim()) {
        return Err("clé invalide (lettres, chiffres, - _ . seulement)".into());
    }
    let chiffre = proteger(cle.trim().as_bytes())?;
    table(conn).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO modeles_exterieurs(fournisseur, base_url, modele, cle) VALUES (?1, ?2, ?3, ?4)",
        params![f.id, base, modele.trim(), chiffre],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

pub fn lister(conn: &Connection) -> Vec<Config> {
    if table(conn).is_err() {
        return Vec::new();
    }
    let Ok(mut st) = conn.prepare("SELECT id, fournisseur, base_url, modele FROM modeles_exterieurs ORDER BY id") else {
        return Vec::new();
    };
    st.query_map([], |r| {
        Ok(Config { id: r.get(0)?, fournisseur: r.get(1)?, base_url: r.get(2)?, modele: r.get(3)? })
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

pub fn trouver(conn: &Connection, id: i64) -> Option<Config> {
    lister(conn).into_iter().find(|c| c.id == id)
}

/// Retire un modèle extérieur ET sa clé.
pub fn retirer(conn: &Connection, id: i64) -> Result<bool, String> {
    table(conn).map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM modeles_exterieurs WHERE id=?1", params![id])
        .map(|n| n > 0)
        .map_err(|e| e.to_string())
}

fn cle(conn: &Connection, id: i64) -> Result<String, String> {
    let chiffre: Vec<u8> = conn
        .query_row("SELECT cle FROM modeles_exterieurs WHERE id=?1", params![id], |r| r.get(0))
        .map_err(|_| "modèle extérieur introuvable".to_string())?;
    String::from_utf8(deproteger(&chiffre)?).map_err(|_| "clé illisible".to_string())
}

#[cfg(windows)]
fn dpapi(entree: &[u8], chiffrer: bool) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB};
    // Pas d'interface : un échec doit rester un échec, jamais un dialogue.
    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;
    let src = CRYPT_INTEGER_BLOB { cbData: entree.len() as u32, pbData: entree.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    let ok = unsafe {
        if chiffrer {
            CryptProtectData(&src, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        } else {
            CryptUnprotectData(&src, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        }
    };
    if ok == 0 || out.pbData.is_null() {
        return Err("le coffre de Windows a refusé (clé enregistrée sous un autre compte ?)".into());
    }
    let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
    unsafe { LocalFree(out.pbData as _) };
    Ok(v)
}

#[cfg(windows)]
pub(crate) fn proteger(clair: &[u8]) -> Result<Vec<u8>, String> {
    dpapi(clair, true)
}

#[cfg(windows)]
pub(crate) fn deproteger(chiffre: &[u8]) -> Result<Vec<u8>, String> {
    dpapi(chiffre, false)
}

#[cfg(not(windows))]
pub(crate) fn proteger(_clair: &[u8]) -> Result<Vec<u8>, String> {
    Err("coffre de clés : Windows uniquement pour l'instant".into())
}

#[cfg(not(windows))]
pub(crate) fn deproteger(_chiffre: &[u8]) -> Result<Vec<u8>, String> {
    Err("coffre de clés : Windows uniquement pour l'instant".into())
}

// --- Ce qui sort : la conversation affichée, sans les fichiers --------------

const DEBUT_FICHIER: &str = "--- Contenu du fichier joint : ";
const FIN_FICHIER: &str = "--- fin du fichier ---";

/// Un message contient-il un fichier joint (bloc posé par la saisie) ?
pub fn contient_fichier(message: &str) -> bool {
    message.contains(DEBUT_FICHIER)
}

/// Retire les fichiers joints d'un message : seul leur NOM reste, le contenu
/// ne part pas (règle 2). Un bloc jamais refermé est retiré jusqu'au bout.
pub fn sans_fichiers(message: &str) -> String {
    let mut out = String::new();
    let mut reste = message;
    while let Some(i) = reste.find(DEBUT_FICHIER) {
        out.push_str(reste[..i].trim_end());
        let apres = &reste[i + DEBUT_FICHIER.len()..];
        let nom = apres.split(" ---").next().unwrap_or("").trim();
        out.push_str(&format!("\n\n[fichier joint « {nom} » : resté sur la machine, non transmis]"));
        reste = match apres.find(FIN_FICHIER) {
            Some(j) => &apres[j + FIN_FICHIER.len()..],
            None => "",
        };
    }
    out.push_str(reste);
    out
}

/// Consigne du modèle extérieur : il sait ce qu'il n'a PAS (honnêteté), et
/// ne reçoit ni nom, ni mémoire, ni instructions personnelles.
pub fn systeme(longueur: &str) -> String {
    format!(
        "Tu es le modèle extérieur que Waly, un assistant personnel local, appelle à la demande \
         de son utilisateur. Réponds en français, {longueur}. Tu ne reçois que le texte de cette \
         conversation : tu n'as accès ni à la mémoire de Waly, ni aux fichiers, ni à l'écran, ni \
         à des outils (rappels, notes, tâches, recherche). Si la demande en a besoin, dis-le en \
         une phrase : Waly la traitera avec son modèle local. {}",
        crate::clock::french_timestamp()
    )
}

/// Ce qui part réellement : (rôle, texte). Les premiers messages de Waly sont
/// écartés (une conversation commence par l'utilisateur), les fichiers joints
/// retirés partout — sauf dans le DERNIER message si `fichiers_autorises`.
pub fn conversation_sortante(historique: &[(String, String)], fichiers_autorises: bool) -> Vec<(String, String)> {
    let debut = historique.iter().position(|(r, _)| r == "user").unwrap_or(historique.len());
    let dernier = historique.len().saturating_sub(1);
    historique
        .iter()
        .enumerate()
        .skip(debut)
        .map(|(i, (role, texte))| {
            let role = if role == "user" { "user" } else { "assistant" };
            let texte = if fichiers_autorises && i == dernier { texte.clone() } else { sans_fichiers(texte) };
            (role.to_string(), texte)
        })
        .filter(|(_, t)| !t.trim().is_empty())
        .collect()
}

// --- Routeur ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cible {
    Local,
    Exterieur,
}

/// Ce que le routeur sait du tour (jamais le contenu de la mémoire).
#[derive(Debug, Clone, Copy, Default)]
pub struct Tour<'a> {
    pub message: &'a str,
    pub mission: bool,
    pub ecran: bool,
    pub image: bool,
    /// Le message appelle des outils (sélection d'outils non vide).
    pub outils: bool,
}

/// Routeur « Auto » : PUR et lisible, la raison est montrée à l'utilisateur.
/// Local dès que le tour touche à ce qui ne doit pas sortir ou à ce que seul
/// le local sait faire ; extérieur pour le travail de fond sur du texte.
pub fn router(t: &Tour) -> (Cible, &'static str) {
    if t.mission {
        return (Cible::Local, "mission : les outils restent locaux");
    }
    if t.ecran || t.image {
        return (Cible::Local, "écran ou image : reste sur la machine");
    }
    if contient_fichier(t.message) {
        return (Cible::Local, "fichier joint : reste sur la machine");
    }
    let m = crate::selection::normaliser(t.message);
    const PERSO: &[&str] = &[
        "souviens", "rappelle", "retiens", "memoire", "mes notes", "ma note", "mes taches", "mon agenda",
        "mes fichiers", "mon fichier", "mon dossier", "mes documents", "tu sais sur moi", "de moi",
    ];
    if t.outils || PERSO.iter().any(|k| m.contains(k)) {
        return (Cible::Local, "outils ou mémoire : reste local");
    }
    const FOND: &[&str] = &[
        "redige", "ecris", "reecris", "reformule", "traduis", "resume", "analyse", "compare", "explique",
        "demontre", "argumente", "plan detaille", "strategie", "code", "programme", "fonction", "script",
        "algorithme", "corrige", "ameliore", "pourquoi", "avantages", "inconvenients", "pas a pas",
    ];
    if t.message.chars().count() > 400 || FOND.iter().any(|k| m.contains(k)) {
        return (Cible::Exterieur, "travail de fond sur du texte");
    }
    (Cible::Local, "question simple")
}

// --- Appel par la passerelle -------------------------------------------------

/// Issue d'un tour extérieur.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reponse {
    pub texte: String,
    /// Plafond de sortie atteint.
    pub tronque: bool,
    /// Le fournisseur a décliné la demande (garde-fous).
    pub refus: bool,
    pub interrompu: bool,
    pub entree: Option<u64>,
    pub sortie: Option<u64>,
}

/// Plafond de sortie d'un tour (réflexion du modèle comprise).
const MAX_SORTIE: u32 = 16_000;

/// Corps de la requête (PUR, testable).
pub fn corps(cfg: &Config, systeme: &str, messages: &[(String, String)]) -> serde_json::Value {
    let msgs = |avec_systeme: bool| -> Vec<serde_json::Value> {
        let tete = avec_systeme.then(|| serde_json::json!({"role": "system", "content": systeme}));
        tete.into_iter()
            .chain(messages.iter().map(|(r, t)| serde_json::json!({"role": r, "content": t})))
            .collect()
    };
    match cfg.protocole() {
        Protocole::Anthropic => {
            // Ni température ni réglage de réflexion : les modèles récents les
            // refusent ; la réflexion adaptative est leur défaut.
            let mut b = serde_json::json!({
                "model": cfg.modele,
                "max_tokens": MAX_SORTIE,
                "stream": true,
                "system": systeme,
                "messages": msgs(false),
            });
            // Garde-fous : un refus est rejoué côté serveur sur le modèle de
            // repli recommandé (modèles qui l'acceptent seulement).
            if repli_serveur(&cfg.modele) {
                b["fallbacks"] = serde_json::json!("default");
            }
            b
        }
        Protocole::OpenAi => serde_json::json!({
            "model": cfg.modele,
            "stream": true,
            "stream_options": {"include_usage": true},
            "messages": msgs(true),
        }),
    }
}

fn repli_serveur(modele: &str) -> bool {
    ["claude-opus-5", "claude-sonnet-5-5", "claude-fable-5"].iter().any(|p| modele.starts_with(p))
}

/// En-têtes de la requête, clé comprise (jamais journalisés).
fn en_tetes(cfg: &Config, cle: &str) -> Vec<String> {
    let mut h = vec!["Content-Type: application/json".to_string()];
    match cfg.protocole() {
        Protocole::Anthropic => {
            h.push(format!("x-api-key: {cle}"));
            h.push("anthropic-version: 2023-06-01".into());
            if repli_serveur(&cfg.modele) {
                h.push("anthropic-beta: server-side-fallback-2026-07-01".into());
            }
        }
        Protocole::OpenAi => h.push(format!("Authorization: Bearer {cle}")),
    }
    h
}

fn route(cfg: &Config) -> String {
    match cfg.protocole() {
        Protocole::Anthropic => format!("{}/messages", cfg.base_url),
        Protocole::OpenAi => format!("{}/chat/completions", cfg.base_url),
    }
}

/// Un événement du flux, quel que soit le protocole.
#[derive(Debug, PartialEq)]
pub enum Evt {
    Texte(String),
    Usage { entree: Option<u64>, sortie: Option<u64> },
    /// Fin annoncée : raison d'arrêt du fournisseur.
    Arret(String),
    Erreur(String),
    Rien,
}

/// Lit une ligne SSE de l'API Messages d'Anthropic.
pub fn ligne_anthropic(line: &str) -> Vec<Evt> {
    let Some(data) = line.trim().strip_prefix("data:") else { return vec![Evt::Rien] };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(data.trim()) else { return vec![Evt::Rien] };
    match v["type"].as_str() {
        Some("content_block_delta") if v["delta"]["type"] == "text_delta" => {
            vec![Evt::Texte(v["delta"]["text"].as_str().unwrap_or("").to_string())]
        }
        Some("message_start") => {
            vec![Evt::Usage { entree: v["message"]["usage"]["input_tokens"].as_u64(), sortie: None }]
        }
        Some("message_delta") => {
            let mut e = vec![Evt::Usage { entree: None, sortie: v["usage"]["output_tokens"].as_u64() }];
            if let Some(r) = v["delta"]["stop_reason"].as_str() {
                e.push(Evt::Arret(r.to_string()));
            }
            e
        }
        Some("error") => vec![Evt::Erreur(v["error"]["message"].as_str().unwrap_or("erreur du fournisseur").to_string())],
        _ => vec![Evt::Rien],
    }
}

/// Lit une ligne SSE au format OpenAI.
pub fn ligne_openai(line: &str) -> Vec<Evt> {
    let mut e = Vec::new();
    if let Some((p, c)) = crate::llm::usage_sse(line) {
        e.push(Evt::Usage { entree: Some(p), sortie: Some(c) });
    }
    if let crate::llm::SseEvent::Delta(t) = crate::llm::parse_sse_line(line) {
        e.push(Evt::Texte(t));
    }
    if let Some(data) = line.trim().strip_prefix("data:") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(data.trim()) {
            if let Some(r) = v["choices"][0]["finish_reason"].as_str() {
                e.push(Evt::Arret(r.to_string()));
            }
        }
    }
    e
}

/// Message d'erreur lisible d'un corps d'erreur HTTP (JSON des fournisseurs).
pub fn message_erreur(code: &str, corps: &str) -> String {
    let detail = serde_json::from_str::<serde_json::Value>(corps.trim())
        .ok()
        .and_then(|v| {
            v["error"]["message"].as_str().or_else(|| v["error"].as_str()).or_else(|| v["message"].as_str()).map(String::from)
        })
        .unwrap_or_else(|| corps.trim().chars().take(200).collect());
    let sens = match code {
        "401" | "403" => " — clé refusée",
        "404" => " — modèle ou adresse introuvable",
        "429" => " — quota ou débit dépassé",
        "000" => " — destination injoignable",
        _ => "",
    };
    format!("le fournisseur a répondu {code}{sens} : {detail}")
}

/// Texte entre guillemets d'une configuration de passerelle.
fn cite(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn passerelle() -> String {
    #[cfg(windows)]
    {
        // Chemin absolu du curl de Windows : jamais celui du PATH.
        let racine = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        format!(r"{racine}\System32\curl.exe")
    }
    #[cfg(not(windows))]
    {
        "curl".into()
    }
}

/// Une requête SIMPLE par la passerelle (messageries, relais de partage) :
/// `methode` = GET | POST | DELETE, `corps` éventuel. Rend (code HTTP, corps de la réponse). L'adresse et les
/// en-têtes peuvent porter un secret : ils ne voyagent que par l'entrée
/// standard de la passerelle, et aucune erreur ne les recopie.
pub(crate) fn requete(methode: &str, url: &str, en_tetes: &[String], corps: Option<&str>, max_s: u32) -> Result<(String, String), String> {
    use std::process::{Command, Stdio};
    url_admise(url)?;
    if !["GET", "POST", "DELETE"].contains(&methode) {
        return Err("méthode non admise".into());
    }
    // Un fichier par fil : messageries et partage relèvent en parallèle.
    let fil = format!("{:?}", std::thread::current().id()).replace(|c: char| !c.is_ascii_digit(), "");
    let fichier = std::env::temp_dir().join(format!("waly-sortie-{}-{fil}.json", std::process::id()));
    let mut config = format!("url = {}\nrequest = {}\n", cite(url), cite(methode));
    if let Some(c) = corps {
        std::fs::write(&fichier, c).map_err(|e| e.to_string())?;
        config.push_str(&format!("data-binary = {}\n", cite(&format!("@{}", fichier.display()))));
    }
    for h in en_tetes {
        config.push_str(&format!("header = {}\n", cite(h)));
    }
    let proto = if url.starts_with("http://") { "=http" } else { "=https" };
    let mut cmd = Command::new(passerelle());
    cmd.args(["--silent", "--connect-timeout", "20", "--max-time", &max_s.to_string()])
        .args(["--proto", proto, "--write-out", r"\nWALY_HTTP %{http_code}", "--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let res = (|| {
        let mut child = cmd.spawn().map_err(|e| format!("passerelle introuvable : {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(config.as_bytes()).map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        let texte = String::from_utf8_lossy(&out.stdout).into_owned();
        let (corps, code) = texte.rsplit_once("\nWALY_HTTP ").unwrap_or((texte.as_str(), "000"));
        let code = code.trim().to_string();
        if code == "000" {
            return Err("destination injoignable (réseau coupé, adresse fausse ou délai dépassé)".to_string());
        }
        Ok((code, corps.to_string()))
    })();
    let _ = std::fs::remove_file(&fichier);
    res
}

/// Joue un tour avec un modèle extérieur. `messages` = ce qui SORT (déjà
/// filtré par [`conversation_sortante`]). `on_delta("")` sonde l'interruption
/// pendant les attentes ; retourner `false` coupe la passerelle.
pub fn converser(
    conn: &Connection,
    cfg: &Config,
    systeme: &str,
    messages: &[(String, String)],
    mut on_delta: impl FnMut(&str) -> bool,
) -> Result<Reponse, String> {
    url_admise(&cfg.base_url)?;
    if !modele_admis(&cfg.modele) {
        return Err("nom de modèle invalide".into());
    }
    let cle = cle(conn, cfg.id)?;
    // Le corps passe par un fichier temporaire (retiré aussitôt) : la clé,
    // elle, ne voyage que par l'entrée standard de la passerelle.
    let fichier = std::env::temp_dir().join(format!("waly-sortie-{}.json", std::process::id()));
    std::fs::write(&fichier, corps(cfg, systeme, messages).to_string()).map_err(|e| e.to_string())?;
    let res = appeler(cfg, &cle, &fichier, &mut on_delta);
    let _ = std::fs::remove_file(&fichier);
    res
}

fn appeler(
    cfg: &Config,
    cle: &str,
    fichier: &std::path::Path,
    on_delta: &mut impl FnMut(&str) -> bool,
) -> Result<Reponse, String> {
    use std::process::{Command, Stdio};
    let mut config = format!(
        "url = {}\nrequest = \"POST\"\ndata-binary = {}\n",
        cite(&route(cfg)),
        cite(&format!("@{}", fichier.display()))
    );
    for h in en_tetes(cfg, cle) {
        config.push_str(&format!("header = {}\n", cite(&h)));
    }
    let proto = if cfg.base_url.starts_with("http://") { "=http" } else { "=https" };
    let mut cmd = Command::new(passerelle());
    cmd.args(["--silent", "--show-error", "--no-buffer", "--connect-timeout", "20", "--max-time", "900"])
        .args(["--proto", proto, "--write-out", r"\nWALY_HTTP %{http_code}\n", "--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| format!("passerelle introuvable ({}) : {e}", passerelle()))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(config.as_bytes()).map_err(|e| e.to_string())?;
    }
    drop(config);
    let stdout = child.stdout.take().ok_or("passerelle sans sortie")?;
    // La lecture bloque : un fil lit, celui-ci sonde l'interruption.
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for l in BufReader::new(stdout).split(b'\n').map_while(Result::ok) {
            if tx.send(String::from_utf8_lossy(&l).into_owned()).is_err() {
                break;
            }
        }
    });
    let anthropic = cfg.protocole() == Protocole::Anthropic;
    let mut r = Reponse::default();
    let mut brut = String::new();
    let mut code = String::new();
    let mut erreur = None;
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(60)) {
            Ok(ligne) => {
                if let Some(c) = ligne.trim().strip_prefix("WALY_HTTP ") {
                    code = c.trim().to_string();
                    continue;
                }
                let t = ligne.trim();
                if !t.starts_with("data:") && !t.starts_with("event:") && !t.starts_with(':') && !t.is_empty() {
                    brut.push_str(t);
                    continue;
                }
                for e in if anthropic { ligne_anthropic(&ligne) } else { ligne_openai(&ligne) } {
                    match e {
                        Evt::Texte(t) => {
                            r.texte.push_str(&t);
                            if !on_delta(&t) {
                                let _ = child.kill();
                                let _ = child.wait();
                                r.interrompu = true;
                                return Ok(r);
                            }
                        }
                        Evt::Usage { entree, sortie } => {
                            r.entree = entree.or(r.entree);
                            r.sortie = sortie.or(r.sortie);
                        }
                        Evt::Arret(raison) => {
                            r.refus |= raison == "refusal" || raison == "content_filter";
                            r.tronque |= raison == "max_tokens" || raison == "length";
                        }
                        Evt::Erreur(m) => erreur = Some(m),
                        Evt::Rien => {}
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if !on_delta("") {
                    let _ = child.kill();
                    let _ = child.wait();
                    r.interrompu = true;
                    return Ok(r);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let statut = child.wait().map_err(|e| e.to_string())?;
    if let Some(m) = erreur {
        return Err(format!("le fournisseur a interrompu la réponse : {m}"));
    }
    if !code.is_empty() && code != "200" {
        let mut detail = String::new();
        if code == "000" {
            if let Some(mut e) = child.stderr.take() {
                let _ = std::io::Read::read_to_string(&mut e, &mut detail);
            }
        }
        return Err(message_erreur(&code, if detail.trim().is_empty() { &brut } else { &detail }));
    }
    if !statut.success() && r.texte.is_empty() {
        return Err("la passerelle n'a pas pu joindre la destination".into());
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(f: &str, modele: &str) -> Config {
        Config { id: 1, fournisseur: f.into(), base_url: fournisseur(f).unwrap().base_url.into(), modele: modele.into() }
    }

    #[test]
    fn adresses_admises_et_refusees() {
        for u in ["https://api.mistral.ai/v1", "http://127.0.0.1:11434/v1", "http://192.168.1.20:8080/v1", "http://localhost:1234/v1", "http://nas.local/v1"] {
            assert!(url_admise(u).is_ok(), "{u}");
        }
        for u in ["http://api.exemple.com/v1", "ftp://x/v1", "https://u:p@hote/v1", "https://hote/v1\"\nheader = x", "https://", "http://8.8.8.8/v1", "http://172.32.0.1/v1"] {
            assert!(url_admise(u).is_err(), "{u}");
        }
        assert_eq!(cfg("anthropic", "claude-opus-5-5").hote(), "api.anthropic.com");
        assert!(cle_admise("sk-ant-api03_AbC.123") && !cle_admise("a b") && !cle_admise("a\"b") && !cle_admise(""));
    }

    #[test]
    fn les_fichiers_joints_ne_sortent_pas() {
        let m = "Résume ça.\n\n--- Contenu du fichier joint : bilan.docx --- (deja lu pour toi)\nCHIFFRE SECRET 42\n--- fin du fichier ---";
        let s = sans_fichiers(m);
        assert!(s.starts_with("Résume ça.") && s.contains("bilan.docx") && !s.contains("SECRET"));
        // Bloc jamais refermé : retiré jusqu'au bout.
        assert!(!sans_fichiers("x\n--- Contenu du fichier joint : a.txt --- \nSECRET").contains("SECRET"));
        assert_eq!(sans_fichiers("rien à retirer"), "rien à retirer");
        let h = vec![
            ("assistant".to_string(), "Bonjour".to_string()),
            ("user".to_string(), m.to_string()),
            ("assistant".to_string(), "Voici.".to_string()),
            ("user".to_string(), m.to_string()),
        ];
        let sortie = conversation_sortante(&h, false);
        assert_eq!(sortie.len(), 3, "le premier message de Waly est écarté");
        assert!(sortie.iter().all(|(_, t)| !t.contains("SECRET")));
        // Autorisation explicite : seulement le DERNIER message.
        let sortie = conversation_sortante(&h, true);
        assert!(!sortie[0].1.contains("SECRET") && sortie[2].1.contains("SECRET"));
    }

    #[test]
    fn le_routeur_garde_local_ce_qui_ne_doit_pas_sortir() {
        let t = |message| Tour { message, ..Default::default() };
        assert_eq!(router(&t("Quelle heure est-il ?")).0, Cible::Local);
        assert_eq!(router(&t("Rédige une lettre de motivation pour un poste d'infirmier")).0, Cible::Exterieur);
        assert_eq!(router(&t("Explique-moi la différence entre TCP et UDP")).0, Cible::Exterieur);
        assert_eq!(router(&t("Rappelle-moi ce que tu sais sur moi et rédige un portrait")).0, Cible::Local);
        assert_eq!(router(&Tour { message: "Rédige un plan", mission: true, ..Default::default() }).0, Cible::Local);
        assert_eq!(router(&Tour { message: "Analyse ça", image: true, ..Default::default() }).0, Cible::Local);
        assert_eq!(router(&Tour { message: "Analyse ça", ecran: true, ..Default::default() }).0, Cible::Local);
        assert_eq!(router(&Tour { message: "Rédige une note", outils: true, ..Default::default() }).0, Cible::Local);
        let joint = "Analyse.\n--- Contenu du fichier joint : a.txt --- \nx\n--- fin du fichier ---";
        assert_eq!(router(&t(joint)).0, Cible::Local);
    }

    #[test]
    fn corps_selon_le_protocole() {
        let msgs = vec![("user".to_string(), "Bonjour".to_string())];
        let a = corps(&cfg("anthropic", "claude-opus-5-5"), "SYS", &msgs);
        assert_eq!((a["system"].as_str(), a["messages"][0]["role"].as_str()), (Some("SYS"), Some("user")));
        assert_eq!(a["fallbacks"], "default");
        assert!(a.get("temperature").is_none() && a.get("thinking").is_none());
        assert!(corps(&cfg("anthropic", "claude-haiku-4-5"), "SYS", &msgs).get("fallbacks").is_none());
        let o = corps(&cfg("mistral", "mistral-large-latest"), "SYS", &msgs);
        assert_eq!(o["messages"][0]["role"], "system");
        assert!(o.get("system").is_none());
        let h = en_tetes(&cfg("anthropic", "claude-opus-5-5"), "CLE");
        assert!(h.contains(&"x-api-key: CLE".to_string()) && h.iter().any(|x| x.starts_with("anthropic-beta")));
        assert!(en_tetes(&cfg("mistral", "m"), "CLE").contains(&"Authorization: Bearer CLE".to_string()));
        assert_eq!(route(&cfg("anthropic", "x")), "https://api.anthropic.com/v1/messages");
    }

    #[test]
    fn flux_anthropic_et_openai() {
        let l = r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Bonjour"}}"#;
        assert_eq!(ligne_anthropic(l), vec![Evt::Texte("Bonjour".into())]);
        // La réflexion du modèle ne s'affiche pas comme une réponse.
        let l = r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":""}}"#;
        assert_eq!(ligne_anthropic(l), vec![Evt::Rien]);
        let l = r#"data: {"type":"message_delta","delta":{"stop_reason":"refusal"},"usage":{"output_tokens":12}}"#;
        assert_eq!(ligne_anthropic(l), vec![Evt::Usage { entree: None, sortie: Some(12) }, Evt::Arret("refusal".into())]);
        assert_eq!(ligne_anthropic("event: message_stop"), vec![Evt::Rien]);
        let l = r#"data: {"choices":[{"delta":{"content":"Oui"},"finish_reason":"stop"}]}"#;
        assert_eq!(ligne_openai(l), vec![Evt::Texte("Oui".into()), Evt::Arret("stop".into())]);
        assert!(message_erreur("401", r#"{"error":{"message":"invalid x-api-key"}}"#).contains("clé refusée"));
        assert_eq!(cite(r#"a"b\c"#), r#""a\"b\\c""#);
    }
}
