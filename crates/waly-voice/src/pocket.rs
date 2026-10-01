//! Pocket TTS (Kyutai) en NATIF sur ONNX Runtime — pipeline maison.
//!
//! Pourquoi maison : les bundles récents (schéma 2, dont le SEUL français
//! existant `french_24l`) exigent `insert_bos_before_voice` que sherpa-onnx
//! ne gère pas (v1.13.3 ET master, vérifié le 2026-07-04) → EOS prématuré.
//! Implémentation de référence : `pocket_tts_onnx.py` du dépôt HF
//! KevinAHM/pocket-tts-onnx (pipeline reproduit pas à pas ici).
//!
//! Pipeline par clause :
//! 1. voix : `mimi_encoder(audio 24 kHz)` → embeddings [1,T,1024], BOS
//!    préfixé (schéma 2), conditionnement dans `flow_lm_main` → état voix
//!    (réutilisé tel quel à chaque synthèse, jamais muté par ORT) ;
//! 2. texte : SentencePiece unigram maison (Viterbi sur vocab.json +
//!    token_scores.json, fallback octets `<0xXX>`) → `text_conditioner` →
//!    conditionnement dans `flow_lm_main` ;
//! 3. boucle : `flow_lm_main(latent précédent)` → conditioning + eos_logit
//!    (fin si > -4, + `frames_after_eos` trames), bruit N(0, √température),
//!    intégration d'Euler via `flow_lm_flow` (`lsd_steps` pas) → latent [32] ;
//! 4. `mimi_decoder` par chunks de 15 trames (12,5 Hz) → audio 24 kHz.
//!
//! Les états récurrents (74 tenseurs pour le LM 24 couches, ~200 Mo f32)
//! circulent en `DynValue` PAR PROPRIÉTÉ (`SessionOutputs::remove`) : zéro
//! copie par trame — ORT ne mute jamais ses entrées, donc l'état voix se
//! passe par vue (`Value::view`) sans clonage.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use ort::session::{Session, SessionInputValue};
use ort::value::{DynValue, Tensor};
use serde::Deserialize;

type Err = Box<dyn std::error::Error>;

// ── Métadonnées du bundle ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct StateEntry {
    dtype: String,
    fill: String,
    input_name: String,
    output_name: String,
    shape: Vec<usize>,
    /// Module d'origine (« transformer.layers.0.self_attn ») et clef
    /// (« cache », « step »…) : adresses des tenseurs dans les fichiers
    /// d'état de voix .safetensors officiels.
    #[serde(default)]
    module: String,
    #[serde(default)]
    key: String,
}

#[derive(Debug, Deserialize)]
struct Bundle {
    sample_rate: u32,
    frame_rate: f32,
    latent_dim: usize,
    conditioning_dim: usize,
    #[serde(default)]
    insert_bos_before_voice: bool,
    #[serde(default)]
    remove_semicolons: bool,
    #[serde(default)]
    model_recommended_frames_after_eos: Option<u32>,
    #[serde(default)]
    bos_before_voice_file: Option<String>,
    #[serde(default)]
    pad_with_spaces_for_short_inputs: bool,
    flow_lm_state_manifest: Vec<StateEntry>,
    mimi_state_manifest: Vec<StateEntry>,
}

// ── Parser .npy minimal (f32, C-order, v1/v2) ───────────────────────────────

fn load_npy_f32(path: &Path) -> Result<(Vec<usize>, Vec<f32>), Err> {
    let bytes = std::fs::read(path)?;
    if bytes.len() < 12 || &bytes[..6] != b"\x93NUMPY" {
        return Err(format!("{}: pas un fichier .npy", path.display()).into());
    }
    let (header_len, data_at) = if bytes[6] == 1 {
        (u16::from_le_bytes([bytes[8], bytes[9]]) as usize, 10)
    } else {
        (u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize, 12)
    };
    let header = std::str::from_utf8(&bytes[data_at..data_at + header_len])?;
    if !header.contains("<f4") || header.contains("'fortran_order': True") {
        return Err(format!("{}: seul le f32 C-order est supporte ({header})", path.display()).into());
    }
    let shape_txt = header
        .split('(')
        .nth(1)
        .and_then(|s| s.split(')').next())
        .ok_or("npy: shape introuvable")?;
    let shape: Vec<usize> =
        shape_txt.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    let n: usize = shape.iter().product();
    let data_bytes = &bytes[data_at + header_len..];
    if data_bytes.len() < n * 4 {
        return Err("npy: donnees tronquees".into());
    }
    let data = data_bytes[..n * 4]
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    Ok((shape, data))
}

// ── Parser .safetensors minimal (états de voix officiels Kyutai) ────────────

struct StTensor {
    dtype: String,
    shape: Vec<usize>,
    /// Offsets absolus dans le fichier.
    begin: usize,
    end: usize,
}

fn f16_to_f32(h: u16) -> f32 {
    let (s, e, m) = ((h >> 15) & 1, (h >> 10) & 0x1f, h & 0x3ff);
    let sign = if s == 1 { -1.0f32 } else { 1.0 };
    match (e, m) {
        (0, 0) => sign * 0.0,
        (0, m) => sign * (m as f32) * 2f32.powi(-24),
        (0x1f, 0) => sign * f32::INFINITY,
        (0x1f, _) => f32::NAN,
        (e, m) => sign * (1.0 + m as f32 / 1024.0) * 2f32.powi(e as i32 - 15),
    }
}

/// Charge un .safetensors : (octets du fichier, index nom → descripteur).
fn load_safetensors(path: &Path) -> Result<(Vec<u8>, HashMap<String, StTensor>), Err> {
    let bytes = std::fs::read(path)?;
    if bytes.len() < 8 {
        return Err(format!("{}: safetensors tronque", path.display()).into());
    }
    let header_len = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
    let header: serde_json::Value = serde_json::from_slice(&bytes[8..8 + header_len])?;
    let data_at = 8 + header_len;
    let mut tensors = HashMap::new();
    for (name, desc) in header.as_object().ok_or("safetensors: en-tete invalide")? {
        if name == "__metadata__" {
            continue;
        }
        let dtype = desc["dtype"].as_str().ok_or("dtype manquant")?.to_string();
        let shape: Vec<usize> = desc["shape"]
            .as_array()
            .ok_or("shape manquante")?
            .iter()
            .filter_map(|v| v.as_u64().map(|x| x as usize))
            .collect();
        let offs = desc["data_offsets"].as_array().ok_or("offsets manquants")?;
        let begin = data_at + offs[0].as_u64().ok_or("offset")? as usize;
        let end = data_at + offs[1].as_u64().ok_or("offset")? as usize;
        if end > bytes.len() || begin > end {
            return Err("safetensors: offsets hors fichier".into());
        }
        tensors.insert(name.clone(), StTensor { dtype, shape, begin, end });
    }
    Ok((bytes, tensors))
}

/// Lit un StTensor en f32 (F32/F16/BF16 acceptés).
fn st_as_f32(raw: &[u8], t: &StTensor) -> Result<Vec<f32>, Err> {
    let b = &raw[t.begin..t.end];
    Ok(match t.dtype.as_str() {
        "F32" => b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
        "F16" => b.chunks_exact(2).map(|c| f16_to_f32(u16::from_le_bytes([c[0], c[1]]))).collect(),
        "BF16" => b
            .chunks_exact(2)
            .map(|c| f32::from_bits((u16::from_le_bytes([c[0], c[1]]) as u32) << 16))
            .collect(),
        other => return Err(format!("dtype {other} non gere en f32").into()),
    })
}

