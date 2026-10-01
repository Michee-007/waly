//! Métriques décomposées par tour de parole.
//!
//! Reprend la bonne idée de l'ancien `voice_loop.py` : chaque tour loggue sa
//! décomposition « VAD + STT + (LLM → 1re clause → TTS) → PREMIER SON », ce
//! qui permet de voir immédiatement quelle brique mange le budget de 1 s.

use std::time::{Duration, Instant};

/// Horloge d'un tour : on marque les jalons au fil du pipeline puis on
/// produit un rapport. Tous les jalons sont optionnels (un tour peut échouer
/// à mi-chemin).
#[derive(Debug, Clone)]
pub struct TurnClock {
    /// Durée du silence de fin de tour (payée AVANT speech_end, par design).
    pub end_of_turn_silence: Duration,
    /// Fin de parole détectée (après le silence) : t0 du budget perçu.
    speech_end: Option<Instant>,
    stt_final: Option<Instant>,
    llm_first_token: Option<Instant>,
    first_clause: Option<Instant>,
    first_audio: Option<Instant>,
    llm_done: Option<Instant>,
}

impl TurnClock {
    pub fn new(end_of_turn_silence: Duration) -> Self {
        Self {
            end_of_turn_silence,
            speech_end: None,
            stt_final: None,
            llm_first_token: None,
            first_clause: None,
            first_audio: None,
            llm_done: None,
        }
    }

    pub fn mark_speech_end(&mut self) {
        self.speech_end = Some(Instant::now());
    }
    pub fn mark_stt_final(&mut self) {
        self.stt_final = Some(Instant::now());
    }
    pub fn mark_llm_first_token(&mut self) {
        self.llm_first_token = Some(Instant::now());
    }
    pub fn mark_first_clause(&mut self) {
        self.first_clause = Some(Instant::now());
    }
    pub fn mark_first_audio(&mut self) {
        self.first_audio = Some(Instant::now());
    }
    pub fn mark_llm_done(&mut self) {
        self.llm_done = Some(Instant::now());
    }

    fn span(from: Option<Instant>, to: Option<Instant>) -> Option<Duration> {
        match (from, to) {
            (Some(a), Some(b)) => b.checked_duration_since(a),
            _ => None,
        }
    }

    /// Fin de parole → transcription finale.
    pub fn stt(&self) -> Option<Duration> {
        Self::span(self.speech_end, self.stt_final)
    }

    /// Transcription finale → premier son audible (LLM + 1re clause + TTS).
    pub fn brain_to_audio(&self) -> Option<Duration> {
        Self::span(self.stt_final, self.first_audio)
    }

    /// Fin de parole → premier son : LA métrique de sortie de R1 (≤ 1 s).
    pub fn to_first_audio(&self) -> Option<Duration> {
        Self::span(self.speech_end, self.first_audio)
    }

    /// Latence perçue par l'utilisateur : silence de fin de tour compris.
    pub fn perceived(&self) -> Option<Duration> {
        self.to_first_audio().map(|d| d + self.end_of_turn_silence)
    }

    /// Ligne de rapport façon voice_loop.py, prête pour `tracing::info!`.
    /// Décompose le segment cerveau : TTFT (réseau+préfill), accumulation de
    /// la 1re clause (décodage), synthèse TTS.
    pub fn summary(&self) -> String {
        fn fmt(d: Option<Duration>) -> String {
            match d {
                Some(d) => format!("{:.2}s", d.as_secs_f64()),
                None => "—".into(),
            }
        }
        format!(
            "VAD {:.2}s + STT {} + (TTFT {} + clause {} + TTS {}) → PREMIER SON ≈ {} (LLM full {})",
            self.end_of_turn_silence.as_secs_f64(),
            fmt(self.stt()),
            fmt(Self::span(self.stt_final, self.llm_first_token)),
            fmt(Self::span(self.llm_first_token, self.first_clause)),
            fmt(Self::span(self.first_clause, self.first_audio)),
            fmt(self.perceived()),
            fmt(Self::span(self.stt_final, self.llm_done)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jalons_dans_l_ordre() {
        let mut clock = TurnClock::new(Duration::from_millis(800));
        clock.mark_speech_end();
        clock.mark_stt_final();
        clock.mark_llm_first_token();
        clock.mark_first_clause();
        clock.mark_first_audio();
        clock.mark_llm_done();
        assert!(clock.stt().is_some());
        assert!(clock.to_first_audio().is_some());
        // perçu = premier son + 800 ms de silence
        let p = clock.perceived().unwrap();
        assert!(p >= Duration::from_millis(800));
        assert!(clock.summary().contains("PREMIER SON"));
    }

    #[test]
    fn tour_incomplet_ne_panique_pas() {
        let mut clock = TurnClock::new(Duration::from_millis(800));
        clock.mark_speech_end();
        assert_eq!(clock.stt(), None);
        assert_eq!(clock.perceived(), None);
        assert!(clock.summary().contains("—"));
    }
}
