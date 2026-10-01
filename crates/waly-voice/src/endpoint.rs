//! Fin de tour sémantique (chantier R1.5) : endpointing adaptatif à trois seuils.
//!
//! Constat mesuré (JOURNAL 2026-07-04) : l'endpointing v0 payait 600 ms de
//! silence fixe PUIS ~180 ms de STT, soit ~0,78 s de budget perçu avant le LLM.
//! Idée : Parakeet coûte ~0,18 s et ponctue bien → dès `fast_ms` (280 ms) de
//! silence on transcrit SPÉCULATIVEMENT l'énoncé en cours. Si le texte est
//! sémantiquement complet, on committe sans attendre : le STT a été payé
//! PENDANT le silence et disparaît du chemin critique. Trois échéances selon
//! la confiance (retour terrain 2026-07-04 : Parakeet ponctue les fragments) :
//! `?`/`!` → `fast_ms` (280 ms) ; point final → `period_ms` (500 ms) ;
//! en suspens → `hold_ms` (800 ms), plus confortable que les 600 ms fixes v0.
//!
//! Deux briques pures (zéro I/O, testables partout) :
//! - [`assess`] : heuristique de complétude d'un transcript FR/EN ;
//! - [`AdaptiveEndpointer`] : hystérésis VAD (mêmes seuils que
//!   [`crate::segment::SpeechSegmenter`]) + émission d'un jalon `Speculate`.

use crate::segment::{FRAME, SAMPLE_RATE};

/// Verdict de complétude d'un transcript partiel.
///
/// Trois niveaux depuis le retour terrain du 2026-07-04 : Parakeet ponctue
/// AUSSI les fragments (« J'ai fait une petite. ») — un point final n'est
/// donc qu'un indice, pas une preuve. Seuls `?` et `!` committent au seuil
/// court ; le point committe à un seuil intermédiaire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    /// Question/exclamation : committer au seuil court, sans ambiguïté.
    Complete,
    /// Point final : probablement complet, committer au seuil intermédiaire
    /// (le STT ponctue parfois les fragments de phrase).
    LikelyComplete,
    /// L'énoncé semble en suspens : laisser courir le filet long.
    Incomplete,
}

/// Mots qui laissent une phrase FR (ou EN) en suspens quand ils la terminent :
/// conjonctions, prépositions, déterminants, pronoms sujets, auxiliaires.
/// Liste volontairement conservatrice : un mot ambigu souvent final à l'oral
/// (« alors », « ça va », « avec nous », « j'y vais ») n'y figure PAS — le
/// pire cas d'un oubli est d'attendre le silence long (confort), alors qu'un
/// excès de zèle coupe l'utilisateur en pleine phrase (inacceptable).
const PENDING_WORDS: &[&str] = &[
    // FR — conjonctions et subordonnants
    "et", "ou", "mais", "donc", "or", "ni", "car", "que", "qu", "qui", "dont",
    "si", "quand", "lorsque", "parce", "puisque", "comme", "puis",
    // FR — prépositions
    "de", "du", "des", "à", "au", "aux", "en", "dans", "sur", "sous", "vers",
    "chez", "avec", "sans", "pour", "par", "entre", "contre",
    // FR — déterminants
    "le", "la", "les", "un", "une", "mon", "ma", "mes", "ton", "ta", "tes",
    "son", "sa", "ses", "nos", "vos", "leur", "leurs", "ce", "cet", "cette", "ces",
    // FR — pronoms sujets et clitiques (jamais finals en position sujet)
    "je", "tu", "il", "elle", "on", "ils", "elles", "j", "y",
    // FR — auxiliaires rarement finals
    "est", "sont", "suis", "ont",
    // EN — l'essentiel (produit FR & EN)
    "and", "but", "the", "a", "an", "of", "to", "in", "at", "with", "from",
    "by", "is", "are", "was", "were", "i", "you", "he", "she", "we", "they",
    "my", "your", "his", "her", "their", "this", "these", "those", "so",
    "because", "when", "that", "which", "who",
];