fn st_as_i64(raw: &[u8], t: &StTensor) -> Result<Vec<i64>, Err> {
    let b = &raw[t.begin..t.end];
    Ok(match t.dtype.as_str() {
        "I64" => b
            .chunks_exact(8)
            .map(|c| i64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
            .collect(),
        "I32" => b.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as i64).collect(),
        other => return Err(format!("dtype {other} non gere en i64").into()),
    })
}

/// Copie la zone de recouvrement de `src` (src_shape) vers `dst` (dst_shape),
/// les deux en ordre C — port de `_adapt_state_tensor` de la référence.
fn copy_overlap_f32(src: &[f32], src_shape: &[usize], dst: &mut [f32], dst_shape: &[usize]) {
    let dims = src_shape.len();
    let common: Vec<usize> =
        src_shape.iter().zip(dst_shape).map(|(a, b)| *a.min(b)).collect();
    if common.iter().any(|&c| c == 0) {
        return;
    }
    let stride = |shape: &[usize]| -> Vec<usize> {
        let mut s = vec![1usize; dims];
        for i in (0..dims.saturating_sub(1)).rev() {
            s[i] = s[i + 1] * shape[i + 1];
        }
        s
    };
    let (ss, ds) = (stride(src_shape), stride(dst_shape));
    let mut idx = vec![0usize; dims];
    'outer: loop {
        let (mut so, mut dof) = (0usize, 0usize);
        for d in 0..dims {
            so += idx[d] * ss[d];
            dof += idx[d] * ds[d];
        }
        dst[dof] = src[so];
        // incrementer l'index multidimensionnel
        for d in (0..dims).rev() {
            idx[d] += 1;
            if idx[d] < common[d] {
                continue 'outer;
            }
            idx[d] = 0;
            if d == 0 {
                break 'outer;
            }
        }
    }
}

// ── Tokenizer SentencePiece unigram (Viterbi) ───────────────────────────────

/// Encodeur unigram sur `vocab.json` + `token_scores.json` (produits par
/// scripts/pocket-tts/convert_tokenizer.py de sherpa-onnx). Reproduit
/// l'algorithme SentencePiece : préfixe `▁`, espaces → `▁`, Viterbi qui
/// maximise la somme des scores, fallback octets `<0xXX>` pour l'inconnu.
pub struct SpTokenizer {
    pieces: HashMap<String, (i64, f32)>,
    max_piece_chars: usize,
}

impl SpTokenizer {
    pub fn load(vocab_json: &Path, scores_json: &Path) -> Result<Self, Err> {
        let vocab: HashMap<String, i64> =
            serde_json::from_str(&std::fs::read_to_string(vocab_json)?)?;
        let scores: HashMap<String, f32> =
            serde_json::from_str(&std::fs::read_to_string(scores_json)?)?;
        let mut pieces = HashMap::with_capacity(vocab.len());
        let mut max_piece_chars = 1;
        for (tok, id) in vocab {
            let score = scores.get(&tok).copied().unwrap_or(0.0);
            max_piece_chars = max_piece_chars.max(tok.chars().count());
            pieces.insert(tok, (id, score));
        }
        Ok(Self { pieces, max_piece_chars: max_piece_chars.min(16) })
    }

    /// Fallback : encode un caractère inconnu en pièces octets `<0xXX>`.
    /// Retourne (ids, somme des scores), ou None si une pièce octet manque.
    fn byte_pieces(&self, ch: char) -> Option<(Vec<i64>, f32)> {
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        let mut ids = Vec::with_capacity(s.len());
        let mut score = 0.0;
        for b in s.bytes() {
            let (id, sc) = *self.pieces.get(&format!("<0x{b:02X}>"))?;
            ids.push(id);
            score += sc;
        }
        Some((ids, score))
    }

    pub fn encode(&self, text: &str) -> Vec<i64> {
        // Normalisation SentencePiece (add_dummy_prefix) : préfixe ▁,
        // espaces → ▁. Le texte arrive déjà nettoyé (clean_for_tts).
        let mut s = String::with_capacity(text.len() + 4);
        s.push('▁');
        for ch in text.chars() {
            s.push(if ch == ' ' { '▁' } else { ch });
        }
        let chars: Vec<char> = s.chars().collect();
        let n = chars.len();

        let mut best = vec![f32::NEG_INFINITY; n + 1];
        best[0] = 0.0;
        // (position précédente, ids émis par la transition)
        let mut back: Vec<(usize, Vec<i64>)> = vec![(0, Vec::new()); n + 1];
        let mut buf = String::new();
        for i in 0..n {
            if best[i] == f32::NEG_INFINITY {
                continue;
            }
            buf.clear();
            for l in 1..=self.max_piece_chars.min(n - i) {
                buf.push(chars[i + l - 1]);
                if let Some(&(id, score)) = self.pieces.get(buf.as_str()) {
                    let cand = best[i] + score;
                    if cand > best[i + l] {
                        best[i + l] = cand;
                        back[i + l] = (i, vec![id]);
                    }
                }
            }
            if best[i + 1] == f32::NEG_INFINITY {
                // Caractère hors vocabulaire : octets UTF-8.
                if let Some((ids, score)) = self.byte_pieces(chars[i]) {
                    best[i + 1] = best[i] + score;
                    back[i + 1] = (i, ids);
                }
            }
        }

        let mut out: Vec<i64> = Vec::new();
        let mut i = n;
        while i > 0 {
            let (prev, ids) = &back[i];
            for &id in ids.iter().rev() {
                out.push(id);
            }
            i = *prev;
        }
        out.reverse();
        out
    }
}

// ── Générateur normal (Box-Muller sur xorshift64*) ──────────────────────────

struct NormalGen {
    state: u64,
    spare: Option<f32>,
}

impl NormalGen {
    fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64 | 1)
            .unwrap_or(0x9e3779b97f4a7c15);
        Self { state: seed.wrapping_mul(0x2545F4914F6CDD1D) | 1, spare: None }
    }

    /// Re-graine le flux de bruit. La préversion `french_24l` tire l'identité
    /// vocale au sort à CHAQUE prise (température 0,7 — verdict R1.5) : une
    /// graine FIXE rend les prises REPRODUCTIBLES (même texte, même voix,
    /// même prise) — on choisit une bonne prise au lieu de la subir.
    fn reseed(&mut self, seed: u64) {
        self.state = (seed | 1).wrapping_mul(0x2545F4914F6CDD1D) | 1;
        self.spare = None;
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn uniform(&mut self) -> f32 {
        // (0, 1] pour pouvoir passer au ln()
        ((self.next_u64() >> 40) as f32 + 1.0) / (1u64 << 24) as f32
    }

    fn normal(&mut self, std_dev: f32) -> f32 {
        if let Some(z) = self.spare.take() {
            return z * std_dev;
        }
        let (u1, u2) = (self.uniform(), self.uniform());
        let r = (-2.0 * u1.ln()).sqrt();
        let (s, c) = (2.0 * std::f32::consts::PI * u2).sin_cos();
        self.spare = Some(r * s);
        r * c * std_dev
    }
}

