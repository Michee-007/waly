//! Client LLM streaming minimal pour FastFlowLM (OpenAI-compat, localhost).
//!
//! Pur `std::net` + serde_json : pas de reqwest/tokio pour parler à
//! 127.0.0.1. Le streaming SSE permet de découper la réponse en clauses et
//! de lancer le TTS sans attendre la fin (cœur du budget ≤ 1 s).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub struct LlmClient {
    pub host: String,
    pub port: u16,
    pub model: String,
}

/// Comment la génération s'est terminée. `Truncated` (EOF serveur sans
/// `[DONE]`) doit être signalé à l'utilisateur : la phrase entendue était
/// incomplète, l'historique ne doit pas la croire entière.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatEnd {
    /// `data: [DONE]` reçu — réponse complète.
    Done,
    /// Interrompue par l'appelant (barge-in).
    Aborted,
    /// Le serveur a fermé le flux avant `[DONE]` — réponse tronquée.
    Truncated,
}

pub struct ChatReply {
    pub text: String,
    pub end: ChatEnd,
}

impl LlmClient {
    pub fn new(host: &str, port: u16, model: &str) -> Self {
        Self { host: host.into(), port, model: model.into() }
    }

    /// Envoie la conversation en streaming. `on_delta` reçoit chaque fragment
    /// de texte ; retourner `false` interrompt la génération (barge-in).
    ///
    /// **Sonde d'interruption** : pendant les attentes réseau (TTFT de
    /// 1-2 s !), `on_delta` est appelé toutes les ~50 ms avec un fragment
    /// VIDE — l'appelant peut ainsi vérifier le barge-in même quand aucun
    /// token n'arrive. Ne pas confondre « premier appel » et « premier
    /// token » : tester `delta.is_empty()`.
    ///
    /// Retourne le texte complet reçu et la façon dont le flux s'est terminé.
    pub fn chat_stream(
        &self,
        messages: &[(String, String)],
        mut on_delta: impl FnMut(&str) -> bool,
    ) -> Result<ChatReply, Box<dyn std::error::Error>> {
        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|(role, content)| serde_json::json!({"role": role, "content": content}))
            .collect();
        let body = serde_json::json!({
            "model": self.model,
            "messages": msgs,
            "stream": true,
            // 160 : un assistant VOCAL n'a pas le droit aux tirades — le 4B
            // ignore le « une a trois phrases » du prompt quand on lui laisse
            // 256 tokens (tirades de 11 s vecues le 2026-07-05).
            "max_tokens": 160,
        })
        .to_string();

