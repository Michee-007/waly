//! Client MCP (Model Context Protocol) — transport STDIO seulement.
//!
//! Parité plateforme (comparatif Hermes agent, 2026-09-10) : les outils de la
//! communauté se branchent par MCP au lieu d'être codés à la main dans le
//! catalogue. ADR : `docs/ADR-2026-09-10-client-mcp-stdio.md`.
//!
//! Choix gravés :
//! - **stdio uniquement** : un serveur MCP est un processus enfant parlant
//!   JSON-RPC 2.0 ligne à ligne sur stdin/stdout. Aucun port, aucune socket
//!   (pas de transport HTTP) — rien à ouvrir côté réseau.
//! - **Hors du sceau** : le processus serveur (node, python…) n'est PAS
//!   scellé par WFP (le sceau filtre par exe ; sceller `node.exe` couperait
//!   le réseau de toute la machine). D'où la règle suivante :
//! - **Approbation humaine par défaut** : chaque outil MCP est SENSIBLE
//!   (gate de risque R2, carte Approuver/Refuser) sauf si l'utilisateur
//!   déclare le serveur `confiance = "lecture"` dans waly.toml.
//! - **Budget prompt** : 8 outils max par serveur (`max_outils`), liste
//!   blanche `outils = "a,b"`, descriptions tronquées à 200 caractères.
//! - **Jamais bloquant** : un serveur absent, lent ou cassé est journalisé
//!   et ignoré — Waly démarre toujours.
//!
//! Pas de `set_read_timeout` (piège 7) : un thread lecteur pousse les lignes
//! dans un canal, l'attente est un `recv_timeout`.

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::llm::ToolSpec;
use crate::safety::Capabilities;
use crate::tools::{Registry, Tool};

/// Version du protocole annoncée ; le serveur répond la sienne (acceptée).
pub const VERSION_PROTOCOLE: &str = "2025-06-18";
const DELAI_INIT: Duration = Duration::from_secs(20);
const DELAI_APPEL: Duration = Duration::from_secs(60);
/// Plafond par défaut d'outils exposés par serveur (budget prompt : chaque
/// outil re-préfille ~86 tok + son schéma à chaque rebuild).
pub const MAX_OUTILS_DEFAUT: usize = 8;
const MAX_RESULTAT: usize = 15_000;
const MAX_DESCRIPTION: usize = 200;
const MAX_PAGES: usize = 5;

// ── Transport ────────────────────────────────────────────────────────────────

/// Un canal JSON-RPC vers un serveur MCP (stdio en vrai, simulé en test).
pub trait Transport {
    fn envoyer(&mut self, msg: &Value) -> Result<(), String>;
    /// Prochain message JSON reçu, au plus tard dans `delai`.
    fn recevoir(&mut self, delai: Duration) -> Result<Value, String>;
}

/// Serveur MCP en processus enfant (stdin/stdout, stderr jeté).
pub struct TransportStdio {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<String>,
}

impl TransportStdio {
    /// Lance le serveur. Windows : un `.cmd` (ex. `npx.cmd`) doit être nommé
    /// avec son extension — `Command` ne résout que les `.exe`.
    pub fn lancer(commande: &str, args: &[String]) -> Result<Self, String> {
        let mut cmd = Command::new(commande);
        cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|e| format!("lancement de {commande}: {e}"))?;
        let stdin = child.stdin.take().ok_or("stdin du serveur indisponible")?;
        let stdout = child.stdout.take().ok_or("stdout du serveur indisponible")?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for ligne in BufReader::new(stdout).lines() {
                match ligne {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self { child, stdin, rx })
    }
}