// ── Tenseurs à dimension nulle ──────────────────────────────────────────────

/// Tenseur f32 dont des dimensions peuvent être NULLES ([1,0,32], [0]…).
/// L'API C d'ORT les accepte (`CreateTensorAsOrtValue`) mais le crate ort
/// rc.9 les refuse dans `ToDimensions` — le pipeline Pocket en a besoin
/// partout (séquences vides, états de convolutions streaming au départ).
fn empty_f32(shape: &[i64]) -> Result<DynValue, Err> {
    let api = ort::api();
    unsafe {
        let mut alloc: *mut ort::sys::OrtAllocator = std::ptr::null_mut();
        let check = |status: *mut ort::sys::OrtStatus| -> Result<(), Err> {
            if status.is_null() {
                return Ok(());
            }
            let msg = api.GetErrorMessage.map(|f| f(status)).unwrap_or(std::ptr::null());
            let text = if msg.is_null() {
                "erreur ORT".to_string()
            } else {
                std::ffi::CStr::from_ptr(msg).to_string_lossy().into_owned()
            };
            if let Some(f) = api.ReleaseStatus {
                f(status);
            }
            Err(text.into())
        };
        check(api.GetAllocatorWithDefaultOptions.ok_or("OrtApi incomplete")?(&mut alloc))?;
        let mut val: *mut ort::sys::OrtValue = std::ptr::null_mut();
        check(api.CreateTensorAsOrtValue.ok_or("OrtApi incomplete")?(
            alloc,
            shape.as_ptr(),
            shape.len(),
            ort::sys::ONNXTensorElementDataType::ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT,
            &mut val,
        ))?;
        let ptr = std::ptr::NonNull::new(val).ok_or("CreateTensorAsOrtValue a rendu null")?;
        Ok(DynValue::from_ptr(ptr, None))
    }
}

// ── Pipeline ────────────────────────────────────────────────────────────────

/// Pocket TTS natif : l'option QUALITÉ + clonage de voix de la cascade.
/// Fonctionne avec les bundles HF (schéma 2, dont `french_24l`) que la DLL
/// sherpa ne sait pas exécuter.
pub struct PocketNative {
    /// Répertoire du bundle : l'encodeur mimi est chargé À LA DEMANDE dans
    /// [`Self::set_voice`] puis jeté — sa session (et son arène d'activations
    /// sur ~10 s d'audio) ne vit que le temps du clonage.
    model_dir: std::path::PathBuf,
    text_conditioner: Session,
    lm_main: Session,
    lm_flow: Session,
    mimi_decoder: Session,
    tokenizer: SpTokenizer,
    meta: Bundle,
    bos: Option<(Vec<usize>, Vec<f32>)>,
    /// État du LM après conditionnement de la voix — jamais muté (ORT ne
    /// touche pas ses entrées), passé par vue à chaque synthèse.
    voice_state: Option<Vec<DynValue>>,
    temperature: f32,
    lsd_steps: usize,
    rng: RefCell<NormalGen>,
    /// Graine fixe optionnelle (WALY_POCKET_SEED) : prises déterministes —
    /// la parade R-V à l'identité instable de la préversion. None = tirage
    /// libre (comportement historique).
    seed: Option<u64>,
}

