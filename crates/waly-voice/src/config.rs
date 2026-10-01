//! Configuration de la cascade voix.

use serde::{Deserialize, Serialize};

/// Configuration du service voix. Tous les défauts correspondent aux mesures
/// et décisions de R0/R1 sur la machine de référence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    /// Endpoint OpenAI-compat du STT (FLM en mode ASR standalone :
    /// `flm serve --asr 1` — en v0.9.43 le endpoint /v1/audio/transcriptions
    /// ne fonctionne PAS quand un LLM est chargé dans le même processus).
    pub stt_endpoint: String,
    /// Endpoint OpenAI-compat du LLM (second processus FLM, port distinct).
    pub llm_endpoint: String,
    /// Modèle LLM du dialogue. Depuis R4 : le cerveau unique texte+vision
    /// (`waly_core::llm::modele_par_defaut()`, surcharge WALY_MODEL) —
    /// qwen3vl-it:4b, TTFT/décodage mesurés identiques à qwen3-it:4b.
    pub llm_model: String,
    /// Silence LONG de fin de tour, en millisecondes : filet de sécurité de
    /// l'endpointing sémantique (R1.5), payé seulement quand le transcript
    /// spéculatif semble en suspens (`endpoint::assess` → Incomplete).
    /// 800 ms : confort pour « je réfléchis en milieu de phrase » — le cas
    /// complet committe bien avant (voir `endpoint_fast_ms`). Le compromis
    /// v0 sans sémantique était 600 ms.
    pub end_of_turn_silence_ms: u64,
    /// Silence COURT avant le STT spéculatif (fin de tour sémantique R1.5).
    /// À ce point Parakeet transcrit l'énoncé en cours (~0,18 s, payé PENDANT
    /// le silence) ; un `?`/`!` final committe immédiatement : ~0,46 s
    /// effectifs au lieu de 0,78 s (600 ms + STT) en v0.
    pub endpoint_fast_ms: u64,
    /// Silence INTERMÉDIAIRE pour un transcript à point final. Retour terrain
    /// 2026-07-04 : Parakeet ponctue AUSSI les fragments (« J'ai fait une
    /// petite. ») — le point ne committe donc pas au seuil court, on laisse
    /// une marge de reprise avant de conclure.
    pub endpoint_period_ms: u64,
    /// Fréquence d'échantillonnage du micro (Hz). Silero VAD et Parakeet
    /// (STT primaire) travaillent à 16 kHz.
    pub sample_rate: u32,
    /// Nom (PAS l'index) du périphérique d'entrée. `None` = re-scanner au
    /// lancement et choisir le défaut système. Piège documenté : l'index 5
    /// était mort sur la machine de référence (dérive audio) — ne JAMAIS
    /// coder un index en dur.
    pub input_device_name: Option<String>,
    /// Voix TTS. Exigence produit (retour Michée 2026-07-03) : l'utilisateur
    /// doit pouvoir choisir parmi plusieurs voix — ce champ est le premier pas
    /// (catalogue Piper FR : upmc/siwis/tom… ; plus tard Pocket TTS et son
    /// clonage de voix). Défaut : upmc, préférée à l'écoute.
    pub tts_voice: String,
    /// Locuteur dans les modèles multi-voix. upmc en contient deux :
    /// 0 = jessica, 1 = pierre. ⚠ Depuis le verdict R-V (Michée,
    /// 2026-07-20), Piper n'est plus que le moteur de SECOURS
    /// (`WALY_TTS=piper`) — la voix de Waly est Pocket french_24l
    /// (fabien / développeuse par clonage). Le canal `WALY_TTS_SPEAKER=0`
    /// signifie « féminine » pour les DEUX moteurs.
    pub tts_speaker: i32,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            stt_endpoint: "http://127.0.0.1:52625/v1".into(),
            // Même contrat que waly_core::llm::port_par_defaut() : 42626 hors
            // de la zone dynamique Windows (WinNAT y réserve des plages),
            // surcharge WALY_LLM_PORT.
            llm_endpoint: format!(
                "http://127.0.0.1:{}/v1",
                std::env::var("WALY_LLM_PORT")
                    .ok()
                    .and_then(|p| p.trim().parse::<u16>().ok())
                    .unwrap_or(42626)
            ),
            // Même contrat que waly_core::llm::modele_par_defaut() (waly-core
            // est une dépendance OPTIONNELLE de cette lib — pas d'appel direct).
            llm_model: std::env::var("WALY_MODEL")
                .unwrap_or_else(|_| "qwen3vl-it:4b".into()),
            end_of_turn_silence_ms: 800,
            endpoint_fast_ms: 280,
            endpoint_period_ms: 500,
            sample_rate: 16_000,
            input_device_name: None,
            tts_voice: "fr_FR-upmc-medium".into(),
            tts_speaker: 1,
        }
    }
}

impl VoiceConfig {
    /// Vérifie la cohérence des seuils d'endpointing : l'échelle
    /// spéculatif ≤ intermédiaire ≤ filet long est un invariant de
    /// `endpoint::AdaptiveEndpointer` — une config chargée d'un fichier qui
    /// le viole produirait des fins de tour incohérentes en silence.
    pub fn validate(&self) -> Result<(), String> {
        if self.endpoint_fast_ms > self.endpoint_period_ms {
            return Err(format!(
                "endpoint_fast_ms ({}) > endpoint_period_ms ({})",
                self.endpoint_fast_ms, self.endpoint_period_ms
            ));
        }
        if self.endpoint_period_ms > self.end_of_turn_silence_ms {
            return Err(format!(
                "endpoint_period_ms ({}) > end_of_turn_silence_ms ({})",
                self.endpoint_period_ms, self.end_of_turn_silence_ms
            ));
        }
        if self.sample_rate != 16_000 {
            return Err(format!(
                "sample_rate {} non supporte (Silero et Parakeet exigent 16000)",
                self.sample_rate
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defauts_coherents_et_serialisables() {
        let cfg = VoiceConfig::default();
        cfg.validate().expect("les defauts doivent etre valides");
        // Aller-retour serde : un champ ajoute sans default casserait ici.
        let json = serde_json::to_string(&cfg).unwrap();
        let back: VoiceConfig = serde_json::from_str(&json).unwrap();
        back.validate().unwrap();
        // Champs absents -> defauts (serde(default)).
        let partial: VoiceConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(partial.tts_speaker, cfg.tts_speaker);
    }

    #[test]
    fn validation_rejette_les_seuils_incoherents() {
        let cfg = VoiceConfig { endpoint_fast_ms: 900, ..VoiceConfig::default() };
        assert!(cfg.validate().is_err());
        let cfg = VoiceConfig { sample_rate: 44_100, ..VoiceConfig::default() };
        assert!(cfg.validate().is_err());
    }
}