        let mut stream = TcpStream::connect((self.host.as_str(), self.port)).map_err(|e| {
            format!(
                "LLM injoignable sur {}:{} ({e}) — lancer engines/start-flm.ps1 -Port {}",
                self.host, self.port, self.port
            )
        })?;
        stream.set_nodelay(true)?; // chaque delta SSE doit partir/arriver sans attendre
        write!(
            stream,
            "POST /v1/chat/completions HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.host, self.port, body.len(), body
        )?;
        // Socket NON BLOQUANT + sommeil applicatif : chaque WouldBlock est une
        // occasion de sonder le barge-in via on_delta(""). SURTOUT PAS de
        // SO_RCVTIMEO (set_read_timeout) : sous Windows un timeout laisse le
        // socket dans un etat indetermine — vecu le 2026-07-04, les RST qui
        // en sortaient ont fait tomber le serveur FLM. Lecture en octets bruts
        // (pas de read_line : une lecture partielle en perdrait le debut).
        stream.set_nonblocking(true)?;

        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        let mut raw: Vec<u8> = Vec::new(); // octets bruts tant que les en-tetes courent
        let mut body: Vec<u8> = Vec::new(); // corps HTTP de-chunke
        let mut chunk = [0u8; 8192];
        let mut status_seen = false;
        let mut headers_done = false;
        let mut chunked = false;
        let mut decoder = ChunkDecoder::new();
        let mut full = String::new();
        loop {
            // La deadline vaut aussi pour un flux qui debite : sans ce test
            // ici, seul le silence (WouldBlock) la verifiait.
            if std::time::Instant::now() > deadline {
                abandon_stream(stream);
                return Err("timeout LLM (120 s)".into());
            }
            match stream.read(&mut chunk) {
                Ok(0) => break, // connexion fermee
                Ok(n) => {
                    let mut input = &chunk[..n];
                    if !headers_done {
                        raw.extend_from_slice(input);
                        input = &[];
                        // Consommer statut + en-tetes ligne a ligne.
                        while let Some(pos) = raw.iter().position(|&b| b == b'\n') {
                            let line_bytes: Vec<u8> = raw.drain(..=pos).collect();
                            let line = String::from_utf8_lossy(&line_bytes);
                            if !status_seen {
                                if !(line.starts_with("HTTP/") && line.contains(" 200")) {
                                    return Err(
                                        format!("reponse LLM inattendue: {}", line.trim()).into()
                                    );
                                }
                                status_seen = true;
                            } else if line.trim().is_empty() {
                                headers_done = true;
                                break;
                            } else if let Some(v) =
                                line.to_ascii_lowercase().strip_prefix("transfer-encoding:")
                            {
                                chunked = v.contains("chunked");
                            }
                        }
                    }
                    if headers_done {
                        // Le debut du corps a pu arriver colle aux en-tetes.
                        if chunked {
                            decoder.feed(&raw, &mut body);
                            decoder.feed(input, &mut body);
                        } else {
                            body.extend_from_slice(&raw);
                            body.extend_from_slice(input);
                        }
                        raw.clear();
                    }
                    // Traiter toutes les lignes SSE completes du corps.
                    while let Some(pos) = body.iter().position(|&b| b == b'\n') {
                        let line_bytes: Vec<u8> = body.drain(..=pos).collect();
                        let line = String::from_utf8_lossy(&line_bytes);
                        match parse_sse_line(&line) {
                            SseEvent::Delta(text) => {
                                full.push_str(&text);
                                if !on_delta(&text) {
                                    abandon_stream(stream); // barge-in
                                    return Ok(ChatReply { text: full, end: ChatEnd::Aborted });
                                }
                            }
                            SseEvent::Done => {
                                return Ok(ChatReply { text: full, end: ChatEnd::Done })
                            }
                            SseEvent::Ignore => {}
                        }
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    // Rien recu : sonde d'interruption pendant l'attente.
                    if !on_delta("") {
                        abandon_stream(stream); // barge-in pendant le TTFT
                        return Ok(ChatReply { text: full, end: ChatEnd::Aborted });
                    }
                    std::thread::sleep(Duration::from_millis(30));
                }
                Err(e) => return Err(e.into()),
            }
        }
        // EOF sans [DONE] : parser une eventuelle derniere ligne partielle
        // (serveur qui clot sans \n final), puis signaler la troncature —
        // la reponse jouee est peut-etre incomplete, l'appelant doit le dire.
        if !body.is_empty() {
            let line = String::from_utf8_lossy(&body).into_owned();
            match parse_sse_line(&line) {
                SseEvent::Delta(text) => {
                    full.push_str(&text);
                    on_delta(&text);
                }
                SseEvent::Done => return Ok(ChatReply { text: full, end: ChatEnd::Done }),
                SseEvent::Ignore => {}
            }
        }
        Ok(ChatReply { text: full, end: ChatEnd::Truncated })
    }

    /// Requête d'échauffement : non-streaming, 1 token, lue JUSQU'AU BOUT
    /// puis fermée proprement. Ne JAMAIS chauffer en avortant une génération
    /// streaming : couper le socket pendant que FLM écrit est brutal pour le
    /// serveur, et l'échauffement n'a besoin que du préfill.
    pub fn warmup(&self) -> Result<(), Box<dyn std::error::Error>> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": "ok"}],
            "stream": false,
            "max_tokens": 1,
        })
        .to_string();
        let mut stream = TcpStream::connect((self.host.as_str(), self.port)).map_err(|e| {
            format!(
                "LLM injoignable sur {}:{} ({e}) — lancer engines/start-flm.ps1 -Port {}",
                self.host, self.port, self.port
            )
        })?;
        write!(
            stream,
            "POST /v1/chat/completions HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.host, self.port, body.len(), body
        )?;
        // Lire la reponse entiere (le premier tour apres chargement peut
        // prendre 5-10 s : piege documente).
        stream.set_nonblocking(true)?;
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
                Err(e) => return Err(e.into()),
            }
        }
    }
}