impl PocketNative {
    /// `model_dir` : bundle HF exporté (`flow_lm_*`, `mimi_*`,
    /// `text_conditioner.onnx`, `bundle.json`, `bos_before_voice.npy`,
    /// `vocab.json` + `token_scores.json` convertis du tokenizer.model).
    /// [`crate::vad::init_onnxruntime`] doit avoir été appelé avant.
    pub fn new(model_dir: &str, num_threads: usize) -> Result<Self, Err> {
        let dir = Path::new(model_dir);
        let meta: Bundle =
            serde_json::from_str(&std::fs::read_to_string(dir.join("bundle.json"))?)?;

        let pick = |first: &str, second: &str| -> std::path::PathBuf {
            let q = dir.join(first);
            if q.exists() {
                q
            } else {
                dir.join(second)
            }
        };
        let session = |p: std::path::PathBuf| -> Result<Session, Err> {
            Ok(Session::builder()?
                .with_intra_threads(num_threads)?
                .with_inter_threads(1)?
                .commit_from_file(p)?)
        };
        // Session SANS ARÈNE : ort garde sinon le PIC d'activations pour
        // toujours — la passe de conditionnement de voix (~130 trames à
        // travers le LM 24 couches) laissait ~2 Go d'arène morte (mesuré
        // R-V 2026-07-20 : 2,7 Go privés au lieu de ~800 Mo). En Device
        // allocator, les activations se libèrent après chaque run ; surcoût
        // RTF mesuré nul (le calcul domine l'allocation).
        let session_sans_arene = |p: std::path::PathBuf| -> Result<Session, Err> {
            use ort::memory::{AllocationDevice, AllocatorType, MemoryInfo, MemoryType};
            Ok(Session::builder()?
                .with_intra_threads(num_threads)?
                .with_inter_threads(1)?
                .with_memory_pattern(false)?
                .with_allocator(MemoryInfo::new(
                    AllocationDevice::CPU,
                    0,
                    AllocatorType::Device,
                    MemoryType::Default,
                )?)?
                .commit_from_file(p)?)
        };

        // Précisions PAR RÔLE (écoute étalon-or 2026-07-04) : la pile
        // officielle PyTorch pleine précision sonne nettement mieux que
        // l'int8 intégral — le grain vient du DÉCODEUR audio et du FLOW
        // quantisés. Eux passent en fp32 (~80 Mo), seul le LM 24 couches
        // (1,2 Go en fp32) reste int8 pour tenir le budget mémoire.
        let text_conditioner = session(dir.join("text_conditioner.onnx"))?;
        let lm_main = session_sans_arene(pick("flow_lm_main_int8.onnx", "flow_lm_main.onnx"))?;
        let lm_flow = session(pick("flow_lm_flow.onnx", "flow_lm_flow_int8.onnx"))?;
        let mimi_decoder = session(pick("mimi_decoder.onnx", "mimi_decoder_int8.onnx"))?;

        let tokenizer =
            SpTokenizer::load(&dir.join("vocab.json"), &dir.join("token_scores.json"))?;

        let bos = match (&meta.bos_before_voice_file, meta.insert_bos_before_voice) {
            (Some(f), true) => Some(load_npy_f32(&dir.join(f))?),
            _ => None,
        };

        let lsd_steps = std::env::var("WALY_POCKET_STEPS")
            .ok()
            .and_then(|v| v.parse().ok())
            // 5 : choix d'écoute de Michée (2026-07-04) — la référence met 1,
            // mais 5 pas ne coûtent que ~3 % de RTF (le flow est minuscule
            // face au LM) et le rendu est plus net.
            .unwrap_or(5);
        let temperature = std::env::var("WALY_POCKET_TEMP")
            .ok()
            .and_then(|v| v.parse().ok())
            // 0,7 (référence) et PAS moins : le bruit est ce qui sort le
            // modèle de l'attracteur silence — à 0,5 des prises entières
            // restent muettes, à 0 il ne parle jamais (mesuré 2026-07-04).
            // Les prises ratées se gèrent par re-prise (voir synth).
            .unwrap_or(0.7f32);

        // Graine 42 PAR DÉFAUT : le kit A/B jugé par Michée (verdict
        // 2026-07-20 : fabien/développeuse retenues) était tiré à 42 —
        // l'identité gravée est CETTE identité, pas un re-tirage par
        // lancement. WALY_POCKET_SEED=0 = tirage libre (bancs).
        let seed = match std::env::var("WALY_POCKET_SEED")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        {
            Some(0) => None,
            Some(s) => Some(s),
            None => Some(42),
        };

        Ok(Self {
            model_dir: dir.to_path_buf(),
            text_conditioner,
            lm_main,
            lm_flow,
            mimi_decoder,
            tokenizer,
            meta,
            bos,
            voice_state: None,
            temperature,
            lsd_steps: lsd_steps.max(1),
            rng: RefCell::new(NormalGen::new()),
            seed,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.meta.sample_rate
    }

    fn init_entry(e: &StateEntry) -> Result<DynValue, Err> {
        let n: usize = e.shape.iter().product();
        let shape = e.shape.clone();
        if n == 0 {
            // fill "empty" : tenseur de taille nulle (f32 uniquement dans
            // les bundles connus).
            if e.dtype != "float32" {
                return Err(format!("etat vide non-f32: {}", e.input_name).into());
            }
            let dims: Vec<i64> = shape.iter().map(|&d| d as i64).collect();
            return empty_f32(&dims);
        }
        Ok(match e.dtype.as_str() {
            "float32" => {
                let fill = if e.fill == "nan" { f32::NAN } else { 0.0 };
                Tensor::from_array((shape, vec![fill; n]))?.into_dyn()
            }
            "int64" => Tensor::from_array((shape, vec![0i64; n]))?.into_dyn(),
            "bool" => {
                let fill = e.fill == "ones";
                Tensor::from_array((shape, vec![fill; n]))?.into_dyn()
            }
            other => return Err(format!("dtype d'etat non gere: {other}").into()),
        })
    }

    fn init_state(manifest: &[StateEntry]) -> Result<Vec<DynValue>, Err> {
        manifest.iter().map(Self::init_entry).collect()
    }

    /// Charge un ÉTAT DE VOIX pré-calculé (.safetensors officiel Kyutai) :
    /// l'état du LM après conditionnement de la voix, préparé en studio —
    /// pas d'audio de référence ni d'encodage mimi. Port fidèle de
    /// `_state_from_model_state` de l'implémentation de référence.
    pub fn set_voice_state(&mut self, path: &Path) -> Result<(), Err> {
        let (raw, tensors) = load_safetensors(path)?;
        let manifest = &self.meta.flow_lm_state_manifest;
        let mut state = Vec::with_capacity(manifest.len());
        for e in manifest {
            let full = format!("{}/{}", e.module, e.key);
            let dv = if let Some(t) = tensors.get(&full) {
                Self::adapt_tensor(&raw, t, e)?
            } else if e.key == "step" {
                // Dérivation du pas quand absent (référence : offset sans
                // end_offset, sinon longueur de current_end, sinon 0).
                let step: i64 = if tensors.contains_key(&format!("{}/offset", e.module))
                    && !tensors.contains_key(&format!("{}/end_offset", e.module))
                {
                    let t = &tensors[&format!("{}/offset", e.module)];
                    st_as_i64(&raw, t)?.first().copied().unwrap_or(0)
                } else if let Some(t) = tensors.get(&format!("{}/current_end", e.module)) {
                    t.shape.first().copied().unwrap_or(0) as i64
                } else {
                    0
                };
                Tensor::from_array((vec![1usize], vec![step]))?.into_dyn()
            } else {
                Self::init_entry(e)?
            };
            state.push(dv);
        }
        self.voice_state = Some(state);
        Ok(())
    }

    /// Adapte un tenseur du fichier à l'entrée attendue par le graphe :
    /// tel quel si la forme colle, reshape à taille égale, sinon copie de la
    /// zone de recouvrement dans un tenseur rempli selon `fill`.
    fn adapt_tensor(raw: &[u8], t: &StTensor, e: &StateEntry) -> Result<DynValue, Err> {
        let target_n: usize = e.shape.iter().product();
        if target_n == 0 {
            // Cible déclarée vide (current_end, previous…) : l'information
            // de longueur passe par « step » — on rend l'état initial vide.
            return Self::init_entry(e);
        }
        match e.dtype.as_str() {
            "float32" => {
                let src = st_as_f32(raw, t)?;
                if t.shape == e.shape || src.len() == target_n {
                    return Ok(Tensor::from_array((e.shape.clone(), src))?.into_dyn());
                }
                let fill = if e.fill == "nan" { f32::NAN } else { 0.0 };
                let mut dst = vec![fill; target_n];
                if t.shape.len() == e.shape.len() {
                    copy_overlap_f32(&src, &t.shape, &mut dst, &e.shape);
                }
                Ok(Tensor::from_array((e.shape.clone(), dst))?.into_dyn())
            }
            "int64" => {
                let src = st_as_i64(raw, t)?;
                if src.len() == target_n {
                    return Ok(Tensor::from_array((e.shape.clone(), src))?.into_dyn());
                }
                let mut dst = vec![0i64; target_n];
                let n = src.len().min(target_n);
                dst[..n].copy_from_slice(&src[..n]);
                Ok(Tensor::from_array((e.shape.clone(), dst))?.into_dyn())
            }
            other => Err(format!("adaptation non geree pour dtype {other}").into()),
        }
    }

    /// Passe dans `flow_lm_main` : (conditioning, eos_logit, nouvel état).
    /// L'état d'entrée est passé par VUE (jamais consommé ni muté).
    fn run_lm_main(
        &self,
        sequence: SessionInputValue<'_>,
        text_embeddings: SessionInputValue<'_>,
        state: &[DynValue],
    ) -> Result<(Vec<f32>, f32, Vec<DynValue>), Err> {
        let manifest = &self.meta.flow_lm_state_manifest;
        let mut inputs: Vec<(std::borrow::Cow<'_, str>, SessionInputValue<'_>)> =
            Vec::with_capacity(2 + manifest.len());
        inputs.push(("sequence".into(), sequence));
        inputs.push(("text_embeddings".into(), text_embeddings));
        for (e, v) in manifest.iter().zip(state) {
            inputs.push((e.input_name.as_str().into(), v.view().into()));
        }
        let mut outputs = self.lm_main.run(inputs)?;
        let conditioning: Vec<f32> =
            outputs["conditioning"].try_extract_tensor::<f32>()?.iter().copied().collect();
        let eos = outputs["eos_logit"]
            .try_extract_tensor::<f32>()?
            .iter()
            .next()
            .copied()
            .ok_or("eos_logit vide")?;
        let new_state: Vec<DynValue> = manifest
            .iter()
            .map(|e| {
                outputs
                    .remove(e.output_name.as_str())
                    .ok_or_else(|| format!("sortie manquante: {}", e.output_name).into())
            })
            .collect::<Result<_, Err>>()?;
        Ok((conditioning, eos, new_state))
    }

    /// Fixe la voix depuis un audio de référence mono (≤ ~10 s conseillé).
    /// L'encodeur mimi est chargé ICI puis jeté : session et arène (~10 s
    /// d'activations) ne survivent pas au clonage (budget mémoire R-V).
    pub fn set_voice(&mut self, samples: &[f32], sample_rate: u32) -> Result<(), Err> {
        let audio = resample_sinc(samples, sample_rate, self.meta.sample_rate);
        let max_len = self.meta.sample_rate as usize * 10;
        let audio = if audio.len() > max_len { audio[..max_len].to_vec() } else { audio };
        let n = audio.len();

        let audio_t = Tensor::from_array((vec![1usize, 1, n], audio))?;
        let emb: Vec<f32> = {
            let mimi_encoder = Session::builder()?
                .with_intra_threads(2)?
                .with_inter_threads(1)?
                .commit_from_file(self.model_dir.join("mimi_encoder.onnx"))?;
            let outputs = mimi_encoder.run(ort::inputs!["audio" => audio_t]?)?;
            let latents = outputs["latents"].try_extract_tensor::<f32>()?;
            latents.iter().copied().collect()
        };
        let dim = self.meta.conditioning_dim;
        let frames = emb.len() / dim;

        // Schéma 2 : BOS préfixé à l'embedding de voix (le correctif que
        // sherpa n'a pas — sans lui, EOS prématuré et charabia).
        let (total_frames, data) = match &self.bos {
            Some((bshape, bdata)) => {
                let bframes: usize = bshape.iter().product::<usize>() / dim;
                let mut d = Vec::with_capacity(bdata.len() + emb.len());
                d.extend_from_slice(bdata);
                d.extend_from_slice(&emb);
                (bframes + frames, d)
            }
            None => (frames, emb),
        };
        let voice_emb = Tensor::from_array((vec![1usize, total_frames, dim], data))?;

        let init = Self::init_state(&self.meta.flow_lm_state_manifest)?;
        let empty_seq = empty_f32(&[1, 0, self.meta.latent_dim as i64])?;
        let (_, _, state) = self.run_lm_main(empty_seq.into(), voice_emb.into(), &init)?;
        self.voice_state = Some(state);
        Ok(())
    }

    /// Préparation du texte, fidèle à l'implémentation de référence.
    fn prepare_text(&self, text: &str) -> String {
        let mut t = text.trim().replace(['\n', '\r'], " ");
        while t.contains("  ") {
            t = t.replace("  ", " ");
        }
        if self.meta.remove_semicolons {
            t = t.replace(';', ",");
        }
        if t.is_empty() {
            return t;
        }
        let mut chars: Vec<char> = t.chars().collect();
        let first_up: Vec<char> = chars[0].to_uppercase().collect();
        chars.splice(0..1, first_up);
        // Un fragment qui finit en ponctuation faible désoriente le modèle
        // (entraîné sur des phrases) : on ferme proprement.
        if matches!(chars.last(), Some(',' | ';' | ':')) {
            chars.pop();
        }
        if chars.last().map(|c| c.is_alphanumeric()).unwrap_or(false) {
            chars.push('.');
        }
        let mut out: String = chars.into_iter().collect();
        if self.meta.pad_with_spaces_for_short_inputs && out.split_whitespace().count() < 5 {
            out = format!("{}{}", " ".repeat(8), out);
        }
        out
    }

    /// Synthétise un texte (déjà nettoyé par `clean_for_tts`).
    /// Retourne les échantillons mono à [`Self::sample_rate`].
    /// Chemin BLOQUANT (bench, tests) : collecte de [`Self::synth_stream`].
    pub fn synth(&self, text: &str, _speed: f32) -> Result<Vec<f32>, Err> {
        let mut audio: Vec<f32> = Vec::new();
        self.synth_stream(text, &mut |chunk| {
            audio.extend_from_slice(chunk);
            true
        })?;
        Ok(audio)
    }

    /// Synthèse STREAMING (chantier R-V) : `on_audio` reçoit l'audio mono à
    /// [`Self::sample_rate`] AU FIL de la génération — le premier chunk bien
    /// avant la fin de la phrase. Un appel avec un chunk VIDE est une sonde
    /// d'interruption (même contrat que le callback LLM de la cascade) :
    /// retourner `false` arrête la synthèse immédiatement (barge-in).
    /// Retourne Ok(false) si interrompue, Ok(true) sinon.
    ///
    /// ⚠ UNE PHRASE PAR GÉNÉRATION, comme l'implémentation de référence :
    /// le modèle n'émet son EOS proprement que sur une phrase isolée. Nourri
    /// de plusieurs phrases d'un coup, l'EOS ne tombe pas et la génération
    /// divague jusqu'au plafond — bruits épars et « suites fantômes »
    /// entendus sur le terrain (profils d'énergie george/jane, 2026-07-04).
    pub fn synth_stream(
        &self,
        text: &str,
        on_audio: &mut dyn FnMut(&[f32]) -> bool,
    ) -> Result<bool, Err> {
        let prepared = self.prepare_text(text);
        if prepared.is_empty() {
            return Ok(true);
        }
        let mut spoke_before = false;
        for sentence in crate::clause::split_clauses(&prepared) {
            // RE-PRISE : les tirages sont indépendants (température 0,7) —
            // une prise MUETTE se rejoue, comme un comédien qui refait la
            // ligne. En streaming la décision tombe AVANT toute émission
            // (porte de silence : ~2,5 s de trames sans voix = prise avortée,
            // rien n'a été entendu). 3 essais max. Les bornes de plausibilité
            // de l'ancien chemin bloquant (prise trop courte/trop longue) ne
            // sont pas rattrapables une fois l'audio émis : assumé, le
            // plafond de trames borne toujours la divagation.
            let mut gap_du = false;
            for essai in 0..3u64 {
                if spoke_before && !gap_du {
                    // Respiration naturelle entre deux phrases (les silences
                    // générés étant rognés par la porte, on la réinsère).
                    let gap = (self.meta.sample_rate as usize * 250) / 1000;
                    if !on_audio(&vec![0.0f32; gap]) {
                        return Ok(false);
                    }
                    gap_du = true;
                }
                match self.stream_sentence(&sentence, essai, on_audio)? {
                    Take::Spoke => {
                        spoke_before = true;
                        break;
                    }
                    Take::Interrupted => return Ok(false),
                    Take::Mute => {
                        tracing::debug!(essai, sentence, "prise muette, on rejoue");
                    }
                }
            }
        }
        Ok(true)
    }

    /// Une phrase en STREAMING : tokenisation → conditionnement texte →
    /// boucle LM/flow avec DÉCODAGE MIMI ENTRELACÉ — chunks courts d'abord
    /// (2, 4, 8 puis 12 trames : premier son tôt), plus larges ensuite
    /// (efficacité). La porte de silence en flux rogne la tête et la queue
    /// (marge 120 ms), et permet d'avorter une prise muette avant d'avoir
    /// rien émis. État mimi frais par phrase, comme la référence.
    fn stream_sentence(
        &self,
        sentence: &str,
        essai: u64,
        on_audio: &mut dyn FnMut(&[f32]) -> bool,
    ) -> Result<Take, Err> {
        let voice_state =
            self.voice_state.as_ref().ok_or("PocketNative: aucune voix (set_voice)")?;
        let prepared = self.prepare_text(sentence);
        if prepared.is_empty() {
            return Ok(Take::Spoke);
        }
        if let Some(base) = self.seed {
            // Graine par phrase ET par essai : même texte → même prise
            // (identité stable, cf. reseed) ; un essai muet re-tire un flux
            // différent.
            let mut h = 0xcbf29ce484222325u64; // FNV-1a du texte préparé
            for b in prepared.bytes() {
                h = (h ^ b as u64).wrapping_mul(0x100000001b3);
            }
            self.rng
                .borrow_mut()
                .reseed(base ^ h ^ essai.wrapping_mul(0x9e3779b97f4a7c15));
        }
        let ids = self.tokenizer.encode(&prepared);
        if ids.is_empty() {
            return Ok(Take::Spoke);
        }
        let n_tokens = ids.len();
        let latent_dim = self.meta.latent_dim;
        let cond_dim = self.meta.conditioning_dim;

        // Conditionnement texte (état voix passé par vue, non consommé).
        let ids_t = Tensor::from_array((vec![1usize, n_tokens], ids))?;
        let mut cond_out = self.text_conditioner.run(ort::inputs!["token_ids" => ids_t]?)?;
        let text_emb = cond_out.remove("embeddings").ok_or("text_conditioner sans sortie")?;
        let empty_seq = empty_f32(&[1, 0, latent_dim as i64])?;
        let (_, _, mut state) =
            self.run_lm_main(empty_seq.into(), text_emb.into(), voice_state)?;

        // Boucle de génération des latents.
        let frames_after_eos =
            self.meta.model_recommended_frames_after_eos.unwrap_or(3) as usize;
        // Plafond par PHRASE, un peu plus souple que la référence (tokens/3
        // + 2 s) : un débit lent tronquait des fins de phrase. L'EOS arrête
        // la génération avant le plafond en régime normal ; s'il ne tombe
        // pas, le plafond borne le charabia.
        let frame_limit = ((n_tokens as f32 / 2.5 + 2.5) * self.meta.frame_rate).ceil() as usize;
        let std_dev = self.temperature.sqrt();
        let dt = 1.0 / self.lsd_steps as f32;

        // Porte de silence en flux (équivalent streaming de trim_silence).
        let sr = self.meta.sample_rate as usize;
        let mut gate = SilenceGate::new(sr);
        // Prise muette : ~2,5 s de trames DÉCODÉES sans voix → on avorte
        // avant d'avoir rien émis (le modèle échantillonne parfois plusieurs
        // secondes de pause ; attendre coûte plus cher que rejouer).
        let mute_abort_samples = sr * 5 / 2;

        let mut mimi_state = Self::init_state(&self.meta.mimi_state_manifest)?;
        let mut pending: Vec<f32> = Vec::new(); // latents en attente de décodage
        let mut decoded_samples = 0usize;
        let mut chunk_target = 2usize; // trames : 2 → 4 → 8 → 12 (premier son tôt)

        let mut curr: Vec<f32> = vec![f32::NAN; latent_dim];
        let mut eos_step: Option<usize> = None;

        for step in 0..frame_limit {
            // Sonde d'interruption au rythme des trames (~50-80 ms de calcul).
            if !on_audio(&[]) {
                return Ok(Take::Interrupted);
            }
            let curr_t = Tensor::from_array((vec![1usize, 1, latent_dim], curr.clone()))?;
            let empty_text = empty_f32(&[1, 0, cond_dim as i64])?;
            let (conditioning, eos, new_state) =
                self.run_lm_main(curr_t.into(), empty_text.into(), &state)?;
            state = new_state;

            if std::env::var("WALY_POCKET_DEBUG").as_deref() == Ok("1") && step < 12 {
                eprintln!("  step {step}: eos_logit {eos:.3}");
            }
            if eos > -4.0 && eos_step.is_none() {
                eos_step = Some(step);
            }
            if let Some(e) = eos_step {
                if step >= e + frames_after_eos {
                    break;
                }
            }

            let mut x: Vec<f32> = {
                let mut rng = self.rng.borrow_mut();
                (0..latent_dim).map(|_| rng.normal(std_dev)).collect()
            };
            for j in 0..self.lsd_steps {
                let s = j as f32 / self.lsd_steps as f32;
                let t = s + dt;
                let c_t = Tensor::from_array((vec![1usize, cond_dim], conditioning.clone()))?;
                let s_t = Tensor::from_array((vec![1usize, 1], vec![s]))?;
                let t_t = Tensor::from_array((vec![1usize, 1], vec![t]))?;
                let x_t = Tensor::from_array((vec![1usize, latent_dim], x.clone()))?;
                let flow_out = self.lm_flow.run(ort::inputs![
                    "c" => c_t,
                    "s" => s_t,
                    "t" => t_t,
                    "x" => x_t,
                ]?)?;
                let flow = flow_out["flow_dir"].try_extract_tensor::<f32>()?;
                for (xi, fi) in x.iter_mut().zip(flow.iter()) {
                    *xi += fi * dt;
                }
            }
            pending.extend_from_slice(&x);
            curr = x;

            // Décodage entrelacé : dès que le chunk cible est plein.
            if pending.len() / latent_dim >= chunk_target {
                let audio = self.decode_chunk(&mut pending, latent_dim, &mut mimi_state)?;
                decoded_samples += audio.len();
                chunk_target = (chunk_target * 2).min(12);
                if !gate.push(&audio, on_audio) {
                    return Ok(Take::Interrupted);
                }
                if !gate.voiced && decoded_samples >= mute_abort_samples {
                    return Ok(Take::Mute);
                }
            }
        }
        // Queue de latents puis clôture de la porte (marge de queue).
        if !pending.is_empty() {
            let audio = self.decode_chunk(&mut pending, latent_dim, &mut mimi_state)?;
            if !gate.push(&audio, on_audio) {
                return Ok(Take::Interrupted);
            }
        }
        if !gate.finish(on_audio) {
            return Ok(Take::Interrupted);
        }
        Ok(if gate.emitted { Take::Spoke } else { Take::Mute })
    }

    /// Décode les latents en attente (les vide) via le décodeur mimi
    /// streaming, en avançant son état.
    fn decode_chunk(
        &self,
        pending: &mut Vec<f32>,
        latent_dim: usize,
        mimi_state: &mut Vec<DynValue>,
    ) -> Result<Vec<f32>, Err> {
        let frames = pending.len() / latent_dim;
        let chunk_t =
            Tensor::from_array((vec![1usize, frames, latent_dim], std::mem::take(pending)))?;
        let manifest = &self.meta.mimi_state_manifest;
        let mut inputs: Vec<(std::borrow::Cow<'_, str>, SessionInputValue<'_>)> =
            Vec::with_capacity(1 + manifest.len());
        inputs.push(("latent".into(), chunk_t.into()));
        for (e, v) in manifest.iter().zip(mimi_state.iter()) {
            inputs.push((e.input_name.as_str().into(), v.view().into()));
        }
        let mut outputs = self.mimi_decoder.run(inputs)?;
        let audio: Vec<f32> =
            outputs["audio_frame"].try_extract_tensor::<f32>()?.iter().copied().collect();
        *mimi_state = manifest
            .iter()
            .map(|e| {
                outputs
                    .remove(e.output_name.as_str())
                    .ok_or_else(|| format!("sortie manquante: {}", e.output_name).into())
            })
            .collect::<Result<_, Err>>()?;
        Ok(audio)
    }
}

/// Issue d'une prise streamée.
enum Take {
    /// De la voix a été émise (prise jouée).
    Spoke,
    /// Rien de voisé — rien n'a été émis, la prise peut se rejouer.
    Mute,
    /// `on_audio` a demandé l'arrêt (barge-in).
    Interrupted,
}

/// Porte de silence EN FLUX : rogne la tête (marge 120 ms avant la première
/// voix), retient le silence courant et ne le relâche que si de la voix suit
/// (les pauses intra-phrase gardent leur durée exacte) ; à la clôture, le
/// silence de queue au-delà de la marge est abandonné. Équivalent streaming
/// de l'ancien `trim_silence` — le modèle traîne parfois des secondes d'air
/// mort avant et après la voix (latence perçue, bruits de queue), parité
/// vérifiée avec la référence : c'est le modèle, pas nous.
struct SilenceGate {
    /// Fenêtre d'énergie (20 ms).
    win: usize,
    /// Marge conservée autour de la voix (120 ms).
    margin: usize,
    /// Porte RMS ABSOLUE : le plancher de l'ancien trim_silence (la porte
    /// adaptative à 12 % du pic exigeait la prise entière — impossible en
    /// flux).
    gate: f32,
    /// Échantillons en attente d'une fenêtre d'énergie complète.
    carry: Vec<f32>,
    /// Silence retenu (avant la voix : borné à la marge ; après : intégral).
    held: Vec<f32>,
    voiced: bool,
    emitted: bool,
}

impl SilenceGate {
    fn new(sample_rate: usize) -> Self {
        Self {
            win: sample_rate * 20 / 1000,
            margin: sample_rate * 120 / 1000,
            gate: 0.012,
            carry: Vec::new(),
            held: Vec::new(),
            voiced: false,
            emitted: false,
        }
    }

    /// Pousse de l'audio décodé ; émet via `on_audio` ce qui passe la porte.
    /// Retourne false si `on_audio` a demandé l'arrêt.
    fn push(&mut self, audio: &[f32], on_audio: &mut dyn FnMut(&[f32]) -> bool) -> bool {
        self.carry.extend_from_slice(audio);
        let mut at = 0usize;
        while self.carry.len() - at >= self.win {
            let win: Vec<f32> = self.carry[at..at + self.win].to_vec();
            at += self.win;
            if !self.window(&win, on_audio) {
                return false;
            }
        }
        self.carry.drain(..at);
        true
    }

    fn window(&mut self, w: &[f32], on_audio: &mut dyn FnMut(&[f32]) -> bool) -> bool {
        let rms = (w.iter().map(|s| s * s).sum::<f32>() / w.len().max(1) as f32).sqrt();
        if rms > self.gate {
            if !self.voiced {
                self.voiced = true;
                // Marge : garder la fin du silence de tête (attaque douce).
                let start = self.held.len().saturating_sub(self.margin);
                if start < self.held.len() {
                    let head: Vec<f32> = self.held[start..].to_vec();
                    if !on_audio(&head) {
                        return false;
                    }
                }
            } else if !self.held.is_empty() {
                // Pause intra-phrase : relâchée intégralement, la voix suit.
                let held = std::mem::take(&mut self.held);
                if !on_audio(&held) {
                    return false;
                }
            }
            self.held.clear();
            self.emitted = true;
            on_audio(w)
        } else {
            self.held.extend_from_slice(w);
            if !self.voiced && self.held.len() > self.margin {
                // Avant la voix, seule la marge compte : mémoire bornée.
                let cut = self.held.len() - self.margin;
                self.held.drain(..cut);
            }
            true
        }
    }

    /// Clôture de prise : dernière fenêtre partielle, puis marge de queue.
    fn finish(&mut self, on_audio: &mut dyn FnMut(&[f32]) -> bool) -> bool {
        if !self.carry.is_empty() {
            let last = std::mem::take(&mut self.carry);
            if !self.window(&last, on_audio) {
                return false;
            }
        }
        if self.voiced && !self.held.is_empty() {
            let keep = self.held.len().min(self.margin);
            let tail: Vec<f32> = self.held[..keep].to_vec();
            self.held.clear();
            if !on_audio(&tail) {
                return false;
            }
        }
        true
    }
}

/// Rééchantillonnage par sinc fenêtré (Hann, 32 taps de demi-largeur).
/// Public : sert aussi à `record` pour produire des références de voix
/// propres (le linéaire suffit pour le chemin VAD/STT temps réel).
/// La QUALITÉ compte ici : l'interpolation linéaire crée des images
/// spectrales (sifflement métallique) que le clonage de voix reproduit
/// fidèlement — entendu sur le terrain (« ça grisouille dans le fond »).
/// Coût unique au `set_voice`, hors du chemin critique.
pub fn resample_sinc(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    const TAPS: isize = 32;
    let step = from as f64 / to as f64;
    // Coupure à la plus basse des deux fréquences de Nyquist.
    let fc = 0.5 * (to as f64 / from as f64).min(1.0);
    let out_len = (input.len() as f64 / step).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for n in 0..out_len {
        let pos = n as f64 * step;
        let center = pos.floor() as isize;
        let frac = pos - center as f64;
        let mut acc = 0.0f64;
        for k in (-TAPS + 1)..=TAPS {
            let idx = center + k;
            if idx < 0 || idx as usize >= input.len() {
                continue;
            }
            let x = k as f64 - frac;
            let sinc = if x.abs() < 1e-9 {
                2.0 * fc
            } else {
                let px = std::f64::consts::PI * x;
                (2.0 * fc * px).sin() / px
            };
            let hann = 0.5 * (1.0 + (std::f64::consts::PI * x / TAPS as f64).cos());
            acc += input[idx as usize] as f64 * sinc * hann;
        }
        out.push(acc as f32);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bundle FR s'il est présent (Windows : C:\waly, WSL : /mnt/c/waly).
    fn bundle_dir() -> Option<std::path::PathBuf> {
        for p in [r"C:\waly\engines\models\pocket-tts-fr-24l", "/mnt/c/waly/engines/models/pocket-tts-fr-24l"] {
            let pb = std::path::PathBuf::from(p);
            if pb.join("vocab.json").exists() {
                return Some(pb);
            }
        }
        None
    }

    /// Vérité terrain générée avec le VRAI SentencePiece (python, 2026-07-04)
    /// sur le tokenizer.model du bundle french_24l.
    #[test]
    fn tokenizer_identique_a_sentencepiece() {
        let Some(dir) = bundle_dir() else {
            eprintln!("bundle french_24l absent: test saute");
            return;
        };
        let tok =
            SpTokenizer::load(&dir.join("vocab.json"), &dir.join("token_scores.json")).unwrap();
        let cases: &[(&str, &[i64])] = &[
            (
                "Il est quatorze heures et il fait très beau aujourd'hui à Paris.",
                &[355, 299, 286, 495, 497, 504, 269, 260, 1467, 272, 295, 300, 338, 372, 274, 447, 262, 437, 273, 1033, 263],
            ),
            ("Bonjour, je suis Waly.", &[875, 1330, 261, 284, 375, 786, 370, 359, 263]),
            ("D'accord !", &[467, 262, 847, 260, 37]),
            (
                "Ça va très bien aujourd'hui, merci.",
                &[462, 328, 338, 341, 274, 447, 262, 437, 261, 1776, 263],
            ),
            ("Le soleil brille sur la ville.", &[435, 260, 2608, 3726, 269, 301, 270, 1039, 263]),
            // Emoji -> fallback octets <0xF0><0x9F><0x8C><0x9E>
            ("Bonjour 🌞.", &[875, 1330, 260, 244, 163, 144, 162, 263]),
        ];
        for (text, expected) in cases {
            assert_eq!(&tok.encode(text)[..], *expected, "texte: {text}");
        }
    }

    #[test]
    fn npy_bos_se_charge() {
        let Some(dir) = bundle_dir() else {
            return;
        };
        let (shape, data) = load_npy_f32(&dir.join("bos_before_voice.npy")).unwrap();
        assert_eq!(shape, vec![1, 1, 1024]);
        assert_eq!(data.len(), 1024);
        assert!(data.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn f16_vers_f32() {
        assert_eq!(f16_to_f32(0x3C00), 1.0);
        assert_eq!(f16_to_f32(0xBC00), -1.0);
        assert_eq!(f16_to_f32(0x0000), 0.0);
        assert!((f16_to_f32(0x3555) - 0.333).abs() < 0.001);
        assert!(f16_to_f32(0x7C00).is_infinite());
        assert!(f16_to_f32(0x7C01).is_nan());
    }

    #[test]
    fn copie_de_recouvrement() {
        // src [2,3] dans dst [2,4] rempli de NaN : colonnes 0..3 copiees.
        let src: Vec<f32> = vec![1., 2., 3., 4., 5., 6.];
        let mut dst = vec![f32::NAN; 8];
        copy_overlap_f32(&src, &[2, 3], &mut dst, &[2, 4]);
        assert_eq!(&dst[..3], &[1., 2., 3.]);
        assert!(dst[3].is_nan());
        assert_eq!(&dst[4..7], &[4., 5., 6.]);
        assert!(dst[7].is_nan());
        // src plus grand que dst : tronque proprement.
        let mut small = vec![0f32; 2];
        copy_overlap_f32(&src, &[2, 3], &mut small, &[1, 2]);
        assert_eq!(small, vec![1., 2.]);
    }

    #[test]
    fn etat_de_voix_officiel_se_charge() {
        let dir = std::path::PathBuf::from(r"C:\waly\engines\models\pocket-voices\french_24l");
        let f = dir.join("cosette.safetensors");
        if !f.exists() {
            eprintln!("voix officielles absentes: test saute");
            return;
        }
        let (raw, tensors) = load_safetensors(&f).unwrap();
        assert!(!tensors.is_empty());
        // Au moins un cache d'attention lisible en f32.
        let cache = tensors
            .iter()
            .find(|(k, _)| k.ends_with("/cache"))
            .expect("aucun cache dans l'etat de voix");
        let vals = st_as_f32(&raw, cache.1).unwrap();
        assert!(!vals.is_empty());
    }

    #[test]
    fn resample_sinc_preserve_le_signal() {
        // Sinusoide 440 Hz, 1 s : l'energie doit survivre au 16k -> 24k.
        let sr_in = 16_000u32;
        let input: Vec<f32> = (0..sr_in)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr_in as f32).sin())
            .collect();
        let out = resample_sinc(&input, sr_in, 24_000);
        assert!((out.len() as i64 - 24_000).unsigned_abs() < 4);
        let rms = |v: &[f32]| (v.iter().map(|s| s * s).sum::<f32>() / v.len() as f32).sqrt();
        let (ri, ro) = (rms(&input), rms(&out));
        assert!((ri - ro).abs() / ri < 0.05, "rms {ri} -> {ro}");
    }

    /// Fabrique un signal : `spec` = liste de (durée_ms, amplitude).
    fn signal(sr: usize, spec: &[(usize, f32)]) -> Vec<f32> {
        let mut v = Vec::new();
        for &(ms, amp) in spec {
            let n = sr * ms / 1000;
            // Sinusoïde 200 Hz : RMS = amp/√2 (0,0 = vrai silence).
            v.extend((0..n).map(|i| {
                amp * (2.0 * std::f32::consts::PI * 200.0 * i as f32 / sr as f32).sin()
            }));
        }
        v
    }

    /// Rejoue `audio` dans une porte neuve par chunks, retourne l'émis.
    fn gate_run(sr: usize, audio: &[f32], chunk: usize) -> (Vec<f32>, bool) {
        let mut g = SilenceGate::new(sr);
        let mut out = Vec::new();
        let mut emit = |c: &[f32]| {
            out.extend_from_slice(c);
            true
        };
        for part in audio.chunks(chunk.max(1)) {
            assert!(g.push(part, &mut emit));
        }
        assert!(g.finish(&mut emit));
        (out, g.emitted)
    }

    #[test]
    fn porte_rogne_tete_et_queue() {
        let sr = 24_000;
        // 2 s d'air mort, 500 ms de voix, 1,5 s de queue.
        let audio = signal(sr, &[(2000, 0.0), (500, 0.3), (1500, 0.0)]);
        let (out, emitted) = gate_run(sr, &audio, 1920);
        assert!(emitted);
        // Voix 500 ms + marges ≤ 120 ms de chaque côté (fenêtres de 20 ms).
        let min = sr * 500 / 1000;
        let max = sr * (500 + 2 * 120 + 40) / 1000;
        assert!(out.len() >= min && out.len() <= max, "emis: {} ech.", out.len());
        // Le début émis est la marge de tête : quasi silencieux.
        let head_rms = (out[..sr / 100].iter().map(|s| s * s).sum::<f32>()
            / (sr / 100) as f32)
            .sqrt();
        assert!(head_rms < 0.012, "tete pas silencieuse: {head_rms}");
    }

    #[test]
    fn porte_prise_muette_rien_emis() {
        let sr = 24_000;
        let audio = signal(sr, &[(3000, 0.0)]);
        let (out, emitted) = gate_run(sr, &audio, 512);
        assert!(!emitted);
        assert!(out.is_empty());
    }

    #[test]
    fn porte_preserve_pause_intra_phrase() {
        let sr = 24_000;
        // voix — pause 400 ms — voix : la pause doit survivre ENTIÈRE.
        let audio = signal(sr, &[(300, 0.3), (400, 0.0), (300, 0.3)]);
        let (out, _) = gate_run(sr, &audio, 960);
        // Tout le milieu est conservé : longueur ≈ 1 s (± fenêtres/marges).
        let min = sr * (300 + 400 + 300) / 1000;
        assert!(out.len() >= min, "pause avalee: {} ech.", out.len());
    }

    #[test]
    fn porte_interruption_stoppe() {
        let sr = 24_000;
        let audio = signal(sr, &[(100, 0.3)]);
        let mut g = SilenceGate::new(sr);
        let mut n = 0;
        let mut emit = |_: &[f32]| {
            n += 1;
            false // barge-in immédiat
        };
        assert!(!g.push(&audio, &mut emit));
        assert_eq!(n, 1);
    }

    #[test]
    fn generateur_normal_plausible() {
        let mut g = NormalGen::new();
        let n = 10_000;
        let vals: Vec<f32> = (0..n).map(|_| g.normal(1.0)).collect();
        let mean = vals.iter().sum::<f32>() / n as f32;
        let var = vals.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n as f32;
        assert!(mean.abs() < 0.05, "moyenne {mean}");
        assert!((var - 1.0).abs() < 0.1, "variance {var}");
        assert!(vals.iter().all(|v| v.is_finite()));
    }
}
