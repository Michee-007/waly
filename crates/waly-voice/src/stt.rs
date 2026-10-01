//! STT Parakeet-TDT via sherpa-onnx-c-api.dll chargée à l'exécution.
//!
//! Pourquoi ce chemin : SAC bloque nos EXE release non signés mais LAISSE
//! CHARGER les DLL non signées (mesuré 2026-07-03). La DLL prébuilt officielle
//! sherpa-onnx nous donne Parakeet (STT) sans compiler une ligne de C++.
//!
//! ⚠ Les structs `#[repr(C)]` ci-dessous répliquent EXACTEMENT
//! `include/sherpa-onnx/c-api/c-api.h` de la version **v1.13.3**
//! (`C:\waly\engines\sherpa\`). Toute mise à jour de la DLL impose de
//! re-vérifier ces layouts champ par champ — un décalage = crash ou config
//! silencieusement fausse.

use std::ffi::{c_char, c_float, c_void, CStr, CString};

// ── Répliques exactes des structs C (v1.13.3) ───────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
struct FeatureConfig {
    sample_rate: i32,
    feature_dim: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineTransducerModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
    joiner: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineParaformerModelConfig {
    model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineNemoEncDecCtcModelConfig {
    model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineWhisperModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
    language: *const c_char,
    task: *const c_char,
    tail_paddings: i32,
    enable_token_timestamps: i32,
    enable_segment_timestamps: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineTdnnModelConfig {
    model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineSenseVoiceModelConfig {
    model: *const c_char,
    language: *const c_char,
    use_itn: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineMoonshineModelConfig {
    preprocessor: *const c_char,
    encoder: *const c_char,
    uncached_decoder: *const c_char,
    cached_decoder: *const c_char,
    merged_decoder: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineFireRedAsrModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineDolphinModelConfig {
    model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineZipformerCtcModelConfig {
    model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineCanaryModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
    src_lang: *const c_char,
    tgt_lang: *const c_char,
    use_pnc: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineWenetCtcModelConfig {
    model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineOmnilingualAsrCtcModelConfig {
    model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineMedAsrCtcModelConfig {
    model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineFunASRNanoModelConfig {
    encoder_adaptor: *const c_char,
    llm: *const c_char,
    embedding: *const c_char,
    tokenizer: *const c_char,
    system_prompt: *const c_char,
    user_prompt: *const c_char,
    max_new_tokens: i32,
    temperature: c_float,
    top_p: c_float,
    seed: i32,
    language: *const c_char,
    itn: i32,
    hotwords: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineFireRedAsrCtcModelConfig {
    model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineQwen3ASRModelConfig {
    conv_frontend: *const c_char,
    encoder: *const c_char,
    decoder: *const c_char,
    tokenizer: *const c_char,
    max_total_len: i32,
    max_new_tokens: i32,
    temperature: c_float,
    top_p: c_float,
    seed: i32,
    hotwords: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineCohereTranscribeModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
    language: *const c_char,
    use_punct: i32,
    use_itn: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineModelConfig {
    transducer: OfflineTransducerModelConfig,
    paraformer: OfflineParaformerModelConfig,
    nemo_ctc: OfflineNemoEncDecCtcModelConfig,
    whisper: OfflineWhisperModelConfig,
    tdnn: OfflineTdnnModelConfig,
    tokens: *const c_char,
    num_threads: i32,
    debug: i32,
    provider: *const c_char,
    model_type: *const c_char,
    modeling_unit: *const c_char,
    bpe_vocab: *const c_char,
    telespeech_ctc: *const c_char,
    sense_voice: OfflineSenseVoiceModelConfig,
    moonshine: OfflineMoonshineModelConfig,
    fire_red_asr: OfflineFireRedAsrModelConfig,
    dolphin: OfflineDolphinModelConfig,
    zipformer_ctc: OfflineZipformerCtcModelConfig,
    canary: OfflineCanaryModelConfig,
    wenet_ctc: OfflineWenetCtcModelConfig,
    omnilingual: OfflineOmnilingualAsrCtcModelConfig,
    medasr: OfflineMedAsrCtcModelConfig,
    funasr_nano: OfflineFunASRNanoModelConfig,
    fire_red_asr_ctc: OfflineFireRedAsrCtcModelConfig,
    qwen3_asr: OfflineQwen3ASRModelConfig,
    cohere_transcribe: OfflineCohereTranscribeModelConfig,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineLMConfig {
    model: *const c_char,
    scale: c_float,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct HomophoneReplacerConfig {
    dict_dir: *const c_char,
    lexicon: *const c_char,
    rule_fsts: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OfflineRecognizerConfig {
    feat_config: FeatureConfig,
    model_config: OfflineModelConfig,
    lm_config: OfflineLMConfig,
    decoding_method: *const c_char,
    max_active_paths: i32,
    hotwords_file: *const c_char,
    hotwords_score: c_float,
    rule_fsts: *const c_char,
    rule_fars: *const c_char,
    blank_penalty: c_float,
    hr: HomophoneReplacerConfig,
}

// Handles opaques
type RecognizerPtr = *const c_void;
type StreamPtr = *const c_void;

// ── Signatures des fonctions chargées dynamiquement ─────────────────────────

type FnCreateRecognizer = unsafe extern "C" fn(*const OfflineRecognizerConfig) -> RecognizerPtr;
type FnDestroyRecognizer = unsafe extern "C" fn(RecognizerPtr);
type FnCreateStream = unsafe extern "C" fn(RecognizerPtr) -> StreamPtr;
type FnDestroyStream = unsafe extern "C" fn(StreamPtr);
type FnAcceptWaveform = unsafe extern "C" fn(StreamPtr, i32, *const c_float, i32);
type FnDecodeStream = unsafe extern "C" fn(RecognizerPtr, StreamPtr);
type FnResultAsJson = unsafe extern "C" fn(StreamPtr) -> *const c_char;
type FnDestroyResultJson = unsafe extern "C" fn(*const c_char);

/// STT Parakeet (transducteur NeMo) au-dessus de la DLL sherpa-onnx.
pub struct ParakeetStt {
    // L'ordre des champs compte : le recognizer doit mourir avant la lib.
    rec: RecognizerPtr,
    destroy_recognizer: FnDestroyRecognizer,
    create_stream: FnCreateStream,
    destroy_stream: FnDestroyStream,
    accept_waveform: FnAcceptWaveform,
    decode_stream: FnDecodeStream,
    result_as_json: FnResultAsJson,
    destroy_result_json: FnDestroyResultJson,
    _lib: libloading::Library,
}

impl ParakeetStt {
    /// Charge la DLL puis le modèle. `model_dir` doit contenir
    /// `encoder.int8.onnx`, `decoder.int8.onnx`, `joiner.int8.onnx`,
    /// `tokens.txt` (archive sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8).
    pub fn new(
        dll_path: &str,
        model_dir: &str,
        num_threads: i32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // LOAD_WITH_ALTERED_SEARCH_PATH : la onnxruntime.dll voisine de la
        // DLL sherpa doit etre resolue depuis SON dossier.
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
            let create: FnCreateRecognizer =
                *lib.get(b"SherpaOnnxCreateOfflineRecognizer\0")?;
            let destroy_recognizer: FnDestroyRecognizer =
                *lib.get(b"SherpaOnnxDestroyOfflineRecognizer\0")?;
            let create_stream: FnCreateStream = *lib.get(b"SherpaOnnxCreateOfflineStream\0")?;
            let destroy_stream: FnDestroyStream = *lib.get(b"SherpaOnnxDestroyOfflineStream\0")?;
            let accept_waveform: FnAcceptWaveform =
                *lib.get(b"SherpaOnnxAcceptWaveformOffline\0")?;
            let decode_stream: FnDecodeStream = *lib.get(b"SherpaOnnxDecodeOfflineStream\0")?;
            let result_as_json: FnResultAsJson =
                *lib.get(b"SherpaOnnxGetOfflineStreamResultAsJson\0")?;
            let destroy_result_json: FnDestroyResultJson =
                *lib.get(b"SherpaOnnxDestroyOfflineStreamResultJson\0")?;

            // Les CString doivent survivre jusqu'au retour de create().
            let encoder = CString::new(format!("{model_dir}/encoder.int8.onnx"))?;
            let decoder = CString::new(format!("{model_dir}/decoder.int8.onnx"))?;
            let joiner = CString::new(format!("{model_dir}/joiner.int8.onnx"))?;
            let tokens = CString::new(format!("{model_dir}/tokens.txt"))?;
            let provider = CString::new("cpu")?;
            let model_type = CString::new("nemo_transducer")?;
            let decoding = CString::new("greedy_search")?;

            // « Zero-initialize this struct before use » (doc officielle) :
            // uniquement des pointeurs (NULL) et des entiers/flottants (0).
            let mut config: OfflineRecognizerConfig = std::mem::zeroed();
            config.feat_config = FeatureConfig { sample_rate: 16_000, feature_dim: 80 };
            config.model_config.transducer = OfflineTransducerModelConfig {
                encoder: encoder.as_ptr(),
                decoder: decoder.as_ptr(),
                joiner: joiner.as_ptr(),
            };
            config.model_config.tokens = tokens.as_ptr();
            config.model_config.num_threads = num_threads;
            config.model_config.provider = provider.as_ptr();
            config.model_config.model_type = model_type.as_ptr();
            config.decoding_method = decoding.as_ptr();

            let rec = create(&config);
            if rec.is_null() {
                return Err(format!(
                    "SherpaOnnxCreateOfflineRecognizer a echoue (modele: {model_dir})"
                )
                .into());
            }
            Ok(Self {
                rec,
                destroy_recognizer,
                create_stream,
                destroy_stream,
                accept_waveform,
                decode_stream,
                result_as_json,
                destroy_result_json,
                _lib: lib,
            })
        }
    }

    /// Transcrit un énoncé complet (mono, f32 [-1,1]).
    pub fn transcribe(
        &self,
        samples: &[f32],
        sample_rate: i32,
    ) -> Result<String, Box<dyn std::error::Error>> {
        unsafe {
            let stream = (self.create_stream)(self.rec);
            if stream.is_null() {
                return Err("SherpaOnnxCreateOfflineStream a echoue".into());
            }
            (self.accept_waveform)(stream, sample_rate, samples.as_ptr(), samples.len() as i32);
            (self.decode_stream)(self.rec, stream);
            let json_ptr = (self.result_as_json)(stream);
            let text = if json_ptr.is_null() {
                String::new()
            } else {
                let json = CStr::from_ptr(json_ptr).to_string_lossy().into_owned();
                (self.destroy_result_json)(json_ptr);
                serde_json::from_str::<serde_json::Value>(&json)
                    .ok()
                    .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_owned))
                    .unwrap_or(json)
            };
            (self.destroy_stream)(stream);
            Ok(text.trim().to_owned())
        }
    }
}

impl Drop for ParakeetStt {
    fn drop(&mut self) {
        unsafe { (self.destroy_recognizer)(self.rec) };
    }
}