impl Transport for TransportStdio {
    fn envoyer(&mut self, msg: &Value) -> Result<(), String> {
        let mut ligne = serde_json::to_string(msg).map_err(|e| e.to_string())?;
        ligne.push('\n');
        self.stdin
            .write_all(ligne.as_bytes())
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("ecriture vers le serveur MCP: {e}"))
    }

    fn recevoir(&mut self, delai: Duration) -> Result<Value, String> {
        let fin = Instant::now() + delai;
        loop {
            let reste = fin.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(reste) {
                // Une ligne non-JSON sur stdout (bannière, log mal routé) est
                // ignorée : seul le JSON-RPC compte.
                Ok(l) => match serde_json::from_str::<Value>(l.trim()) {
                    Ok(v) if v.is_object() => return Ok(v),
                    _ => continue,
                },
                Err(RecvTimeoutError::Timeout) => return Err("delai depasse".into()),
                Err(RecvTimeoutError::Disconnected) => return Err("serveur MCP termine".into()),
            }
        }
    }
}

impl Drop for TransportStdio {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ── Client JSON-RPC ──────────────────────────────────────────────────────────

pub struct Client {
    t: Box<dyn Transport>,
    id: u64,
    /// Nom annoncé par le serveur (`serverInfo.name`), pour les journaux.
    pub serveur: String,
}

impl Client {
    /// Poignée de main MCP : `initialize` puis `notifications/initialized`.
    pub fn initialiser(t: Box<dyn Transport>) -> Result<Self, String> {
        let mut c = Self { t, id: 0, serveur: String::new() };
        let r = c.requete(
            "initialize",
            json!({
                "protocolVersion": VERSION_PROTOCOLE,
                "capabilities": {},
                "clientInfo": {"name": "waly", "version": env!("CARGO_PKG_VERSION")},
            }),
            DELAI_INIT,
        )?;
        c.serveur = r["serverInfo"]["name"].as_str().unwrap_or("?").to_string();
        c.t.envoyer(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;
        Ok(c)
    }

    fn requete(&mut self, methode: &str, params: Value, delai: Duration) -> Result<Value, String> {
        self.id += 1;
        let id = self.id;
        self.t.envoyer(&json!({"jsonrpc": "2.0", "id": id, "method": methode, "params": params}))?;
        let fin = Instant::now() + delai;
        loop {
            let v = self.t.recevoir(fin.saturating_duration_since(Instant::now()))?;
            match (v.get("method"), v.get("id")) {
                // Requête du serveur vers nous (ping, roots, sampling…) :
                // ping honoré, le reste poliment refusé (Waly n'expose rien).
                (Some(m), Some(rid)) => {
                    let rep = if m == "ping" {
                        json!({"jsonrpc": "2.0", "id": rid, "result": {}})
                    } else {
                        json!({"jsonrpc": "2.0", "id": rid,
                               "error": {"code": -32601, "message": "non supporte par Waly"}})
                    };
                    self.t.envoyer(&rep)?;
                }
                // Notification (logs, progression) : ignorée.
                (Some(_), None) => {}
                // Réponse : la nôtre ? (les autres ids sont périmés, ignorés)
                (None, Some(rid)) if rid.as_u64() == Some(id) => {
                    if let Some(e) = v.get("error") {
                        return Err(format!(
                            "erreur MCP {}: {}",
                            e["code"],
                            e["message"].as_str().unwrap_or("?")
                        ));
                    }
                    return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                }
                _ => {}
            }
        }
    }

    /// `tools/list`, pagination comprise (5 pages max).
    pub fn lister_outils(&mut self) -> Result<Vec<Value>, String> {
        let mut tous = Vec::new();
        let mut curseur: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let params = match &curseur {
                Some(c) => json!({"cursor": c}),
                None => json!({}),
            };
            let r = self.requete("tools/list", params, DELAI_INIT)?;
            if let Some(outils) = r["tools"].as_array() {
                tous.extend(outils.iter().cloned());
            }
            curseur = r["nextCursor"].as_str().map(String::from);
            if curseur.is_none() {
                break;
            }
        }
        Ok(tous)
    }