/// Abandon PROPRE d'un flux en cours (barge-in) : on ne claque pas le socket
/// au nez de FLM pendant qu'il écrit — les RST l'ont déjà fait tomber (vécu
/// deux fois). Un thread détaché lit le reste du flux jusqu'à EOF (ou 10 s)
/// puis ferme proprement. Pas de `set_read_timeout` : piège SO_RCVTIMEO
/// Windows (CLAUDE.md n°7), le socket est déjà non bloquant.
fn abandon_stream(stream: TcpStream) {
    std::thread::spawn(move || {
        let mut s = stream;
        let mut sink = [0u8; 8192];
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            match s.read(&mut sink) {
                Ok(0) => break, // EOF : le serveur a fini d'ecrire
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

/// Décodeur incrémental de corps HTTP `Transfer-Encoding: chunked`
/// (RFC 9112 §7.1). Une frontière de chunk peut tomber N'IMPORTE OÙ, y
/// compris au milieu d'un `data: {...}` SSE : découper le flux brut sur
/// `\n` sans dé-chunker perdrait silencieusement le token coupé.
struct ChunkDecoder {
    state: ChunkState,
    remaining: usize,
    size_line: Vec<u8>,
}

enum ChunkState {
    Size,    // ligne `taille-hex[;extension]\r\n`
    Data,    // `remaining` octets de corps à copier
    DataEnd, // consommer le \r\n qui suit les données du chunk
    Trailer, // chunk de taille 0 vu : tout le reste est trailer, ignoré
}

impl ChunkDecoder {
    fn new() -> Self {
        Self { state: ChunkState::Size, remaining: 0, size_line: Vec::new() }
    }

    /// Décode `input` et pousse les octets de corps dans `out`.
    fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) {
        let mut i = 0;
        while i < input.len() {
            match self.state {
                ChunkState::Size => {
                    let b = input[i];
                    i += 1;
                    if b == b'\n' {
                        let line = String::from_utf8_lossy(&self.size_line).into_owned();
                        self.size_line.clear();
                        let hex = line.trim().split(';').next().unwrap_or("").trim().to_owned();
                        if hex.is_empty() {
                            continue; // séparateur résiduel, on reste en Size
                        }
                        match usize::from_str_radix(&hex, 16) {
                            Ok(0) => self.state = ChunkState::Trailer,
                            Ok(n) => {
                                self.remaining = n;
                                self.state = ChunkState::Data;
                            }
                            // Flux corrompu : on s'arrête plutôt que de
                            // réinterpréter du corps comme des tailles.
                            Err(_) => self.state = ChunkState::Trailer,
                        }
                    } else if b != b'\r' {
                        self.size_line.push(b);
                    }
                }
                ChunkState::Data => {
                    let take = self.remaining.min(input.len() - i);
                    out.extend_from_slice(&input[i..i + take]);
                    i += take;
                    self.remaining -= take;
                    if self.remaining == 0 {
                        self.state = ChunkState::DataEnd;
                    }
                }
                ChunkState::DataEnd => {
                    if input[i] == b'\n' {
                        self.state = ChunkState::Size;
                    }
                    i += 1; // \r puis \n consommés octet par octet
                }
                ChunkState::Trailer => return,
            }
        }
    }
}

pub enum SseEvent {
    Delta(String),
    Done,
    Ignore,
}

/// Extrait le fragment de texte d'une ligne SSE OpenAI-compat.
pub fn parse_sse_line(line: &str) -> SseEvent {
    let trimmed = line.trim();
    let Some(data) = trimmed.strip_prefix("data:") else {
        return SseEvent::Ignore; // taille de chunk, ligne vide, commentaire...
    };
    let data = data.trim();
    if data == "[DONE]" {
        return SseEvent::Done;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
        return SseEvent::Ignore;
    };
    match v["choices"][0]["delta"]["content"].as_str() {
        Some(text) if !text.is_empty() => SseEvent::Delta(text.to_owned()),
        _ => SseEvent::Ignore,
    }
}

/// Filtre le bloc de raisonnement `<think>…</think>` que qwen3 peut émettre
/// en tête de flux — on ne vocalise JAMAIS la réflexion interne. Gère les
/// balises coupées entre deux fragments SSE.
#[derive(Default)]
pub struct ThinkFilter {
    state: ThinkState,
    held: String,
}

#[derive(Default, PartialEq)]
enum ThinkState {
    #[default]
    Start, // on ne sait pas encore si le flux commence par <think>
    InThink,
    Passing,
}

impl ThinkFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pousse un fragment brut, retourne ce qui doit être vocalisé.
    pub fn feed(&mut self, delta: &str) -> String {
        const OPEN: &str = "<think>";
        match self.state {
            ThinkState::Passing => delta.to_owned(),
            ThinkState::Start => {
                self.held.push_str(delta);
                let trimmed = self.held.trim_start();
                if trimmed.is_empty() || (OPEN.starts_with(trimmed) && trimmed.len() < OPEN.len())
                {
                    String::new() // pas encore decidable
                } else if trimmed.starts_with(OPEN) {
                    self.state = ThinkState::InThink;
                    let held = std::mem::take(&mut self.held);
                    self.feed_in_think(&held)
                } else {
                    self.state = ThinkState::Passing;
                    std::mem::take(&mut self.held)
                }
            }
            ThinkState::InThink => {
                let d = delta.to_owned();
                self.feed_in_think(&d)
            }
        }
    }

    fn feed_in_think(&mut self, chunk: &str) -> String {
        const CLOSE: &str = "</think>";
        self.held.push_str(chunk);
        if let Some(pos) = self.held.find(CLOSE) {
            let after = self.held[pos + CLOSE.len()..].to_owned();
            self.held.clear();
            self.state = ThinkState::Passing;
            after
        } else {
            // Garder une queue courte : la balise fermante peut arriver coupee.
            if self.held.len() > CLOSE.len() {
                let keep_from = self.held.len() - CLOSE.len();
                // Rester sur une frontiere UTF-8.
                let keep_from = (0..=keep_from)
                    .rev()
                    .find(|&i| self.held.is_char_boundary(i))
                    .unwrap_or(0);
                self.held.drain(..keep_from);
            }
            String::new()
        }
    }
}

/// Retire l'horodatage `[...]` que le modèle recopie en tête de réponse.
/// Monté dans waly-core (2026-07-07) pour servir aussi desktop et bin —
/// ré-exporté ici (seul le service en a l'usage, et la lib pure n'a pas
/// waly-core) pour garder l'API de la voix inchangée.
#[cfg(feature = "service")]
pub use waly_core::llm::BracketFilter;

#[cfg(test)]
mod tests {
    use super::*;

    // Les tests de BracketFilter ont suivi le code dans waly-core (llm.rs).

    #[test]
    fn think_filtre_le_raisonnement() {
        let mut f = ThinkFilter::new();
        assert_eq!(f.feed("<think>"), "");
        assert_eq!(f.feed("je reflechis..."), "");
        assert_eq!(f.feed("</think>Bonjour !"), "Bonjour !");
        assert_eq!(f.feed(" Ca va ?"), " Ca va ?");
    }

    #[test]
    fn think_balise_coupee_entre_fragments() {
        let mut f = ThinkFilter::new();
        assert_eq!(f.feed("<th"), "");
        assert_eq!(f.feed("ink>blabla</th"), "");
        assert_eq!(f.feed("ink> Voila."), " Voila.");
    }

    #[test]
    fn think_absent_passe_tout() {
        let mut f = ThinkFilter::new();
        assert_eq!(f.feed("Salut"), "Salut");
        assert_eq!(f.feed(" toi."), " toi.");
    }

    #[test]
    fn parse_delta() {
        let l = r#"data: {"choices":[{"delta":{"content":"Bonjour"}}]}"#;
        match parse_sse_line(l) {
            SseEvent::Delta(t) => assert_eq!(t, "Bonjour"),
            _ => panic!("delta attendu"),
        }
    }

    #[test]
    fn dechunke_une_frontiere_en_plein_json() {
        // Le token SSE est coupe en deux chunks HTTP au milieu du JSON :
        // sans de-chunkage, la ligne serait scindee et le token perdu.
        let part1 = br#"data: {"choices":[{"delta":{"con"#;
        let part2 = "tent\":\"Bonjour\"}}]}\n".as_bytes();
        let mut wire = Vec::new();
        wire.extend_from_slice(format!("{:x}\r\n", part1.len()).as_bytes());
        wire.extend_from_slice(part1);
        wire.extend_from_slice(b"\r\n");
        wire.extend_from_slice(format!("{:x}\r\n", part2.len()).as_bytes());
        wire.extend_from_slice(part2);
        wire.extend_from_slice(b"\r\n0\r\n\r\n");

        // Nourri octet par octet pour stresser toutes les transitions d'etat.
        let mut dec = ChunkDecoder::new();
        let mut body = Vec::new();
        for b in &wire {
            dec.feed(std::slice::from_ref(b), &mut body);
        }
        let line = String::from_utf8(body).unwrap();
        match parse_sse_line(&line) {
            SseEvent::Delta(t) => assert_eq!(t, "Bonjour"),
            _ => panic!("delta attendu apres de-chunkage"),
        }
    }

    #[test]
    fn dechunke_extension_et_trailers() {
        let mut dec = ChunkDecoder::new();
        let mut body = Vec::new();
        dec.feed(b"5;ext=oui\r\nhello\r\n0\r\nX-Trailer: 1\r\n\r\n", &mut body);
        assert_eq!(body, b"hello");
    }

    #[test]
    fn dechunke_taille_corrompue_sans_boucler() {
        let mut dec = ChunkDecoder::new();
        let mut body = Vec::new();
        dec.feed(b"zzz\r\ndata: perdu\r\n", &mut body);
        assert!(body.is_empty()); // flux corrompu : on jette, on ne panique pas
    }

    #[test]
    fn parse_done_et_bruit() {
        assert!(matches!(parse_sse_line("data: [DONE]"), SseEvent::Done));
        assert!(matches!(parse_sse_line("1a3\r\n"), SseEvent::Ignore)); // taille de chunk
        assert!(matches!(parse_sse_line("\r\n"), SseEvent::Ignore));
        assert!(matches!(
            parse_sse_line(r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#),
            SseEvent::Ignore
        ));
    }
}
