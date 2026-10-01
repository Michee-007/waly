//! Machine à états du tour de parole, barge-in compris.
//!
//! Déterministe et sans I/O : le service (waly-voice.exe) traduit les
//! événements du monde réel (VAD, STT, LLM, lecteur audio) en [`Event`], la
//! machine répond par des [`Action`] à exécuter. Testable sans micro.
//!
//! États et transitions (v0) :
//!
//! ```text
//! Idle ──SpeechStart──▶ Listening ──SilenceElapsed──▶ Finalizing
//!   ▲                      ▲                              │SttFinal
//!   │                      │barge-in (SpeechStart)        ▼
//!   └──PlaybackDone── Speaking ◀──FirstAudio─────────── Thinking
//! ```

use crate::metrics::TurnClock;
use std::time::Duration;

/// État courant du dialogue vocal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnState {
    /// Silence, personne ne parle, rien ne joue.
    Idle,
    /// L'utilisateur parle (VAD actif) ; le STT streaming tourne en tâche de
    /// fond pour pré-transcrire.
    Listening,
    /// Fin de tour détectée : on attend la transcription finale.
    Finalizing,
    /// Transcription envoyée au LLM ; clauses en cours de synthèse.
    Thinking,
    /// De l'audio sort des haut-parleurs (interruptible).
    Speaking,
}

/// Événements produits par les briques du pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Le VAD détecte un début de parole.
    SpeechStart,
    /// Le silence de fin de tour (cf. config) s'est écoulé.
    SilenceElapsed,
    /// La transcription finale de l'énoncé est prête.
    SttFinal(String),
    /// Le premier échantillon audio de la réponse est parti au haut-parleur.
    FirstAudio,
    /// La lecture de la réponse est terminée.
    PlaybackDone,
    /// Le tour a échoué (moteur injoignable, transcription vide…).
    Aborted,
}

/// Ordres à exécuter par le service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Démarrer la capture/pré-transcription STT.
    StartStt,
    /// Demander la transcription finale de l'énoncé.
    FinalizeStt,
    /// Lancer le LLM (streaming clause par clause) sur ce texte.
    QueryLlm(String),
    /// Stopper net la lecture ET annuler la génération en cours (barge-in).
    StopPlayback,
    /// Logger le rapport de latence du tour qui s'achève.
    ReportMetrics,
}

/// Machine à états d'un dialogue. Chaque tour a son [`TurnClock`].
#[derive(Debug)]
pub struct TurnMachine {
    state: TurnState,
    end_of_turn_silence: Duration,
    clock: TurnClock,
}

impl TurnMachine {
    pub fn new(end_of_turn_silence: Duration) -> Self {
        Self {
            state: TurnState::Idle,
            end_of_turn_silence,
            clock: TurnClock::new(end_of_turn_silence),
        }
    }

    pub fn state(&self) -> TurnState {
        self.state
    }

    /// Horloge du tour courant (jalons LLM à marquer par le service).
    pub fn clock_mut(&mut self) -> &mut TurnClock {
        &mut self.clock
    }

    fn new_turn(&mut self) {
        self.clock = TurnClock::new(self.end_of_turn_silence);
    }

    /// Fait avancer la machine ; retourne les actions à exécuter, dans
    /// l'ordre.
    pub fn on_event(&mut self, event: Event) -> Vec<Action> {
        use TurnState::*;
        match (self.state, event) {
            (Idle, Event::SpeechStart) => {
                self.new_turn();
                self.state = Listening;
                vec![Action::StartStt]
            }
            // Barge-in : la voix de l'utilisateur coupe la parole (et la
            // génération) en cours, et ouvre immédiatement un nouveau tour.
            (Speaking, Event::SpeechStart) => {
                self.new_turn();
                self.state = Listening;
                vec![Action::StopPlayback, Action::StartStt]
            }
            (Listening, Event::SilenceElapsed) => {
                self.state = Finalizing;
                self.clock.mark_speech_end();
                vec![Action::FinalizeStt]
            }
            (Finalizing, Event::SttFinal(text)) => {
                self.clock.mark_stt_final();
                if crate::sanitize::has_speech(&text) {
                    self.state = Thinking;
                    vec![Action::QueryLlm(text)]
                } else {
                    // Faux positif du VAD (bruit) : on ne réveille pas le LLM.
                    self.state = Idle;
                    vec![]
                }
            }
            (Thinking, Event::FirstAudio) => {
                self.clock.mark_first_audio();
                self.state = Speaking;
                vec![]
            }
            (Speaking, Event::PlaybackDone) => {
                self.state = Idle;
                vec![Action::ReportMetrics]
            }
            // Échec n'importe où : retour au calme, on loggue ce qu'on a.
            (_, Event::Aborted) => {
                self.state = Idle;
                vec![Action::StopPlayback, Action::ReportMetrics]
            }
            // Événement hors séquence (course bénigne) : ignoré mais tracé.
            (state, event) => {
                tracing::debug!(?state, ?event, "événement ignoré");
                vec![]
            }
        }
    }

    /// Rapport de latence du tour courant.
    pub fn summary(&self) -> String {
        self.clock.summary()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> TurnMachine {
        TurnMachine::new(Duration::from_millis(800))
    }

    #[test]
    fn tour_nominal() {
        let mut m = machine();
        assert_eq!(m.on_event(Event::SpeechStart), vec![Action::StartStt]);
        assert_eq!(m.on_event(Event::SilenceElapsed), vec![Action::FinalizeStt]);
        assert_eq!(
            m.on_event(Event::SttFinal("Quelle heure est-il ?".into())),
            vec![Action::QueryLlm("Quelle heure est-il ?".into())]
        );
        assert_eq!(m.on_event(Event::FirstAudio), vec![]);
        assert_eq!(m.state(), TurnState::Speaking);
        assert_eq!(m.on_event(Event::PlaybackDone), vec![Action::ReportMetrics]);
        assert_eq!(m.state(), TurnState::Idle);
    }

    #[test]
    fn barge_in_coupe_la_lecture() {
        let mut m = machine();
        m.on_event(Event::SpeechStart);
        m.on_event(Event::SilenceElapsed);
        m.on_event(Event::SttFinal("Raconte une histoire.".into()));
        m.on_event(Event::FirstAudio);
        assert_eq!(m.state(), TurnState::Speaking);
        // L'utilisateur reprend la parole pendant la lecture :
        assert_eq!(
            m.on_event(Event::SpeechStart),
            vec![Action::StopPlayback, Action::StartStt]
        );
        assert_eq!(m.state(), TurnState::Listening);
    }

    #[test]
    fn bruit_sans_parole_ne_reveille_pas_le_llm() {
        let mut m = machine();
        m.on_event(Event::SpeechStart);
        m.on_event(Event::SilenceElapsed);
        assert_eq!(m.on_event(Event::SttFinal("...".into())), vec![]);
        assert_eq!(m.state(), TurnState::Idle);
    }

    #[test]
    fn abandon_remet_au_calme() {
        let mut m = machine();
        m.on_event(Event::SpeechStart);
        m.on_event(Event::SilenceElapsed);
        let actions = m.on_event(Event::Aborted);
        assert!(actions.contains(&Action::StopPlayback));
        assert_eq!(m.state(), TurnState::Idle);
    }

    #[test]
    fn evenement_hors_sequence_ignore() {
        let mut m = machine();
        assert_eq!(m.on_event(Event::PlaybackDone), vec![]);
        assert_eq!(m.state(), TurnState::Idle);
    }
}