    /// `tools/call` → texte lisible par le modèle ; `isError` = `Err`.
    pub fn appeler(&mut self, nom: &str, args: &Value) -> Result<String, String> {
        let r = self.requete("tools/call", json!({"name": nom, "arguments": args}), DELAI_APPEL)?;
        let texte = texte_resultat(&r);
        if r["isError"].as_bool() == Some(true) {
            Err(texte)
        } else {
            Ok(texte)
        }
    }
}

/// Aplatit un `CallToolResult` en texte : parts texte, ressources textuelles,
/// liens ; les images sont signalées, pas transmises (budget, vision coupée
/// hors cerveau VLM). Borné à 15 000 caractères.
pub fn texte_resultat(r: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    for c in r["content"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        match c["type"].as_str() {
            Some("text") => parts.push(c["text"].as_str().unwrap_or("").to_string()),
            Some("image") => parts.push("[image non transmise]".into()),
            Some("audio") => parts.push("[audio non transmis]".into()),
            Some("resource") => {
                let res = &c["resource"];
                match res["text"].as_str() {
                    Some(t) => parts.push(t.to_string()),
                    None => parts.push(format!("[ressource {}]", res["uri"].as_str().unwrap_or("?"))),
                }
            }
            Some("resource_link") => {
                parts.push(format!("[lien {}]", c["uri"].as_str().unwrap_or("?")))
            }
            _ => {}
        }
    }
    let mut texte = parts.join("\n");
    if texte.trim().is_empty() {
        if let Some(s) = r.get("structuredContent") {
            texte = s.to_string();
        }
    }
    if texte.chars().count() > MAX_RESULTAT {
        texte = texte.chars().take(MAX_RESULTAT).collect::<String>() + "\n[… tronque]";
    }
    texte
}

/// Nom d'outil exposé au modèle : `mcp_<serveur>_<outil>`, [a-z0-9_], 64 max
/// (limite OpenAI des noms de fonctions).
pub fn nom_outil(serveur: &str, outil: &str) -> String {
    fn slug(s: &str) -> String {
        let mut out = String::new();
        for c in s.to_lowercase().chars() {
            if c.is_ascii_alphanumeric() {
                out.push(c);
            } else if !out.ends_with('_') {
                out.push('_');
            }
        }
        out.trim_matches('_').to_string()
    }
    let nom = format!("mcp_{}_{}", slug(serveur), slug(outil));
    nom.chars().take(64).collect()
}

/// Schéma d'entrée présentable au modèle : objet garanti, `$schema` retiré
/// (certains serveurs d'inférence locaux s'étouffent dessus).
fn schema_propre(s: &Value) -> Value {
    let mut s = if s.is_object() { s.clone() } else { json!({}) };
    if let Some(o) = s.as_object_mut() {
        o.remove("$schema");
        o.entry("type").or_insert(json!("object"));
        o.entry("properties").or_insert(json!({}));
    }
    s
}

// ── Configuration (waly.toml) ────────────────────────────────────────────────

/// Un serveur déclaré dans waly.toml :
/// ```toml
/// [mcp.demo]
/// commande = 'C:\Python314\python.exe'
/// arg1 = 'C:\waly\lab\mcp-banc\serveur_demo.py'
/// confiance = "lecture"      # défaut : "approbation"
/// outils = "compter_mots"    # liste blanche optionnelle
/// max_outils = 8
/// actif = true
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigServeur {
    pub nom: String,
    pub commande: String,
    pub args: Vec<String>,
    /// `confiance = "lecture"` : pas d'approbation par appel (choix explicite).
    pub lecture: bool,
    pub outils: Option<Vec<String>>,
    pub max_outils: usize,
}

/// Construit la config d'un serveur depuis ses clés (pure, testable).
/// `None` si désactivé (`actif = false`) ou sans `commande`.
pub fn config_depuis(nom: &str, get: impl Fn(&str) -> Option<String>) -> Option<ConfigServeur> {
    if get("actif").as_deref() == Some("false") {
        return None;
    }
    let commande = get("commande").filter(|c| !c.trim().is_empty())?;
    let mut args = Vec::new();
    for i in 1..=32 {
        match get(&format!("arg{i}")) {
            Some(a) => args.push(a),
            None => break,
        }
    }
    let outils = get("outils").map(|l| {
        l.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    });
    let max_outils = get("max_outils")
        .and_then(|m| m.parse::<usize>().ok())
        .filter(|m| *m > 0)
        .unwrap_or(MAX_OUTILS_DEFAUT);
    Some(ConfigServeur {
        nom: nom.to_string(),
        commande,
        args,
        lecture: get("confiance").as_deref() == Some("lecture"),
        outils,
        max_outils,
    })
}

/// Les serveurs déclarés sous `[mcp.<nom>]` dans waly.toml, puis ceux des
/// plugins ACTIFS (toujours sous approbation — plugins.rs).
pub fn serveurs_configures() -> Vec<ConfigServeur> {
    let mut v: Vec<ConfigServeur> = crate::config::sections_sous("mcp")
        .into_iter()
        .filter_map(|nom| {
            let section = format!("mcp.{nom}");
            config_depuis(&nom, |cle| crate::config::valeur(&section, cle))
        })
        .collect();
    v.extend(crate::plugins::serveurs());
    v
}

// ── Outils ───────────────────────────────────────────────────────────────────

/// Un outil distant, vu par le registre comme un outil natif : il traverse
/// les MÊMES murs (financiers, validation, gate de risque, anti-boucle).
pub struct OutilMcp {
    nom: String,
    distant: String,
    description: String,
    schema: Value,
    lecture: bool,
    client: Rc<RefCell<Client>>,
}

impl Tool for OutilMcp {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.nom.clone(),
            description: self.description.clone(),
            parameters: self.schema.clone(),
        }
    }

    fn capabilities(&self) -> Capabilities {
        if self.lecture {
            Capabilities::default()
        } else {
            // Processus tiers HORS du sceau : effet externe possible →
            // confirmation humaine avant chaque appel.
            Capabilities { irreversible: true, ..Capabilities::default() }
        }
    }

    fn run(&self, args: &Value) -> Result<String, String> {
        self.client.borrow_mut().appeler(&self.distant, args)
    }
}

