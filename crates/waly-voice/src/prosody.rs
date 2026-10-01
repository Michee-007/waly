//! Prosodie émotionnelle (clôture R1.5 voix).
//!
//! Piper/VITS n'a pas de contrôle d'émotion explicite, mais trois leviers
//! réels existent et se combinent :
//! 1. **la ponctuation traverse le phonémiseur** (espeak-ng) : « ? » produit
//!    une intonation montante, « ! » une emphase — d'où l'importance de la
//!    préserver (fait par `clean_for_tts`) et d'encourager le LLM à ponctuer
//!    expressivement (prompt système) ;
//! 2. **la vitesse par clause** (paramètre `speed` de la synthèse) :
//!    l'excitation accélère, la réflexion ralentit ;
//! 3. **les pauses** avant/après la clause, insérées par le lecteur : une
//!    respiration avant une pensée, un temps après une question.
//!
//! Ce module ANALYSE la clause BRUTE (avant `clean_for_tts` : les emojis du
//! LLM — 😊 🤔 🎉 — sont des indices d'émotion précieux avant d'être retirés
//! du texte vocalisé) et rend les réglages à appliquer.

/// Humeur détectée d'une clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    /// Affirmation neutre.
    Neutral,
    /// Question : intonation montante (espeak), léger temps après.
    Question,
    /// Enthousiasme, fierté, joie : débit plus vif.
    Excited,
    /// Réflexion, hésitation : débit posé, respirations.
    Pensive,
}

/// Réglages de prosodie d'une clause.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prosody {
    pub mood: Mood,
    /// Paramètre `speed` de la synthèse (1.0 = normal, >1 plus rapide).
    pub speed: f32,
    /// Silence à insérer AVANT la clause (ms).
    pub pre_pause_ms: u32,
    /// Silence à insérer APRÈS la clause (ms).
    pub post_pause_ms: u32,
}

impl Prosody {
    fn of(mood: Mood) -> Self {
        // Écarts de vitesse MODÉRÉS : à 1,1 Piper avale des syllabes
        // (retour d'écoute 2026-07-05) — l'émotion doit s'entendre sans
        // coûter l'intelligibilité.
        match mood {
            Mood::Neutral => Self { mood, speed: 1.0, pre_pause_ms: 0, post_pause_ms: 120 },
            Mood::Question => Self { mood, speed: 0.98, pre_pause_ms: 0, post_pause_ms: 220 },
            Mood::Excited => Self { mood, speed: 1.06, pre_pause_ms: 0, post_pause_ms: 100 },
            Mood::Pensive => Self { mood, speed: 0.93, pre_pause_ms: 140, post_pause_ms: 300 },
        }
    }
}

/// Emojis « joie/enthousiasme » les plus émis par les LLM.
const EXCITED_EMOJI: &[char] = &[
    '😀', '😃', '😄', '😁', '😆', '😊', '🤩', '😍', '🥳', '🎉', '🔥', '💪', '👏', '✨', '🌞',
    '🚀', '😜', '😝',
];
/// Emojis « réflexion ».
const PENSIVE_EMOJI: &[char] = &['🤔', '😌', '💭', '🧐', '😶'];

/// Indices lexicaux d'enthousiasme (français oral, minuscules).
const EXCITED_WORDS: &[&str] = &[
    "génial", "genial", "super", "bravo", "incroyable", "excellent", "magnifique",
    "fantastique", "formidable", "félicitations", "felicitations", "trop bien", "haha",
    "waouh", "wow", "quelle bonne", "fier de", "fière de", "fiere de", "j'adore",
];
/// Indices lexicaux de réflexion/hésitation.
const PENSIVE_WORDS: &[&str] = &[
    "hmm", "hum", "peut-être", "peut-etre", "je me demande", "voyons", "réfléchis",
    "reflechis", "réfléchir", "reflechir", "pas sûr", "pas sur", "pas certain",
    "difficile à dire", "difficile a dire", "laisse-moi penser", "bonne question",
];

/// Analyse une clause BRUTE (emojis et ponctuation d'origine inclus).
pub fn analyze(clause: &str) -> Prosody {
    let lower = clause.to_lowercase();
    let has = |set: &[&str]| set.iter().any(|w| lower.contains(w));
    let has_emoji = |set: &[char]| clause.chars().any(|c| set.contains(&c));

    let bang = clause.contains('!');
    let question = clause.contains('?');
    let ellipsis = clause.contains('…') || clause.contains("...");

    // Priorités : l'exclamation l'emporte (même « ?! » est de l'excitation,
    // espeak garde l'intonation montante du « ? » de toute façon) ; puis la
    // question ; puis les marqueurs de réflexion ; puis l'enthousiasme sans
    // point d'exclamation (« c'est génial. »).
    let mood = if bang || has_emoji(EXCITED_EMOJI) {
        Mood::Excited
    } else if question {
        Mood::Question
    } else if ellipsis || has(PENSIVE_WORDS) || has_emoji(PENSIVE_EMOJI) {
        Mood::Pensive
    } else if has(EXCITED_WORDS) {
        Mood::Excited
    } else {
        Mood::Neutral
    };
    Prosody::of(mood)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions() {
        assert_eq!(analyze("Comment ça va ?").mood, Mood::Question);
        assert!(analyze("Tu veux essayer ?").speed < 1.0);
    }

    #[test]
    fn exclamations_et_emojis() {
        assert_eq!(analyze("C'est génial !").mood, Mood::Excited);
        assert_eq!(analyze("Bienvenue 🎉").mood, Mood::Excited);
        assert_eq!(analyze("Je suis fier de toi.").mood, Mood::Excited);
        // ?! = excitation, pas simple question
        assert_eq!(analyze("Tu te rends compte ?!").mood, Mood::Excited);
        assert!(analyze("Bravo !").speed > 1.0);
    }

    #[test]
    fn reflexion() {
        assert_eq!(analyze("Hmm, laisse-moi réfléchir…").mood, Mood::Pensive);
        assert_eq!(analyze("C'est difficile à dire.").mood, Mood::Pensive);
        assert_eq!(analyze("Peut-être demain.").mood, Mood::Pensive);
        assert_eq!(analyze("Bonne question 🤔").mood, Mood::Pensive);
        let p = analyze("Voyons voir...");
        assert!(p.speed < 1.0 && p.pre_pause_ms > 0);
    }

    #[test]
    fn neutre() {
        let p = analyze("Il est quatorze heures.");
        assert_eq!(p.mood, Mood::Neutral);
        assert_eq!(p.speed, 1.0);
    }
}
