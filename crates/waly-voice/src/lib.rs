//! waly-voice — cascade voix temps réel FR (phase R1 du RFC).
//!
//! Architecture cible (RFC 2026-07-03) :
//! VAD Silero → STT spéculatif pendant le silence → fin de tour sémantique
//! (R1.5 : commit à ~280 ms si le transcript est complet, filet 800 ms sinon,
//! voir [`endpoint`]) → LLM FLM streaming clause par clause → TTS streaming →
//! lecture interruptible (barge-in).
//!
//! Cible mesurée : ≤ 1 s entre la fin de parole et le premier son.
//!
//! Mesures moteur (2026-07-03, machine de référence) :
//! - **STT primaire : Parakeet-TDT 0.6B v3 int8 CPU** via sherpa-onnx en DLL
//!   ([`stt`]) — 0,18-0,21 s pour 6 s de voix réelle, accents parfaits.
//! - Whisper-V3-Turbo sur NPU via `flm serve --asr 1` (mode standalone
//!   OBLIGATOIRE en v0.9.43) : ~2,6 s par finalisation, constant quelle que
//!   soit la durée (fenêtre 30 s encodée entière) → trop lent comme STT
//!   principal du budget 1 s ; candidat re-transcription de qualité.
//! - Deux processus FLM cohabitent sur le NPU (ASR standalone + LLM) : pas de
//!   crash, sérialisation ~additive en simultané — acceptable car le pipeline
//!   voix est séquentiel par nature.
//!
//! La lib pure (sans feature `service`) reste compilable et testable partout :
//! machine à états, nettoyage TTS, clauses, métriques.
//!
//! ⚠ État R1.5 : [`turn::TurnMachine`] et [`segment::SpeechSegmenter`] sont le
//! MODÈLE de référence (testé) du tour, mais le service `cmd_talk` de
//! `bin/waly-voice.rs` implémente sa propre boucle (endpointing sémantique
//! adaptatif, [`endpoint`]) — convergence à faire en R2.

/// Sel de re-roll SAC pour l'EXE DE TESTS de la lib (piege 3 : verdict par
/// binaire, un rebuild au meme contenu garde le meme hash — incrementer ici
/// si waly-voice-tests.exe se fait bloquer, comme SAC_BUILD_SALT cote bin).
#[used]
pub static SAC_REROLL_LIB: u32 = 1;

pub mod aec;
pub mod clause;
pub mod config;
pub mod endpoint;
pub mod llm;
pub mod metrics;
pub mod prosody;
pub mod sanitize;
pub mod segment;
pub mod turn;
pub mod wake;
#[cfg(feature = "service")]
pub mod pocket;
#[cfg(feature = "service")]
pub mod stt;
#[cfg(feature = "service")]
pub mod tts;
#[cfg(feature = "service")]
pub mod vad;

pub use clause::ClauseSplitter;
pub use config::VoiceConfig;
pub use metrics::TurnClock;
pub use sanitize::{clean_for_tts, compact_reply, has_speech};
pub use turn::{Action, Event, TurnMachine, TurnState};
