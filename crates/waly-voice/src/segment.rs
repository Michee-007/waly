//! Segmentation de la parole : hystérésis au-dessus des probabilités VAD.
//! Logique pure (aucune dépendance moteur) — le wrapper Silero est dans
//! [`crate::vad`] (feature `service`).

/// Taille de trame imposée par Silero v5 à 16 kHz (512 éch. = 32 ms).
pub const FRAME: usize = 512;
/// Fréquence d'échantillonnage de travail de la cascade voix.
pub const SAMPLE_RATE: i64 = 16_000;

/// Segmenteur par hystérésis. Seuils hérités des réglages usuels Silero :
/// démarre > 0,5 ; termine quand la probabilité reste < 0,35 pendant
/// `hang_ms` (le silence de fin de tour, 800 ms en v0).
pub struct SpeechSegmenter {
    speaking: bool,
    hang_frames: u32,
    below_count: u32,
}

/// Transition détectée sur la trame courante.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SpeechEdge {
    None,
    Started,
    Ended,
}

impl SpeechSegmenter {
    pub fn new(hang_ms: u64) -> Self {
        let frame_ms = FRAME as u64 * 1000 / SAMPLE_RATE as u64; // 32 ms
        Self {
            speaking: false,
            hang_frames: (hang_ms / frame_ms).max(1) as u32,
            below_count: 0,
        }
    }

    pub fn speaking(&self) -> bool {
        self.speaking
    }

    pub fn push(&mut self, prob: f32) -> SpeechEdge {
        if self.speaking {
            if prob < 0.35 {
                self.below_count += 1;
                if self.below_count >= self.hang_frames {
                    self.speaking = false;
                    self.below_count = 0;
                    return SpeechEdge::Ended;
                }
            } else {
                self.below_count = 0;
            }
            SpeechEdge::None
        } else if prob > 0.5 {
            self.speaking = true;
            self.below_count = 0;
            SpeechEdge::Started
        } else {
            SpeechEdge::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segmenteur_hysteresis() {
        // 800 ms de retombee = 25 trames de 32 ms
        let mut seg = SpeechSegmenter::new(800);
        assert_eq!(seg.push(0.1), SpeechEdge::None);
        assert_eq!(seg.push(0.9), SpeechEdge::Started);
        // Un creux bref ne termine pas le tour
        for _ in 0..24 {
            assert_eq!(seg.push(0.1), SpeechEdge::None);
        }
        assert_eq!(seg.push(0.8), SpeechEdge::None); // reprise -> compteur remis
        for _ in 0..24 {
            assert_eq!(seg.push(0.1), SpeechEdge::None);
        }
        assert_eq!(seg.push(0.1), SpeechEdge::Ended);
        assert!(!seg.speaking());
    }
}
