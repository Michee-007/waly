//! TTS Piper (VITS) via sherpa-onnx-c-api.dll — même DLL et même stratégie
//! que le STT ([`crate::stt`]) : FFI chargée à l'exécution, zéro C++.
//!
//! ⚠ Structs répliqués de c-api.h **v1.13.3** — re-vérifier à chaque mise à
//! jour de la DLL. Les structs Matcha/Kokoro/Pocket/Supertonic ne servent
//! qu'au layout ABI de `TtsModelConfig` : NE PAS les supprimer. (Le TTS
//! Pocket, lui, passe par le pipeline natif `crate::pocket` — la DLL ne sait
//! pas exécuter le schéma 2 `french_24l`.)

use std::ffi::{c_char, c_float, c_void, CString};

#[repr(C)]
#[derive(Clone, Copy)]
struct TtsVitsModelConfig {
    model: *const c_char,
    lexicon: *const c_char,
    tokens: *const c_char,
    data_dir: *const c_char,
    noise_scale: c_float,
    noise_scale_w: c_float,
    length_scale: c_float,
    dict_dir: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TtsMatchaModelConfig {
    acoustic_model: *const c_char,
    vocoder: *const c_char,
    lexicon: *const c_char,
    tokens: *const c_char,
    data_dir: *const c_char,
    noise_scale: c_float,
    length_scale: c_float,
    dict_dir: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TtsKokoroModelConfig {
    model: *const c_char,
    voices: *const c_char,
    tokens: *const c_char,
    data_dir: *const c_char,
    length_scale: c_float,
    dict_dir: *const c_char,
    lexicon: *const c_char,
    lang: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TtsKittenModelConfig {
    model: *const c_char,
    voices: *const c_char,
    tokens: *const c_char,
    data_dir: *const c_char,
    length_scale: c_float,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TtsZipvoiceModelConfig {
    tokens: *const c_char,
    encoder: *const c_char,
    decoder: *const c_char,
    vocoder: *const c_char,
    data_dir: *const c_char,
    lexicon: *const c_char,
    feat_scale: c_float,
    t_shift: c_float,
    target_rms: c_float,
    guidance_scale: c_float,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TtsPocketModelConfig {
    lm_flow: *const c_char,
    lm_main: *const c_char,
    encoder: *const c_char,
    decoder: *const c_char,
    text_conditioner: *const c_char,
    vocab_json: *const c_char,
    token_scores_json: *const c_char,
    voice_embedding_cache_capacity: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TtsSupertonicModelConfig {
    duration_predictor: *const c_char,
    text_encoder: *const c_char,
    vector_estimator: *const c_char,
    vocoder: *const c_char,
    tts_json: *const c_char,
    unicode_indexer: *const c_char,
    voice_style: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TtsModelConfig {
    vits: TtsVitsModelConfig,
    num_threads: i32,
    debug: i32,
    provider: *const c_char,
    matcha: TtsMatchaModelConfig,
    kokoro: TtsKokoroModelConfig,
    kitten: TtsKittenModelConfig,
    zipvoice: TtsZipvoiceModelConfig,
    pocket: TtsPocketModelConfig,
    supertonic: TtsSupertonicModelConfig,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TtsConfig {
    model: TtsModelConfig,
    rule_fsts: *const c_char,
    max_num_sentences: i32,
    rule_fars: *const c_char,
    silence_scale: c_float,
}

#[repr(C)]
struct GeneratedAudio {
    samples: *const c_float,
    n: i32,
    sample_rate: i32,
}

type TtsPtr = *const c_void;
type FnCreateTts = unsafe extern "C" fn(*const TtsConfig) -> TtsPtr;
type FnDestroyTts = unsafe extern "C" fn(TtsPtr);
type FnTtsSampleRate = unsafe extern "C" fn(TtsPtr) -> i32;
type FnTtsGenerate =
    unsafe extern "C" fn(TtsPtr, *const c_char, i32, c_float) -> *const GeneratedAudio;
type FnDestroyAudio = unsafe extern "C" fn(*const GeneratedAudio);

/// Synthèse Piper (VITS). Voix par défaut du produit : fr_FR-upmc-medium
/// (choix Michée, cf. VoiceConfig::tts_voice).
pub struct PiperTts {
    tts: TtsPtr,
    destroy_tts: FnDestroyTts,
    generate: FnTtsGenerate,
    destroy_audio: FnDestroyAudio,
    sample_rate: u32,
    /// Identifiant de locuteur pour les modèles multi-voix
    /// (ex. fr_FR-upmc-medium : 0 = jessica, 1 = pierre).
    speaker: i32,
    _lib: libloading::Library,
}

impl PiperTts {
    /// `model_dir` doit contenir `<voice>.onnx`, `tokens.txt`,
    /// `espeak-ng-data/` (archive vits-piper-*).
    pub fn new(
        dll_path: &str,
        model_dir: &str,
        voice: &str,
        num_threads: i32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        #[cfg(windows)]
        let lib: libloading::Library = unsafe {
            libloading::os::windows::Library::load_with_flags(
                dll_path,
                libloading::os::windows::LOAD_WITH_ALTERED_SEARCH_PATH,
            )?
            .into()
        };
        #[cfg(not(windows))]
        let lib: libloading::Library = unsafe { libloading::Library::new(dll_path)? };

        unsafe {
            let create: FnCreateTts = *lib.get(b"SherpaOnnxCreateOfflineTts\0")?;
            let destroy_tts: FnDestroyTts = *lib.get(b"SherpaOnnxDestroyOfflineTts\0")?;
            let sample_rate_fn: FnTtsSampleRate = *lib.get(b"SherpaOnnxOfflineTtsSampleRate\0")?;
            let generate: FnTtsGenerate = *lib.get(b"SherpaOnnxOfflineTtsGenerate\0")?;
            let destroy_audio: FnDestroyAudio =
                *lib.get(b"SherpaOnnxDestroyOfflineTtsGeneratedAudio\0")?;

            let model = CString::new(format!("{model_dir}/{voice}.onnx"))?;
            let tokens = CString::new(format!("{model_dir}/tokens.txt"))?;
            let data_dir = CString::new(format!("{model_dir}/espeak-ng-data"))?;
            let provider = CString::new("cpu")?;

            let mut config: TtsConfig = std::mem::zeroed();
            config.model.vits.model = model.as_ptr();
            config.model.vits.tokens = tokens.as_ptr();
            config.model.vits.data_dir = data_dir.as_ptr();
            // noise_scale = vivacite de la prosodie, noise_scale_w =
            // variation des durees de phonemes (0.0 = defauts internes faux :
            // sherpa attend des valeurs explicites ; usuels : 0,667/0,8).
            // Defauts 0,5/0,6 : variante « sage » choisie a l'ecoute par
            // Michee (2026-07-05) pour pierre — plus posee, mieux articulee.
            let noise = std::env::var("WALY_PIPER_NOISE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.5);
            let noise_w = std::env::var("WALY_PIPER_NOISE_W")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.6);
            config.model.vits.noise_scale = noise;
            config.model.vits.noise_scale_w = noise_w;
            config.model.vits.length_scale = 1.0;
            config.model.num_threads = num_threads;
            config.model.provider = provider.as_ptr();
            config.max_num_sentences = 1; // clause par clause : pas de refente interne

            let tts = create(&config);
            if tts.is_null() {
                return Err(format!("SherpaOnnxCreateOfflineTts a echoue ({model_dir})").into());
            }
            let sample_rate = sample_rate_fn(tts) as u32;
            Ok(Self { tts, destroy_tts, generate, destroy_audio, sample_rate, speaker: 0, _lib: lib })
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Choisit le locuteur (modèles multi-voix ; 0 par défaut).
    pub fn set_speaker(&mut self, sid: i32) {
        self.speaker = sid;
    }

    /// Synthétise un texte (déjà nettoyé par [`crate::sanitize::clean_for_tts`]).
    pub fn synth(&self, text: &str, speed: f32) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        let c_text = CString::new(text)?;
        unsafe {
            let audio = (self.generate)(self.tts, c_text.as_ptr(), self.speaker, speed);
            if audio.is_null() {
                return Err("SherpaOnnxOfflineTtsGenerate a echoue".into());
            }
            let n = (*audio).n as usize;
            let samples = std::slice::from_raw_parts((*audio).samples, n).to_vec();
            (self.destroy_audio)(audio);
            Ok(samples)
        }
    }
}

impl Drop for PiperTts {
    fn drop(&mut self) {
        unsafe { (self.destroy_tts)(self.tts) };
    }
}