/// Initialise un serveur sur `transport` et en tire ses outils (filtrés,
/// plafonnés). Retourne aussi le nombre d'outils écartés par le plafond.
pub fn brancher(
    cfg: &ConfigServeur,
    transport: Box<dyn Transport>,
) -> Result<(Vec<OutilMcp>, usize), String> {
    let mut client = Client::initialiser(transport)?;
    let decrits = client.lister_outils()?;
    let client = Rc::new(RefCell::new(client));
    let mut outils = Vec::new();
    let mut ecartes = 0;
    for d in decrits {
        let Some(distant) = d["name"].as_str() else { continue };
        if let Some(liste) = &cfg.outils {
            if !liste.iter().any(|n| n == distant) {
                continue;
            }
        }
        if outils.len() >= cfg.max_outils {
            ecartes += 1;
            continue;
        }
        let desc = d["description"].as_str().unwrap_or("").trim();
        let desc: String = desc.chars().take(MAX_DESCRIPTION).collect();
        outils.push(OutilMcp {
            nom: nom_outil(&cfg.nom, distant),
            distant: distant.to_string(),
            description: format!("[MCP {}] {desc}", cfg.nom),
            schema: schema_propre(&d["inputSchema"]),
            lecture: cfg.lecture,
            client: client.clone(),
        });
    }
    Ok((outils, ecartes))
}

/// Un serveur MCP branché dans CE processus — ce que le panneau « Vie
/// privée — la preuve » affiche : la promesse « rien ne sort » ne vaut que
/// hors de ces processus tiers (ADR 2026-09-10), il faut donc les montrer.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ServeurActif {
    pub nom: String,
    /// Exécutable lancé (nom de fichier seul : lisible, sans chemin privé).
    pub programme: String,
    pub outils: Vec<String>,
    /// `true` = confiance « lecture » (pas d'approbation par appel).
    pub lecture: bool,
}

static ACTIFS: std::sync::Mutex<Vec<ServeurActif>> = std::sync::Mutex::new(Vec::new());

/// Les serveurs MCP branchés dans ce processus (vide si aucun).
pub fn actifs() -> Vec<ServeurActif> {
    ACTIFS.lock().map(|v| v.clone()).unwrap_or_default()
}