/// Heuristique de complétude d'un transcript (sortie Parakeet, qui ponctue).
///
/// Règles, dans l'ordre :
/// 1. vide → incomplet ;
/// 2. finit par `?` ou `!` → complet (même après un mot « pendant » :
///    « et alors ? ») ;
/// 3. points de suspension → incomplet ;
/// 4. dernier mot en suspens → incomplet (même si Parakeet a mis un point :
///    il ponctue parfois les fragments) ;
/// 5. finit par `.` → PROBABLEMENT complet (seuil intermédiaire) ;
/// 6. pas de ponctuation finale → incomplet (Parakeet ponctue bien : son
///    absence suggère un milieu de phrase).
pub fn assess(transcript: &str) -> Completeness {
    let t = transcript.trim();
    let Some(last) = t.chars().last() else {
        return Completeness::Incomplete;
    };
    if matches!(last, '?' | '!') {
        return Completeness::Complete;
    }
    if t.ends_with('…') || t.ends_with("...") {
        return Completeness::Incomplete;
    }
    let word = t.rsplit(char::is_whitespace).next().unwrap_or("");
    let word = word
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    if PENDING_WORDS.contains(&word.as_str()) {
        return Completeness::Incomplete;
    }
    if last == '.' {
        Completeness::LikelyComplete
    } else {
        Completeness::Incomplete
    }
}

/// Transition détectée sur la trame courante (sur-ensemble de
/// [`crate::segment::SpeechEdge`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointEdge {
    None,
    /// Début de parole.
    Started,
    /// `fast_ms` de silence écoulés : lancer le STT spéculatif, puis passer
    /// le verdict [`assess`] à [`AdaptiveEndpointer::commit_on`].
    Speculate,
    /// `hold_ms` de silence : fin de tour (chemin filet de sécurité).
    Ended,
}

/// Segmenteur à trois seuils. Mêmes seuils de probabilité que
/// [`crate::segment::SpeechSegmenter`] (démarre > 0,5 ; silence < 0,35).
/// Jalons : `Speculate` à `fast_ms`, puis selon le verdict du STT spéculatif
/// la fin de tour tombe à `fast_ms` (Complete), `period_ms` (LikelyComplete)
/// ou `hold_ms` (Incomplete / pas de verdict).
pub struct AdaptiveEndpointer {
    speaking: bool,
    below: u32,
    speculated: bool,
    fast_frames: u32,
    period_frames: u32,
    hold_frames: u32,
    /// Échéance de fin de tour de l'épisode de silence courant (en trames).
    deadline: u32,
    /// Silence réellement écoulé au dernier `Ended`, pour les métriques.
    last_end_ms: u64,
}

impl AdaptiveEndpointer {
    pub fn new(fast_ms: u64, period_ms: u64, hold_ms: u64) -> Self {
        let frame_ms = FRAME as u64 * 1000 / SAMPLE_RATE as u64; // 32 ms
        let hold_frames = (hold_ms / frame_ms).max(3) as u32;
        let fast_frames = ((fast_ms / frame_ms).max(1) as u32).min(hold_frames - 2);
        let period_frames =
            ((period_ms / frame_ms) as u32).clamp(fast_frames + 1, hold_frames - 1);
        Self {
            speaking: false,
            below: 0,
            speculated: false,
            fast_frames,
            period_frames,
            hold_frames,
            deadline: hold_frames,
            last_end_ms: 0,
        }
    }

    pub fn speaking(&self) -> bool {
        self.speaking
    }

    /// Silence réellement attendu au moment du dernier [`EndpointEdge::Ended`]
    /// (500 ms sur point final, 800 ms en suspens) — pour les métriques.
    pub fn last_end_silence_ms(&self) -> u64 {
        self.last_end_ms
    }

    fn frame_ms(&self) -> u64 {
        FRAME as u64 * 1000 / SAMPLE_RATE as u64
    }

