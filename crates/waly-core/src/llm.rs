//! Client LLM OpenAI-compat avec tool-calling natif (FastFlowLM, localhost).
//!
//! Même philosophie que waly-voice/llm.rs : pur `std::net`, jamais de
//! `set_read_timeout` (piège SO_RCVTIMEO Windows, CLAUDE.md n°7), socket non
//! bloquant + sommeil. Un tour à outil est NON-streaming (on a besoin du tool
//! call complet avant d'agir) ; la réponse finale pourra être streamée par
//! l'appelant voix via waly-voice.
//!
//! Protocole validé au banc le 2026-07-05 sur FLM v0.9.43 + qwen3-it:4b :
//! `tools` → `finish_reason:"tool_calls"` + `message.tool_calls[]`, puis
//! réinjection `role:"assistant"(tool_calls)` + `role:"tool"` → réponse.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Un message de la conversation, sérialisable au format OpenAI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Msg {
    System(String),
    User(String),
    Assistant(String),
    /// Le modèle a demandé des appels d'outils (contenu textuel absent).
    AssistantToolCalls(Vec<ToolCall>),
    /// Résultat d'un outil, renvoyé au modèle.
    ToolResult { call_id: String, content: String },
    /// Message utilisateur portant une IMAGE (moment vision R4) : texte +
    /// data-URL base64 en content-parts OpenAI (forme validée au banc,
    /// GATE A du plan R4). Injecté par l'outil `regarder` — le VLM la voit
    /// au tour suivant.
    UserImage { texte: String, data_url: String },
}

/// Modèle par défaut du cerveau local. R4 (2026-07-07, décision gravée) :
/// cerveau UNIQUE texte+vision `qwen3vl-it:4b` — TTFT et décodage mesurés
/// identiques à qwen3-it:4b, tool-calling natif OK (engines/README § VLM).
/// Surcharge : env `WALY_MODEL` › waly.toml `[llm] modele` (config.rs).
pub fn modele_par_defaut() -> String {
    MODELE_CHOISI
        .lock()
        .ok()
        .and_then(|m| m.clone())
        .or_else(|| std::env::var("WALY_MODEL").ok())
        .or_else(|| crate::config::valeur("llm", "modele"))
        .unwrap_or_else(|| "qwen3vl-it:4b".into())
}

/// Cerveau local choisi DANS l'app (menu des modèles, lot 3) : prime sur
/// l'environnement et waly.toml pour ce processus. `None` = revenir au défaut.
static MODELE_CHOISI: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn choisir_modele(nom: Option<String>) {
    if let Ok(mut m) = MODELE_CHOISI.lock() {
        *m = nom.filter(|n| !n.trim().is_empty());
    }
}

/// Moteur A : FLM (NPU) ou tout serveur de l'utilisateur. HORS de la zone
/// dynamique Windows (49152-65535), où WinNAT/Hyper-V réserve des plages au
/// démarrage de WSL — vécu 2026-09-10 : 52579-52678 réservé, l'ancien port
/// 52626 devenait inattachable (bind 10013).
pub const PORT_MOTEUR_A: u16 = 42626;
/// Moteur B : Ollama standard — démarré par son app à chaque session, signé
/// (passe SAC), survit aux redémarrages.
pub const PORT_MOTEUR_B: u16 = 11434;

/// Port du serveur LLM local (OpenAI-compat), choisi à la construction du
/// client : `WALY_LLM_PORT` si posé (choix explicite, jamais contredit) ›
/// waly.toml `[llm] port` › moteur A s'il écoute › moteur B s'il écoute ›
/// moteur A (l'erreur du client pointe alors vers lui).
pub fn port_par_defaut() -> u16 {
    choisir_port(
        std::env::var("WALY_LLM_PORT").ok().as_deref(),
        crate::config::valeur("llm", "port").as_deref(),
        ecoute_local,
    )
}

fn choisir_port(env: Option<&str>, fichier: Option<&str>, ecoute: impl Fn(u16) -> bool) -> u16 {
    if let Some(p) = env.and_then(|p| p.trim().parse::<u16>().ok()) {
        return p;
    }
    if let Some(p) = fichier.and_then(|p| p.trim().parse::<u16>().ok()) {
        return p;
    }
    if ecoute(PORT_MOTEUR_A) {
        return PORT_MOTEUR_A;
    }
    if ecoute(PORT_MOTEUR_B) {
        return PORT_MOTEUR_B;
    }
    PORT_MOTEUR_A
}

/// Sonde TCP sur le loopback : connexion ouverte puis refermée aussitôt —
/// aucun octet échangé, aucun timeout de lecture (piège 7). Un port fermé
/// est refusé instantanément ; 300 ms de plafond sinon.
pub fn ecoute_local(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(300)).is_ok()
}

#[cfg(test)]
mod tests_port {
    use super::*;

    #[test]
    fn env_prime_toujours() {
        assert_eq!(choisir_port(Some("11500"), None, |_| true), 11500);
        assert_eq!(choisir_port(Some(" 11500 "), Some("12000"), |_| false), 11500);
    }

    #[test]
    fn fichier_prime_sur_la_sonde_mais_pas_sur_env() {
        assert_eq!(choisir_port(None, Some("12000"), |_| true), 12000);
        assert_eq!(choisir_port(Some("11500"), Some("12000"), |_| false), 11500);
    }

    #[test]
    fn moteur_a_prioritaire_s_il_ecoute() {
        assert_eq!(choisir_port(None, None, |_| true), PORT_MOTEUR_A);
    }

    #[test]
    fn repli_sur_ollama_si_a_muet() {
        assert_eq!(choisir_port(None, None, |p| p == PORT_MOTEUR_B), PORT_MOTEUR_B);
    }

    #[test]
    fn rien_n_ecoute_reste_sur_a() {
        assert_eq!(choisir_port(None, None, |_| false), PORT_MOTEUR_A);
    }