fn noter_actif(s: ServeurActif) {
    if let Ok(mut v) = ACTIFS.lock() {
        v.retain(|x| x.nom != s.nom);
        v.push(s);
    }
}

/// Nom de fichier d'une commande (`C:\…\node.exe` → `node.exe`).
fn programme(commande: &str) -> String {
    commande.rsplit(['\\', '/']).next().unwrap_or(commande).to_string()
}

/// Branche tous les serveurs de waly.toml dans le registre. Ne panique ni ne
/// bloque jamais au-delà des délais : retourne des lignes de journal.
pub fn register_mcp_tools(registry: &mut Registry) -> Vec<String> {
    let mut journal = Vec::new();
    for cfg in serveurs_configures() {
        let transport = match TransportStdio::lancer(&cfg.commande, &cfg.args) {
            Ok(t) => t,
            Err(e) => {
                journal.push(format!("{} : {e}", cfg.nom));
                continue;
            }
        };
        match brancher(&cfg, Box::new(transport)) {
            Ok((outils, ecartes)) => {
                let mut noms = Vec::new();
                for o in outils {
                    if registry.connait(&o.nom) {
                        journal.push(format!("{} : {} deja pris, ignore", cfg.nom, o.nom));
                        continue;
                    }
                    noms.push(o.nom.clone());
                    registry.register(Box::new(o));
                }
                noter_actif(ServeurActif {
                    nom: cfg.nom.clone(),
                    programme: programme(&cfg.commande),
                    outils: noms.clone(),
                    lecture: cfg.lecture,
                });
                let mut ligne = format!(
                    "{} : {} outil(s) [{}] — {}, hors du sceau reseau",
                    cfg.nom,
                    noms.len(),
                    noms.join(", "),
                    if cfg.lecture { "confiance lecture" } else { "approbation a chaque appel" },
                );
                if ecartes > 0 {
                    ligne.push_str(&format!(" ; {ecartes} ecarte(s) par max_outils"));
                }
                journal.push(ligne);
            }
            Err(e) => journal.push(format!("{} : {e}", cfg.nom)),
        }
    }
    journal
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::Risk;
    use std::collections::VecDeque;

    /// Serveur MCP simulé : une fonction message → réponses, file d'attente.
    struct Faux {
        repondre: Box<dyn FnMut(&Value) -> Vec<Value>>,
        file: VecDeque<Value>,
        envoyes: Rc<RefCell<Vec<Value>>>,
    }

    impl Transport for Faux {
        fn envoyer(&mut self, msg: &Value) -> Result<(), String> {
            self.envoyes.borrow_mut().push(msg.clone());
            let reps = (self.repondre)(msg);
            self.file.extend(reps);
            Ok(())
        }
        fn recevoir(&mut self, _delai: Duration) -> Result<Value, String> {
            self.file.pop_front().ok_or_else(|| "delai depasse".into())
        }
    }

    fn outil(nom: &str) -> Value {
        json!({"name": nom, "description": format!("outil {nom}"),
               "inputSchema": {"$schema": "x", "type": "object",
                               "properties": {"texte": {"type": "string"}}}})
    }

    /// Serveur de démo : 2 pages d'outils, un ping serveur→client avant la
    /// réponse d'appel, un outil en erreur.
    fn serveur_demo(envoyes: Rc<RefCell<Vec<Value>>>) -> Box<dyn Transport> {
        let repondre = move |m: &Value| -> Vec<Value> {
            let id = m["id"].clone();
            match m["method"].as_str() {
                Some("initialize") => vec![json!({"jsonrpc": "2.0", "id": id, "result": {
                    "protocolVersion": "2025-06-18",
                    "serverInfo": {"name": "demo"}, "capabilities": {"tools": {}}}})],
                Some("tools/list") => {
                    let page2 = m["params"]["cursor"].as_str() == Some("p2");
                    let mut v = vec![json!({"jsonrpc": "2.0", "method": "notifications/message",
                                            "params": {"data": "log"}})];
                    v.push(if page2 {
                        json!({"jsonrpc": "2.0", "id": id, "result": {"tools": [outil("boom")]}})
                    } else {
                        json!({"jsonrpc": "2.0", "id": id,
                               "result": {"tools": [outil("echo")], "nextCursor": "p2"}})
                    });
                    v
                }
                Some("tools/call") => {
                    let ping = json!({"jsonrpc": "2.0", "id": 99, "method": "ping"});
                    let rep = if m["params"]["name"] == "boom" {
                        json!({"jsonrpc": "2.0", "id": id, "result": {"isError": true,
                               "content": [{"type": "text", "text": "ca a explose"}]}})
                    } else {
                        let t = m["params"]["arguments"]["texte"].as_str().unwrap_or("");
                        json!({"jsonrpc": "2.0", "id": id, "result": {
                               "content": [{"type": "text", "text": format!("echo: {t}")}]}})
                    };
                    vec![ping, rep]
                }
                _ => vec![], // notifications : pas de réponse
            }
        };
        Box::new(Faux { repondre: Box::new(repondre), file: VecDeque::new(), envoyes })
    }

    fn cfg(lecture: bool) -> ConfigServeur {
        ConfigServeur {
            nom: "demo".into(),
            commande: "x".into(),
            args: vec![],
            lecture,
            outils: None,
            max_outils: MAX_OUTILS_DEFAUT,
        }
    }

    #[test]
    fn poignee_de_main_pagination_appel_et_ping() {
        let envoyes = Rc::new(RefCell::new(Vec::new()));
        let (outils, ecartes) = brancher(&cfg(true), serveur_demo(envoyes.clone())).unwrap();
        assert_eq!(ecartes, 0);
        let noms: Vec<String> = outils.iter().map(|o| o.spec().name).collect();
        assert_eq!(noms, vec!["mcp_demo_echo", "mcp_demo_boom"]);
        // initialize → notifications/initialized dans cet ordre.
        assert_eq!(envoyes.borrow()[0]["method"], "initialize");
        assert_eq!(envoyes.borrow()[1]["method"], "notifications/initialized");
        // Schéma nettoyé, description préfixée.
        let spec = outils[0].spec();
        assert!(spec.parameters.get("$schema").is_none());
        assert!(spec.description.starts_with("[MCP demo] "));
        // Appel : le ping du serveur reçoit sa réponse, puis le résultat.
        assert_eq!(outils[0].run(&json!({"texte": "salut"})).unwrap(), "echo: salut");
        assert!(envoyes.borrow().iter().any(|m| m["id"] == 99 && m.get("result").is_some()));
        // isError → Err lisible.
        assert_eq!(outils[1].run(&json!({})).unwrap_err(), "ca a explose");
    }

    #[test]
    fn approbation_par_defaut_lecture_sur_declaration() {
        let (o, _) = brancher(&cfg(false), serveur_demo(Rc::default())).unwrap();
        assert_eq!(o[0].capabilities().risk(), Risk::Sensitive);
        let (o, _) = brancher(&cfg(true), serveur_demo(Rc::default())).unwrap();
        assert_eq!(o[0].capabilities().risk(), Risk::Read);
    }

    #[test]
    fn liste_blanche_et_plafond() {
        let mut c = cfg(true);
        c.outils = Some(vec!["boom".into()]);
        let (o, _) = brancher(&c, serveur_demo(Rc::default())).unwrap();
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].spec().name, "mcp_demo_boom");
        let mut c = cfg(true);
        c.max_outils = 1;
        let (o, ecartes) = brancher(&c, serveur_demo(Rc::default())).unwrap();
        assert_eq!((o.len(), ecartes), (1, 1));
    }

    #[test]
    fn serveur_muet_echoue_proprement() {
        let muet = Faux { repondre: Box::new(|_| vec![]), file: VecDeque::new(), envoyes: Rc::default() };
        let e = brancher(&cfg(true), Box::new(muet)).err().unwrap();
        assert!(e.contains("delai"));
    }

    #[test]
    fn erreur_json_rpc_remontee() {
        let rep = Faux {
            repondre: Box::new(|m| {
                vec![json!({"jsonrpc": "2.0", "id": m["id"],
                            "error": {"code": -32602, "message": "version inconnue"}})]
            }),
            file: VecDeque::new(),
            envoyes: Rc::default(),
        };
        let e = Client::initialiser(Box::new(rep)).err().unwrap();
        assert!(e.contains("-32602") && e.contains("version inconnue"));
    }

    #[test]
    fn serveurs_actifs_notes_sans_doublon_et_programme_court() {
        assert_eq!(programme(r"C:\Program Files\nodejs\node.exe"), "node.exe");
        assert_eq!(programme("python"), "python");
        let s = |outils: Vec<String>| ServeurActif {
            nom: "test-actifs".into(),
            programme: "node.exe".into(),
            outils,
            lecture: true,
        };
        noter_actif(s(vec!["a".into()]));
        noter_actif(s(vec!["b".into()]));
        let a: Vec<ServeurActif> = actifs().into_iter().filter(|x| x.nom == "test-actifs").collect();
        assert_eq!(a, vec![s(vec!["b".into()])]);
    }

    #[test]
    fn nom_outil_assaini_et_borne() {
        assert_eq!(nom_outil("Mon Serveur", "read-file.v2"), "mcp_mon_serveur_read_file_v2");
        assert!(nom_outil("s", &"x".repeat(100)).len() <= 64);
    }

    #[test]
    fn texte_resultat_mixte_et_tronque() {
        let r = json!({"content": [
            {"type": "text", "text": "a"},
            {"type": "image", "data": "...", "mimeType": "image/png"},
            {"type": "resource", "resource": {"uri": "file:///x", "text": "contenu"}},
            {"type": "resource_link", "uri": "file:///y"}
        ]});
        assert_eq!(texte_resultat(&r), "a\n[image non transmise]\ncontenu\n[lien file:///y]");
        assert_eq!(texte_resultat(&json!({"content": [], "structuredContent": {"n": 3}})), "{\"n\":3}");
        let long = json!({"content": [{"type": "text", "text": "é".repeat(20_000)}]});
        assert!(texte_resultat(&long).ends_with("[… tronque]"));
    }

    #[test]
    fn config_depuis_cles() {
        let cles = |k: &str| -> Option<String> {
            match k {
                "commande" => Some("python".into()),
                "arg1" => Some("s.py".into()),
                "arg2" => Some("--x".into()),
                "arg4" => Some("orphelin".into()), // arg3 absent : s'arrête à 2
                "outils" => Some("a, b ,".into()),
                "max_outils" => Some("3".into()),
                _ => None,
            }
        };
        let c = config_depuis("demo", cles).unwrap();
        assert_eq!(c.args, vec!["s.py", "--x"]);
        assert_eq!(c.outils, Some(vec!["a".to_string(), "b".to_string()]));
        assert_eq!(c.max_outils, 3);
        assert!(!c.lecture);
        assert!(config_depuis("x", |k| (k == "commande").then(|| "p".into())).is_some());
        assert!(config_depuis("x", |_| None).is_none());
        assert!(config_depuis("x", |k| match k {
            "commande" => Some("p".into()),
            "actif" => Some("false".into()),
            _ => None,
        })
        .is_none());
    }

    #[test]
    fn outil_mcp_passe_par_le_gate_du_registre() {
        let mut reg = Registry::new();
        let (o, _) = brancher(&cfg(false), serveur_demo(Rc::default())).unwrap();
        for t in o {
            reg.register(Box::new(t));
        }
        let mut guard = crate::safety::LoopGuard::new();
        let mut ctx = crate::tools::TurnCtx { user_message: "", loop_guard: &mut guard, approvals: None };
        let call = crate::llm::ToolCall {
            id: "1".into(),
            name: "mcp_demo_echo".into(),
            arguments: "{\"texte\":\"x\"}".into(),
        };
        // Sensible sans base d'approbations : refus fail-safe, jamais exécuté.
        assert!(matches!(reg.dispatch(&call, &mut ctx), crate::tools::Dispatched::Blocked(_)));
    }
}