    pub fn push(&mut self, prob: f32) -> EndpointEdge {
        if self.speaking {
            if prob < 0.35 {
                self.below += 1;
                if self.below >= self.deadline {
                    self.last_end_ms = self.below as u64 * self.frame_ms();
                    self.speaking = false;
                    self.below = 0;
                    self.speculated = false;
                    self.deadline = self.hold_frames;
                    return EndpointEdge::Ended;
                }
                if self.below == self.fast_frames && !self.speculated {
                    self.speculated = true;
                    return EndpointEdge::Speculate;
                }
            } else {
                // Reprise de voix : spéculation et échéance raccourcie caduques.
                self.below = 0;
                self.speculated = false;
                self.deadline = self.hold_frames;
            }
            EndpointEdge::None
        } else if prob > 0.5 {
            self.speaking = true;
            self.below = 0;
            self.speculated = false;
            self.deadline = self.hold_frames;
            EndpointEdge::Started
        } else {
            EndpointEdge::None
        }
    }

    /// À appeler avec le verdict du STT spéculatif, immédiatement après
    /// [`EndpointEdge::Speculate`]. `true` = committer le tour MAINTENANT
    /// (le transcript spéculatif EST la transcription finale). Sur
    /// `LikelyComplete`, l'échéance de l'épisode est raccourcie à `period_ms`
    /// et `push` émettra `Ended` à ce moment-là.
    pub fn commit_on(&mut self, verdict: Completeness) -> bool {
        if !(self.speaking && self.speculated) {
            return false;
        }
        match verdict {
            Completeness::Complete => {
                self.speaking = false;
                self.below = 0;
                self.speculated = false;
                self.deadline = self.hold_frames;
                true
            }
            Completeness::LikelyComplete => {
                self.deadline = self.period_frames;
                false
            }
            Completeness::Incomplete => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completude_fr() {
        use Completeness::*;
        assert_eq!(assess("Quelle heure est-il ?"), Complete);
        assert_eq!(assess("C'est parti !"), Complete);
        // Point final : probable seulement (Parakeet ponctue les fragments)
        assert_eq!(assess("Il est quatorze heures."), LikelyComplete);
        assert_eq!(assess("D'accord."), LikelyComplete);
        assert_eq!(assess("J'ai fait une petite."), LikelyComplete); // fragment vécu
        // Mots en suspens, même ponctués par le STT
        assert_eq!(assess("Mets un minuteur de dix minutes et"), Incomplete);
        assert_eq!(assess("Et."), Incomplete);
        assert_eq!(assess("Je voudrais que tu"), Incomplete);
        assert_eq!(assess("Rappelle-moi de sortir la"), Incomplete);
        // Virgule, suspension, absence de ponctuation
        assert_eq!(assess("Alors en fait,"), Incomplete);
        assert_eq!(assess("Je pense que..."), Incomplete);
        assert_eq!(assess("Attends je réfléchis"), Incomplete);
        assert_eq!(assess(""), Incomplete);
        assert_eq!(assess("   "), Incomplete);
        // Le ? l'emporte sur le mot pendant (« et alors ? »)
        assert_eq!(assess("Et alors ?"), Complete);
        // « là » (adverbe, accentué par Parakeet) n'est pas « la » (article)
        assert_eq!(assess("Je suis là."), LikelyComplete);
    }

    #[test]
    fn completude_en() {
        use Completeness::*;
        assert_eq!(assess("What time is it?"), Complete);
        assert_eq!(assess("Set a timer for"), Incomplete);
        assert_eq!(assess("I think that"), Incomplete);
        assert_eq!(assess("Sounds good."), LikelyComplete);
    }

    // 280 ms → 8 trames de 32 ms ; 500 ms → 15 ; 800 ms → 25.
    fn endpointer() -> AdaptiveEndpointer {
        AdaptiveEndpointer::new(280, 500, 800)
    }

    #[test]
    fn speculation_puis_commit_complet() {
        let mut ep = endpointer();
        assert_eq!(ep.push(0.9), EndpointEdge::Started);
        for _ in 0..7 {
            assert_eq!(ep.push(0.1), EndpointEdge::None);
        }
        assert_eq!(ep.push(0.1), EndpointEdge::Speculate);
        assert!(ep.commit_on(Completeness::Complete));
        assert!(!ep.speaking());
    }

    #[test]
    fn incomplet_attend_le_silence_long() {
        let mut ep = endpointer();
        ep.push(0.9);
        for _ in 0..7 {
            ep.push(0.1);
        }
        assert_eq!(ep.push(0.1), EndpointEdge::Speculate);
        assert!(!ep.commit_on(Completeness::Incomplete));
        assert!(ep.speaking());
        // Pas de seconde spéculation dans le même épisode de silence
        for _ in 9..25 {
            assert_eq!(ep.push(0.1), EndpointEdge::None);
        }
        assert_eq!(ep.push(0.1), EndpointEdge::Ended);
        assert!(!ep.speaking());
        assert_eq!(ep.last_end_silence_ms(), 800);
    }

    #[test]
    fn point_final_committe_au_seuil_intermediaire() {
        let mut ep = endpointer();
        ep.push(0.9);
        for _ in 0..7 {
            ep.push(0.1);
        }
        assert_eq!(ep.push(0.1), EndpointEdge::Speculate);
        // Point final : pas de commit immediat, echeance raccourcie a 500 ms
        assert!(!ep.commit_on(Completeness::LikelyComplete));
        assert!(ep.speaking());
        for _ in 9..15 {
            assert_eq!(ep.push(0.1), EndpointEdge::None);
        }
        assert_eq!(ep.push(0.1), EndpointEdge::Ended);
        assert_eq!(ep.last_end_silence_ms(), 480); // 15 trames de 32 ms
    }

    #[test]
    fn reprise_annule_l_echeance_raccourcie() {
        let mut ep = endpointer();
        ep.push(0.9);
        for _ in 0..8 {
            ep.push(0.1);
        }
        ep.commit_on(Completeness::LikelyComplete);
        // L'utilisateur continue : l'échéance 500 ms redevient 800 ms
        assert_eq!(ep.push(0.8), EndpointEdge::None);
        for _ in 0..7 {
            assert_eq!(ep.push(0.1), EndpointEdge::None);
        }
        assert_eq!(ep.push(0.1), EndpointEdge::Speculate);
        assert!(!ep.commit_on(Completeness::Incomplete));
        for _ in 9..25 {
            assert_eq!(ep.push(0.1), EndpointEdge::None);
        }
        assert_eq!(ep.push(0.1), EndpointEdge::Ended);
        assert_eq!(ep.last_end_silence_ms(), 800);
    }

    #[test]
    fn reprise_de_voix_annule_la_speculation() {
        let mut ep = endpointer();
        ep.push(0.9);
        for _ in 0..8 {
            ep.push(0.1);
        }
        // L'utilisateur continue sa phrase : compteur et spéculation remis
        assert_eq!(ep.push(0.8), EndpointEdge::None);
        assert!(ep.speaking());
        for _ in 0..7 {
            assert_eq!(ep.push(0.1), EndpointEdge::None);
        }
        // Nouvelle spéculation possible dans le nouvel épisode
        assert_eq!(ep.push(0.1), EndpointEdge::Speculate);
    }

    #[test]
    fn commit_refuse_hors_episode() {
        let mut ep = endpointer();
        // Jamais parlé : rien à committer
        assert!(!ep.commit_on(Completeness::Complete));
        ep.push(0.9);
        // Pas encore de spéculation en cours
        assert!(!ep.commit_on(Completeness::Complete));
    }

    #[test]
    fn fast_reste_sous_hold() {
        // fast >= hold : clampé sous le seuil long
        let mut ep = AdaptiveEndpointer::new(900, 950, 800);
        ep.push(0.9);
        let mut saw_speculate = false;
        for _ in 0..24 {
            if ep.push(0.1) == EndpointEdge::Speculate {
                saw_speculate = true;
            }
        }
        assert!(saw_speculate);
        assert_eq!(ep.push(0.1), EndpointEdge::Ended);
    }
}