    #[test]
    fn capacites_lues_et_sonde_limitee_au_moteur_b() {
        let show = r#"{"license":"...","capabilities":["completion","vision"],"details":{}}"#;
        assert_eq!(
            capacites_depuis_show(show),
            Some(vec!["completion".to_string(), "vision".to_string()])
        );
        assert_eq!(capacites_depuis_show(r#"{"details":{}}"#), None);
        assert_eq!(
            noms_depuis_tags(r#"{"models":[{"name":"gemma3:4b"},{"name":"qwen3:4b"}]}"#),
            vec!["gemma3:4b".to_string(), "qwen3:4b".to_string()]
        );
        assert!(noms_depuis_tags("{}").is_empty());
        assert_eq!(capacites_depuis_show("pas du json"), None);
        // Hors moteur B : aucune sonde (FLM n'a pas /api/show).
        let c = LlmClient::new("127.0.0.1", 1, "m");
        assert_eq!(c.capacites(), None);
        assert!(c.modeles_qui_voient().is_empty());
    }

    #[test]
    fn garder_chaud_no_op_hors_moteur_b() {
        // Port 1 : rien n'y écoute — un no-op ne doit même pas s'y connecter.
        let c = LlmClient::new("127.0.0.1", 1, "m");
        assert!(c.garder_chaud().is_ok());
    }

    #[test]
    fn env_invalide_ignore() {
        assert_eq!(choisir_port(Some("abc"), Some("xyz"), |p| p == PORT_MOTEUR_B), PORT_MOTEUR_B);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Arguments JSON BRUTS tels qu'émis par le modèle — la validation
    /// appartient au dispatch (sûreté), pas au client réseau.
    pub arguments: String,
}

/// Déclaration d'outil envoyée au modèle (schéma JSON des paramètres).
#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Issue d'un tour LLM : soit du texte final, soit des appels d'outils.
#[derive(Debug, Clone, PartialEq)]
pub enum Turn {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}

pub fn msg_to_json(m: &Msg) -> serde_json::Value {
    match m {
        Msg::System(c) => serde_json::json!({"role": "system", "content": c}),
        Msg::User(c) => serde_json::json!({"role": "user", "content": c}),
        Msg::Assistant(c) => serde_json::json!({"role": "assistant", "content": c}),
        Msg::AssistantToolCalls(calls) => serde_json::json!({
            "role": "assistant",
            "tool_calls": calls.iter().map(|c| serde_json::json!({
                "id": c.id,
                "type": "function",
                "function": {"name": c.name, "arguments": c.arguments},
            })).collect::<Vec<_>>(),
        }),
        Msg::ToolResult { call_id, content } => serde_json::json!({
            "role": "tool", "tool_call_id": call_id, "content": content,
        }),
        Msg::UserImage { texte, data_url } => serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": texte},
                {"type": "image_url", "image_url": {"url": data_url}},
            ],
        }),
    }
}

pub fn tool_to_json(t: &ToolSpec) -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": t.name,
            "description": t.description,
            "parameters": t.parameters,
        },
    })
}

/// Noms des modèles depuis la réponse de `/api/tags` d'Ollama.
pub fn noms_depuis_tags(json: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| {
            v["models"]
                .as_array()
                .map(|a| a.iter().filter_map(|m| m["name"].as_str().map(String::from)).collect())
        })
        .unwrap_or_default()
}

/// Capacités d'un modèle depuis la réponse de `/api/show` d'Ollama
/// (`"capabilities": ["completion", "vision", "tools", …]`).
pub fn capacites_depuis_show(json: &str) -> Option<Vec<String>> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    Some(
        v["capabilities"]
            .as_array()?
            .iter()
            .filter_map(|c| c.as_str().map(String::from))
            .collect(),
    )
}

/// Extrait le tour depuis une réponse non-streaming `/v1/chat/completions`.
pub fn parse_completion(body: &str) -> Result<Turn, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("reponse LLM illisible: {e}"))?;
    let msg = &v["choices"][0]["message"];
    if let Some(calls) = msg["tool_calls"].as_array() {
        let calls: Vec<ToolCall> = calls
            .iter()
            .filter_map(|c| {
                Some(ToolCall {
                    id: c["id"].as_str()?.to_owned(),
                    name: c["function"]["name"].as_str()?.to_owned(),
                    arguments: c["function"]["arguments"].as_str().unwrap_or("{}").to_owned(),
                })
            })
            .collect();
        if !calls.is_empty() {
            return Ok(Turn::ToolCalls(calls));
        }
    }
    match msg["content"].as_str() {
        Some(text) => Ok(Turn::Text(text.to_owned())),
        None => Err(format!("reponse LLM sans contenu ni tool_calls: {}", truncate(body, 200))),
    }
}

fn truncate(s: &str, max: usize) -> &str {
    let end = (0..=max.min(s.len())).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
    &s[..end]
}

/// Issue d'un tour STREAMING.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamTurn {
    /// Texte final (déjà livré delta par delta à `on_delta`).
    /// `complete=false` : EOF serveur AVANT `[DONE]` — la réponse est
    /// peut-être tronquée, le signaler (leçon waly-voice ChatEnd).
    Text { text: String, complete: bool },
    /// Le modèle demande des outils (FLM les émet en UN delta complet —
    /// mesuré 2026-07-05 — mais on tolère la fragmentation OpenAI).
    /// `text` = ce qui a été streamé AVANT l'appel (déjà prononcé).
    ToolCalls { text: String, calls: Vec<ToolCall> },
    /// Interrompu par l'appelant (barge-in) ; texte partiel déjà émis.
    Aborted(String),
}

pub struct LlmClient {
    pub host: String,
    pub port: u16,
    pub model: String,
    pub max_tokens: u32,
    /// Réflexion approfondie : le raisonnement NATIF d'un modèle (champ
    /// `reasoning` du flux) est transmis à `on_delta`, emballé dans
    /// `<think>…</think>` — l'appelant le sépare (reflexion.rs). `false`
    /// (défaut, la voix) : il est ignoré, jamais prononcé.
    pub reflexion: bool,
}

impl LlmClient {
    pub fn new(host: &str, port: u16, model: &str) -> Self {
        Self { host: host.into(), port, model: model.into(), max_tokens: 512, reflexion: false }
    }

    /// Un tour non-streaming avec outils. Bloquant, timeout 120 s.
    pub fn chat(&self, messages: &[Msg], tools: &[ToolSpec]) -> Result<Turn, String> {
        self.chat_with_usage(messages, tools).map(|(t, _)| t)
    }

    /// Comme [`Self::chat`], en exposant `usage.prompt_tokens` (mesure du
    /// budget prompt — le tokenizer de FLM fait foi).
    pub fn chat_with_usage(
        &self,
        messages: &[Msg],
        tools: &[ToolSpec],
    ) -> Result<(Turn, Option<u64>), String> {
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": messages.iter().map(msg_to_json).collect::<Vec<_>>(),
            "stream": false,
            "max_tokens": self.max_tokens,
        });
        echantillonnage(&mut body, !tools.is_empty(), None);
        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(tools.iter().map(tool_to_json).collect());
        }
        let raw = self.post_json(&body.to_string())?;
        let usage = serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|v| v["usage"]["prompt_tokens"].as_u64());
        Ok((parse_completion(&raw)?, usage))
    }

    /// Un tour STREAMING avec outils, pour la voix : chaque fragment de
    /// TEXTE passe par `on_delta` (retourner `false` interrompt — barge-in) ;
    /// pendant les attentes réseau, `on_delta("")` sonde l'interruption
    /// toutes les ~30 ms (le TTFT dure 1-2 s). Un éventuel tool call termine
    /// le flux en [`StreamTurn::ToolCalls`]. Jamais de SO_RCVTIMEO (piège 7).
    pub fn chat_stream(
        &self,
        messages: &[Msg],
        tools: &[ToolSpec],
        mut on_delta: impl FnMut(&str) -> bool,
    ) -> Result<StreamTurn, String> {
        match self.chat_stream_avec(messages, tools, &mut on_delta, None) {
            // Vécu 2026-10-01 : le 4B enchaîne parfois des appels d'outils en
            // boucle DANS une réponse ; coupé à max_tokens en plein appel, Ollama
            // ne sait plus le lire -> 500. Une relance à température 0.
            Err(e) if !tools.is_empty() && e.contains(" 500 ") => {
                eprintln!("Waly: 500 sur un tour d'outils, relance a temperature 0");
                self.chat_stream_avec(messages, tools, &mut on_delta, Some(0.0))
            }
            r => r,
        }
    }

    fn chat_stream_avec(
        &self,
        messages: &[Msg],
        tools: &[ToolSpec],
        on_delta: &mut impl FnMut(&str) -> bool,
        temperature: Option<f64>,
    ) -> Result<StreamTurn, String> {
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": messages.iter().map(msg_to_json).collect::<Vec<_>>(),
            "stream": true,
            "max_tokens": self.max_tokens,
            // Paramètres › Utilisation : le dernier chunk porte `usage`
            // (Ollama, llama.cpp, LM Studio ; ignoré sinon — on estime alors).
            "stream_options": { "include_usage": true },
        });
        echantillonnage(&mut body, !tools.is_empty(), temperature);
        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(tools.iter().map(tool_to_json).collect());
        }
        // Diagnostic opt-in : la requête EXACTE envoyée (local, écrasée).
        if std::env::var("WALY_DEBUG_PROMPT").is_ok() {
            let _ = std::fs::write(std::env::temp_dir().join("waly-requete.json"), body.to_string());
        }
        let mut stream = self.connect(&body.to_string())?;

        // Délai d'INACTIVITÉ (repoussé à chaque octet reçu) : une réponse
        // longue qui coule — réflexion approfondie, style détaillé — dépasse
        // 120 s au total sur cette machine sans être en panne.
        let mut deadline = std::time::Instant::now() + Duration::from_secs(120);
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 8192];
        // Un bloc de raisonnement natif est ouvert côté appelant.
        let mut raisonne = false;
        let mut status_seen = false;
        let mut headers_done = false;
        let mut text = String::new();
        let mut calls: Vec<ToolCall> = Vec::new();
        let mut call_index_map = std::collections::HashMap::new();
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break, // EOF sans [DONE] : on rend ce qu'on a
                Ok(n) => {
                    deadline = std::time::Instant::now() + Duration::from_secs(120);
                    buf.extend_from_slice(&chunk[..n]);
                    while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                        let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
                        let line = String::from_utf8_lossy(&line_bytes);
                        if !status_seen {
                            if !line.contains("200") {
                                return Err(format!("statut LLM inattendu: {}", line.trim()));
                            }
                            status_seen = true;
                        } else if !headers_done {
                            if line.trim().is_empty() {
                                headers_done = true;
                            }
                        } else {
                            if let Some((p, c)) = usage_sse(&line) {
                                usage_ajouter(p, c);
                            }
                            match parse_sse_line(&line) {
                                SseEvent::Delta(t) => {
                                    if std::mem::take(&mut raisonne) {
                                        on_delta("</think>");
                                    }
                                    text.push_str(&t);
                                    if !on_delta(&t) {
                                        abandon_stream(stream);
                                        return Ok(StreamTurn::Aborted(text));
                                    }
                                }
                                // Hors `text` : le raisonnement n'entre ni dans
                                // l'historique ni dans ce qui est persisté.
                                SseEvent::Reasoning(t) => {
                                    if self.reflexion {
                                        let ouvre = if raisonne { "" } else { "<think>" };
                                        raisonne = true;
                                        if !on_delta(&format!("{ouvre}{t}")) {
                                            abandon_stream(stream);
                                            return Ok(StreamTurn::Aborted(text));
                                        }
                                    }
                                }
                                SseEvent::ToolCallDelta(fragments) => {
                                    merge_tool_call_deltas(
                                        &mut calls,
                                        &mut call_index_map,
                                        fragments,
                                    );
                                }
                                SseEvent::Done => {
                                    if raisonne {
                                        on_delta("</think>");
                                    }
                                    return Ok(if calls.is_empty() {
                                        StreamTurn::Text { text, complete: true }
                                    } else {
                                        StreamTurn::ToolCalls { text, calls }
                                    });
                                }
                                SseEvent::Ignore => {}
                            }
                        }
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    if !on_delta("") {
                        abandon_stream(stream);
                        return Ok(StreamTurn::Aborted(text));
                    }
                    if std::time::Instant::now() > deadline {
                        return Err("timeout LLM (120 s)".into());
                    }
                    std::thread::sleep(Duration::from_millis(30));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        // EOF sans [DONE] : réponse potentiellement tronquée.
        if raisonne {
            on_delta("</think>");
        }
        Ok(if calls.is_empty() {
            StreamTurn::Text { text, complete: false }
        } else {
            StreamTurn::ToolCalls { text, calls }
        })
    }

    /// Requête d'échauffement : non-streaming, 1 token, lue JUSQU'AU BOUT
    /// puis fermée proprement (la première requête après chargement coûte
    /// 5-10 s). Ne JAMAIS chauffer en avortant un stream (piège 7).
    pub fn warmup(&self) -> Result<(), String> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": "ok"}],
            "stream": false,
            "max_tokens": 1,
        })
        .to_string();
        let mut stream = self.connect(&body)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        let mut chunk = [0u8; 8192];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => return Ok(()),
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    if std::time::Instant::now() > deadline {
                        return Err("timeout du warmup LLM (60 s)".into());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    /// Préchauffe le CACHE DE PRÉFIXE : `[système + outils]` lu une fois, un
    /// seul token décodé. Mesuré 2026-09-10 (Ollama iGPU, 23 outils) : 1ᵉʳ
    /// tour d'une session 23,1 s, tours suivants 3,4-3,6 s — le 1ᵉʳ tour
    /// payait le préfill du prompt entier. Préchauffé, il ne paie plus que le
    /// message. Même discipline que le warmup : non-streaming, lu jusqu'au
    /// bout (piège 7). Le préfixe doit être IDENTIQUE au vrai tour (même
    /// système, mêmes outils, même ordre) — append-only R4.5.
    pub fn prechauffer_prefixe(&self, systeme: &Msg, tools: &[ToolSpec]) -> Result<(), String> {
        let mut c = LlmClient::new(&self.host, self.port, &self.model);
        c.max_tokens = 1;
        c.chat(&[systeme.clone(), Msg::User("ok".into())], tools).map(|_| ())
    }

    /// Garde le modèle CHARGÉ sur Ollama (moteur B) : `POST /api/generate`
    /// sans prompt + `keep_alive` = charge si besoin, renouvelle l'expiration,
    /// AUCUN calcul. Mesuré 2026-09-10 : Ollama ignore `keep_alive` sur /v1 et
    /// décharge après 5 min → premier mot 4,7 s (texte) / 19,6 s (vision) à
    /// froid contre 0,1-0,4 s à chaud. No-op hors moteur B : FLM tient son
    /// modèle en permanence et n'a pas cette route (jamais d'envoi inconnu à
    /// FLM — piège 5). Réponse lue jusqu'au bout (piège 7).
    pub fn garder_chaud(&self) -> Result<(), String> {
        if self.port != PORT_MOTEUR_B {
            return Ok(());
        }
        let body = serde_json::json!({"model": self.model, "keep_alive": "10m"}).to_string();
        let mut stream = self.connect_to("/api/generate", &body)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        let mut raw: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => raw.extend_from_slice(&chunk[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    if std::time::Instant::now() > deadline {
                        return Err("timeout du prechargement (120 s)".into());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        let text = String::from_utf8_lossy(&raw);
        let statut = text.lines().next().unwrap_or("");
        if statut.contains("200") {
            Ok(())
        } else {
            Err(format!("prechargement refuse: {}", truncate(statut, 120)))
        }
    }

    fn connect(&self, body: &str) -> Result<TcpStream, String> {
        self.connect_to("/v1/chat/completions", body)
    }

    fn connect_to(&self, path: &str, body: &str) -> Result<TcpStream, String> {
        self.ouvrir("POST", path, body)
    }

    /// Capacités déclarées du modèle (`completion`, `tools`, `vision`…) via
    /// l'API native d'Ollama (`/api/show` — ne charge PAS le modèle). `None`
    /// hors moteur B (FLM n'a pas cette route : jamais d'envoi inconnu) ou si
    /// la réponse est illisible — l'appelant garde le comportement historique.
    pub fn capacites(&self) -> Option<Vec<String>> {
        if self.port != PORT_MOTEUR_B {
            return None;
        }
        let body = serde_json::json!({"model": self.model}).to_string();
        let raw = self.echange("POST", "/api/show", &body).ok()?;
        capacites_depuis_show(&raw)
    }

    /// Taille du modèle en milliards de paramètres (sélection d'outils) :
    /// `details.parameter_size` d'Ollama, sinon déduite du nom (FLM).
    pub fn taille_milliards(&self) -> Option<f32> {
        if self.port == PORT_MOTEUR_B {
            let body = serde_json::json!({"model": self.model}).to_string();
            if let Ok(raw) = self.echange("POST", "/api/show", &body) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if let Some(t) = v["details"]["parameter_size"].as_str() {
                        if let Some(b) = crate::selection::milliards_depuis_taille(t) {
                            return Some(b);
                        }
                    }
                }
            }
        }
        crate::selection::milliards_depuis_nom(&self.model)
    }

    /// Modèles installés sur Ollama (`/api/tags`) — moteur B seulement.
    pub fn modeles_installes(&self) -> Vec<String> {
        if self.port != PORT_MOTEUR_B {
            return Vec::new();
        }
        match self.echange("GET", "/api/tags", "") {
            Ok(raw) => noms_depuis_tags(&raw),
            Err(_) => Vec::new(),
        }
    }

    /// Modèles installés sur Ollama qui déclarent la vision — la suggestion
    /// faite à l'utilisateur quand son cerveau ne voit pas (20 examinés max).
    pub fn modeles_qui_voient(&self) -> Vec<String> {
        self.modeles_installes()
            .into_iter()
            .take(20)
            .filter(|n| {
                LlmClient::new(&self.host, self.port, n)
                    .capacites()
                    .is_some_and(|c| c.iter().any(|x| x == "vision"))
            })
            .map(String::from)
            .collect()
    }

    /// Modèles installés sur Ollama avec taille et capacités — moteur B.
    pub fn modeles_detail(&self) -> Vec<crate::modeles::Installe> {
        if self.port != PORT_MOTEUR_B {
            return Vec::new();
        }
        match self.echange("GET", "/api/tags", "") {
            Ok(raw) => crate::modeles::installes_depuis_tags(&raw),
            Err(_) => Vec::new(),
        }
    }

    /// Flux NDJSON d'une route native du moteur (`/api/pull`) : chaque ligne
    /// JSON passe par `on_line` ; pendant les attentes, `on_line("")` sonde
    /// l'annulation. Retourner `false` arrête (le socket est fermé : c'est le
    /// geste d'annulation documenté d'Ollama). `Ok(true)` = flux lu jusqu'au
    /// bout. Délai d'INACTIVITÉ, jamais SO_RCVTIMEO (piège 7).
    pub fn flux_lignes(
        &self,
        methode: &str,
        path: &str,
        body: &str,
        inactivite_s: u64,
        mut on_line: impl FnMut(&str) -> bool,
    ) -> Result<bool, String> {
        let mut stream = self.ouvrir(methode, path, body)?;
        let delai = Duration::from_secs(inactivite_s);
        let mut deadline = std::time::Instant::now() + delai;
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 8192];
        let mut statut: Option<bool> = None;
        let mut headers_done = false;
        let mut refus = String::new();
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    deadline = std::time::Instant::now() + delai;
                    buf.extend_from_slice(&chunk[..n]);
                    while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                        let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
                        let line = String::from_utf8_lossy(&line_bytes);
                        let line = line.trim();
                        if statut.is_none() {
                            statut = Some(line.contains("200"));
                        } else if !headers_done {
                            headers_done = line.is_empty();
                        } else if statut == Some(false) {
                            refus.push_str(line);
                        } else if line.starts_with('{') && !on_line(line) {
                            return Ok(false);
                        }
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    if !on_line("") {
                        return Ok(false);
                    }
                    if std::time::Instant::now() > deadline {
                        return Err(format!("le moteur ne repond plus ({inactivite_s} s sans rien recevoir)"));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        if statut == Some(false) {
            let e = serde_json::from_str::<serde_json::Value>(&refus)
                .ok()
                .and_then(|v| v["error"].as_str().map(String::from))
                .unwrap_or_else(|| truncate(&refus, 200).to_string());
            return Err(format!("refus du moteur : {e}"));
        }
        Ok(true)
    }

    fn ouvrir(&self, methode: &str, path: &str, body: &str) -> Result<TcpStream, String> {
        let mut stream = TcpStream::connect((self.host.as_str(), self.port)).map_err(|e| {
            format!(
                "LLM injoignable sur {}:{} ({e}) — aucun moteur local : ouvrir Ollama \
                 (moteur B, port {PORT_MOTEUR_B}) ou lancer engines/start-flm.ps1 \
                 (moteur A, port {PORT_MOTEUR_A})",
                self.host, self.port
            )
        })?;
        stream.set_nodelay(true).map_err(|e| e.to_string())?;
        write!(
            stream,
            "{methode} {path} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.host, self.port, body.len(), body
        )
        .map_err(|e| e.to_string())?;
        stream.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(stream)
    }

    /// POST bloquant, lecture jusqu'à fermeture serveur (Connection: close).
    fn post_json(&self, body: &str) -> Result<String, String> {
        self.echange("POST", "/v1/chat/completions", body)
    }

    /// Requête HTTP bloquante (méthode, chemin) lue jusqu'à fermeture ; corps
    /// dé-chunké, statut non-200 = erreur. Jamais SO_RCVTIMEO (piège 7).
    fn echange(&self, methode: &str, path: &str, body: &str) -> Result<String, String> {
        let mut stream = self.ouvrir(methode, path, body)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        let mut raw: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => raw.extend_from_slice(&chunk[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    if std::time::Instant::now() > deadline {
                        return Err("timeout LLM (120 s)".into());
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        let text = String::from_utf8_lossy(&raw);
        let Some(header_end) = text.find("\r\n\r\n") else {
            return Err("reponse HTTP sans en-tetes".into());
        };
        if !text[..header_end].contains("200") {
            return Err(format!("statut LLM inattendu: {}", truncate(&text, 200)));
        }
        Ok(dechunk(&text[header_end + 4..]))
    }
}

/// Abandon PROPRE d'un flux en cours (barge-in) : on ne claque pas le socket
/// au nez de FLM pendant qu'il écrit — les RST l'ont déjà tué deux fois
/// (piège 7). Un thread détaché draine jusqu'à EOF (ou 10 s) puis ferme.
fn abandon_stream(stream: TcpStream) {
    std::thread::spawn(move || {
        let mut s = stream;
        let mut sink = [0u8; 8192];
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            match s.read(&mut sink) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    if std::time::Instant::now() > deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => break,
            }
        }
    });
}

/// Échantillonnage EXPLICITE (vécu 2026-10-01) : sans ces champs, l'API
/// compatible OpenAI d'Ollama impose temperature = top_p = 1,0 et ÉCRASE le
/// Modelfile (0,7 / 0,8 pour Qwen) — le 4B partait en boucles d'appels
/// d'outils. Mesuré sur 5 essais d'un même tour « rappelle-moi » : défaut =
/// boucles jusqu'à 512 tokens puis 500 ; 0,7 = 2/5 boucles ; 0,2 = 0 (un
/// appel, au pire 3). D'où : tour d'outils = 0,2 (précision), tour de
/// texte = 0,7 (naturel), pénalité de présence 1,5 (fiche Qwen3).
pub fn echantillonnage(body: &mut serde_json::Value, outils: bool, temperature: Option<f64>) {
    let t = temperature.unwrap_or(if outils { 0.2 } else { 0.7 });
    body["temperature"] = serde_json::json!(t);
    body["top_p"] = serde_json::json!(0.8);
    body["presence_penalty"] = serde_json::json!(1.5);
}

/// `usage` d'une ligne SSE (`include_usage`) : (tokens d'entrée, de sortie).
pub fn usage_sse(line: &str) -> Option<(u64, u64)> {
    if !line.contains("\"usage\"") {
        return None;
    }
    let data = line.trim().strip_prefix("data:")?.trim();
    let v: serde_json::Value = serde_json::from_str(data).ok()?;
    let u = &v["usage"];
    Some((u["prompt_tokens"].as_u64()?, u["completion_tokens"].as_u64().unwrap_or(0)))
}

/// Cumul d'`usage` des streams du processus depuis [`usage_reinit`] : un tour
/// = plusieurs rounds (outils), chacun re-paie son entrée — c'est le vrai
/// coût du tour. Un processus joue ses tours en série (worker unique).
static USAGE: std::sync::Mutex<Option<(u64, u64)>> = std::sync::Mutex::new(None);

fn usage_ajouter(p: u64, c: u64) {
    if let Ok(mut u) = USAGE.lock() {
        let (ap, ac) = u.unwrap_or((0, 0));
        *u = Some((ap + p, ac + c));
    }
}

pub fn usage_reinit() {
    if let Ok(mut u) = USAGE.lock() {
        *u = None;
    }
}

/// Cumul depuis le dernier [`usage_reinit`] ; `None` si le serveur n'a
/// jamais renvoyé `usage` (l'appelant estime).
pub fn usage_tour() -> Option<(u64, u64)> {
    USAGE.lock().ok().and_then(|u| *u)
}

/// Événement d'une ligne SSE OpenAI-compat.
pub enum SseEvent {
    Delta(String),
    /// Raisonnement natif d'un modèle qui « pense » (`delta.reasoning` chez
    /// Ollama, `reasoning_content` chez d'autres) — distinct de la réponse.
    Reasoning(String),
    /// Fragments de tool calls du delta (index, id?, name?, arguments?).
    ToolCallDelta(Vec<(usize, Option<String>, Option<String>, Option<String>)>),
    Done,
    Ignore,
}

/// Extrait l'événement d'une ligne SSE (les tailles de chunk HTTP hexa et
/// lignes vides entrelacées tombent dans `Ignore`).
pub fn parse_sse_line(line: &str) -> SseEvent {
    let trimmed = line.trim();
    let Some(data) = trimmed.strip_prefix("data:") else {
        return SseEvent::Ignore;
    };
    let data = data.trim();
    if data == "[DONE]" {
        return SseEvent::Done;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
        return SseEvent::Ignore;
    };
    let delta = &v["choices"][0]["delta"];
    if let Some(calls) = delta["tool_calls"].as_array() {
        let fragments = calls
            .iter()
            .map(|c| {
                (
                    c["index"].as_u64().unwrap_or(0) as usize,
                    c["id"].as_str().map(str::to_owned),
                    c["function"]["name"].as_str().map(str::to_owned),
                    c["function"]["arguments"].as_str().map(str::to_owned),
                )
            })
            .collect();
        return SseEvent::ToolCallDelta(fragments);
    }
    match delta["content"].as_str() {
        Some(text) if !text.is_empty() => SseEvent::Delta(text.to_owned()),
        _ => ["reasoning", "reasoning_content"]
            .iter()
            .find_map(|c| delta[*c].as_str().filter(|t| !t.is_empty()))
            .map_or(SseEvent::Ignore, |t| SseEvent::Reasoning(t.to_owned())),
    }
}

/// Fusionne des fragments de tool calls streamés. ⚠ `index` n'est PAS une
/// position de tableau fiable : FLM émet des index arbitraires (« index »: 3
/// pour un appel unique, vécu 2026-07-05 — compteur interne). Règle : un
/// fragment portant `id` ou `name` est un NOUVEL appel ; un fragment
/// arguments-seuls (fragmentation OpenAI) se raccorde via l'index, sinon au
/// dernier appel.
fn merge_tool_call_deltas(
    calls: &mut Vec<ToolCall>,
    index_map: &mut std::collections::HashMap<usize, usize>,
    fragments: Vec<(usize, Option<String>, Option<String>, Option<String>)>,
) {
    for (index, id, name, args) in fragments {
        let pos = if id.is_some() || name.is_some() {
            calls.push(ToolCall {
                id: id.unwrap_or_default(),
                name: name.unwrap_or_default(),
                arguments: String::new(),
            });
            let pos = calls.len() - 1;
            index_map.insert(index, pos);
            pos
        } else if let Some(&pos) = index_map.get(&index) {
            pos
        } else if !calls.is_empty() {
            calls.len() - 1
        } else {
            continue; // fragment orphelin sans appel ouvert : ignoré
        };
        if let Some(args) = args {
            calls[pos].arguments.push_str(&args);
        }
    }
}

/// Décode un corps `Transfer-Encoding: chunked` (FLM ferme la connexion mais
/// chunke quand même) ; si le corps n'est pas chunké, le rend tel quel.
fn dechunk(body: &str) -> String {
    // Heuristique : un corps JSON commence par '{' — pas de taille hexa.
    if body.trim_start().starts_with('{') {
        return body.to_owned();
    }
    let mut out = String::new();
    let mut rest = body;
    loop {
        let Some(eol) = rest.find("\r\n") else { break };
        let Ok(size) = usize::from_str_radix(rest[..eol].trim(), 16) else { break };
        if size == 0 {
            break;
        }
        let start = eol + 2;
        let Some(chunk) = rest.get(start..start + size) else { break };
        out.push_str(chunk);
        rest = rest.get(start + size + 2..).unwrap_or("");
    }
    if out.is_empty() { body.to_owned() } else { out }
}

/// Retire un éventuel horodatage `[...]` que le modèle recopie en tête de
/// réponse (il singe le préfixe `[jour date heure]` des messages utilisateur).
/// Une seule fois, au tout début du flux ; au-delà de 80 octets sans `]`, on
/// relâche tout. Monté de waly-voice (2026-07-07) : tous les clients qui
/// horodatent (voix, desktop, bin) partagent la même parade.
#[derive(Default)]
pub struct BracketFilter {
    state: BracketState,
    held: String,
}

#[derive(Default, PartialEq)]
enum BracketState {
    #[default]
    Start,
    InBracket,
    Passing,
}

impl BracketFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Version une-passe pour un texte complet (clients non-streaming).
    pub fn strip(text: &str) -> String {
        let mut f = Self::new();
        let mut out = f.feed(text);
        out.push_str(&f.finish());
        out
    }

    pub fn feed(&mut self, delta: &str) -> String {
        match self.state {
            BracketState::Passing => delta.to_owned(),
            BracketState::Start => {
                let trimmed = delta.trim_start();
                if trimmed.is_empty() {
                    String::new()
                } else if trimmed.starts_with('[') {
                    self.state = BracketState::InBracket;
                    let t = trimmed.to_owned();
                    self.feed_in_bracket(&t)
                } else {
                    self.state = BracketState::Passing;
                    delta.trim_start().to_owned()
                }
            }
            BracketState::InBracket => {
                let d = delta.to_owned();
                self.feed_in_bracket(&d)
            }
        }
    }

    /// Fin de flux : rend ce qui restait retenu (un `[` jamais refermé n'est
    /// pas un horodatage — ne jamais avaler du texte pour de bon).
    pub fn finish(&mut self) -> String {
        self.state = BracketState::Passing;
        std::mem::take(&mut self.held)
    }

    fn feed_in_bracket(&mut self, chunk: &str) -> String {
        self.held.push_str(chunk);
        if let Some(pos) = self.held.find(']') {
            let after = self.held[pos + 1..].trim_start().to_owned();
            self.held.clear();
            self.state = BracketState::Passing;
            after
        } else if self.held.len() > 80 {
            // Pas un horodatage : on rend tout et on ne filtre plus.
            self.state = BracketState::Passing;
            std::mem::take(&mut self.held)
        } else {
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echantillonnage_explicite_selon_le_tour() {
        let mut b = serde_json::json!({});
        echantillonnage(&mut b, true, None);
        assert_eq!((b["temperature"].as_f64(), b["top_p"].as_f64()), (Some(0.2), Some(0.8)));
        echantillonnage(&mut b, false, None);
        assert_eq!(b["temperature"].as_f64(), Some(0.7));
        echantillonnage(&mut b, true, Some(0.0));
        assert_eq!(b["temperature"].as_f64(), Some(0.0));
        assert!(b["presence_penalty"].is_number());
    }

    #[test]
    fn usage_sse_lit_le_chunk_final() {
        let l = r#"data: {"id":"x","choices":[],"usage":{"prompt_tokens":1234,"completion_tokens":56,"total_tokens":1290}}"#;
        assert_eq!(usage_sse(l), Some((1234, 56)));
        assert_eq!(usage_sse(r#"data: {"choices":[{"delta":{"content":"usage"}}]}"#), None);
        assert_eq!(usage_sse("data: [DONE]"), None);
    }

    #[test]
    fn bracket_retire_l_horodatage_singe() {
        let mut f = BracketFilter::new();
        assert_eq!(f.feed("[samedi 4 juillet"), "");
        assert_eq!(f.feed(" 2026, 00:49] Merci !"), "Merci !");
        assert_eq!(f.feed(" [pas touche]"), " [pas touche]");
    }

    #[test]
    fn bracket_laisse_une_reponse_normale() {
        let mut f = BracketFilter::new();
        assert_eq!(f.feed("Bonjour,"), "Bonjour,");
        assert_eq!(f.feed(" [note] ok"), " [note] ok");
    }

    #[test]
    fn bracket_strip_une_passe_et_finish() {
        assert_eq!(BracketFilter::strip("[2026-07-07 14:51] Ok"), "Ok");
        assert_eq!(BracketFilter::strip("Pas de crochet."), "Pas de crochet.");
        // Un `[` jamais refermé est rendu par finish, pas avalé.
        assert_eq!(BracketFilter::strip("[incomplet"), "[incomplet");
    }

    #[test]
    fn messages_au_format_openai() {
        let m = Msg::ToolResult { call_id: "call_1".into(), content: "14:32".into() };
        assert_eq!(
            msg_to_json(&m),
            serde_json::json!({"role": "tool", "tool_call_id": "call_1", "content": "14:32"})
        );
        let m = Msg::AssistantToolCalls(vec![ToolCall {
            id: "call_1".into(),
            name: "get_time".into(),
            arguments: "{}".into(),
        }]);
        let v = msg_to_json(&m);
        assert_eq!(v["role"], "assistant");
        assert_eq!(v["tool_calls"][0]["function"]["name"], "get_time");
    }

    #[test]
    fn parse_tool_calls_flm() {
        // Réponse réelle FLM v0.9.43 (banc 2026-07-05), abrégée.
        let body = r#"{"choices":[{"message":{"role":"assistant","tool_calls":[{"index":0,"id":"call_1783205927","type":"function","function":{"name":"get_time","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}"#;
        match parse_completion(body).unwrap() {
            Turn::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "get_time");
                assert_eq!(calls[0].arguments, "{}");
            }
            other => panic!("tool calls attendus, recu {other:?}"),
        }
    }

    #[test]
    fn parse_texte_final() {
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"Il est 14h32."},"finish_reason":"stop"}]}"#;
        assert_eq!(parse_completion(body).unwrap(), Turn::Text("Il est 14h32.".into()));
    }

    #[test]
    fn parse_reponse_vide_est_une_erreur() {
        let body = r#"{"choices":[{"message":{"role":"assistant"},"finish_reason":"stop"}]}"#;
        assert!(parse_completion(body).is_err());
    }

    #[test]
    fn sse_delta_texte_et_done() {
        match parse_sse_line(r#"data: {"choices":[{"delta":{"content":"Bonjour"}}]}"#) {
            SseEvent::Delta(t) => assert_eq!(t, "Bonjour"),
            _ => panic!("delta attendu"),
        }
        assert!(matches!(parse_sse_line("data: [DONE]"), SseEvent::Done));
        assert!(matches!(parse_sse_line("1a3\r\n"), SseEvent::Ignore)); // taille de chunk
        assert!(matches!(
            parse_sse_line(r#"data: {"choices":[{"delta":{"content":null},"finish_reason":"tool_calls"}]}"#),
            SseEvent::Ignore
        ));
    }

    #[test]
    fn sse_raisonnement_natif_distinct_de_la_reponse() {
        let l = r#"data: {"choices":[{"delta":{"role":"assistant","content":"","reasoning":"Posons"}}]}"#;
        assert!(matches!(parse_sse_line(l), SseEvent::Reasoning(t) if t == "Posons"));
        let l = r#"data: {"choices":[{"delta":{"reasoning_content":"le calcul"}}]}"#;
        assert!(matches!(parse_sse_line(l), SseEvent::Reasoning(t) if t == "le calcul"));
        let l = r#"data: {"choices":[{"delta":{"content":"Oui","reasoning":""}}]}"#;
        assert!(matches!(parse_sse_line(l), SseEvent::Delta(t) if t == "Oui"));
    }

    #[test]
    fn sse_tool_call_flm_index_arbitraire() {
        // Ligne réelle FLM v0.9.43 (banc 2026-07-05) : « index »: 3 pour un
        // appel UNIQUE — l'index FLM n'est pas une position de tableau.
        let line = r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":3,"id":"call_1","type":"function","function":{"name":"memoriser","arguments":"{\"cle\":\"sortie_paul\"}"}}]},"finish_reason":null}]}"#;
        let SseEvent::ToolCallDelta(frags) = parse_sse_line(line) else {
            panic!("tool call delta attendu");
        };
        let mut calls = Vec::new();
        let mut map = std::collections::HashMap::new();
        merge_tool_call_deltas(&mut calls, &mut map, frags);
        assert_eq!(calls.len(), 1, "aucun appel vide ne doit apparaitre");
        assert_eq!(calls[0].name, "memoriser");
        assert_eq!(calls[0].arguments, r#"{"cle":"sortie_paul"}"#);
    }

    #[test]
    fn merge_deux_appels_flm_index_disjoints() {
        let mut calls = Vec::new();
        let mut map = std::collections::HashMap::new();
        merge_tool_call_deltas(
            &mut calls,
            &mut map,
            vec![(3, Some("c1".into()), Some("heure".into()), Some("{}".into()))],
        );
        merge_tool_call_deltas(
            &mut calls,
            &mut map,
            vec![(7, Some("c2".into()), Some("memoriser".into()), Some("{}".into()))],
        );
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].name, "memoriser");
    }

    #[test]
    fn merge_tool_calls_fragmentes_style_openai() {
        let mut calls = Vec::new();
        let mut map = std::collections::HashMap::new();
        merge_tool_call_deltas(
            &mut calls,
            &mut map,
            vec![(0, Some("call_1".into()), Some("meteo".into()), Some("{\"vi".into()))],
        );
        merge_tool_call_deltas(&mut calls, &mut map, vec![(0, None, None, Some("lle\":\"Paris\"}".into()))]);
        assert_eq!(calls[0].arguments, "{\"ville\":\"Paris\"}");
        assert_eq!(calls[0].name, "meteo");
        // Fragment orphelin (index inconnu, aucun appel) : ignoré sans panique.
        let mut vides = Vec::new();
        let mut map2 = std::collections::HashMap::new();
        merge_tool_call_deltas(&mut vides, &mut map2, vec![(5, None, None, Some("x".into()))]);
        assert!(vides.is_empty());
    }

    #[test]
    fn dechunk_corps_chunke() {
        assert_eq!(dechunk("5\r\n{\"a\":\r\n2\r\n1}\r\n0\r\n\r\n"), "{\"a\":1}");
        assert_eq!(dechunk("{\"deja\":\"plat\"}"), "{\"deja\":\"plat\"}");
    }
}
