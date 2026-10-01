//! Pipeline openWakeWord en ONNX pur (ort load-dynamic, DLL signée déjà en
//! place — SAC-safe, piège n°3 : jamais d'exe tiers).
//!
//! Trois étages, reproduits de `openwakeword/utils.py` (AudioFeatures) :
//! 1. `melspectrogram.onnx` : audio 16 kHz À L'ÉCHELLE int16 (±32768,
//!    PAS [-1,1] — vécu de la référence Python) → trames mel 32 bandes,
//!    hop 10 ms ; transformée `x/10 + 2` avant l'embedding.
//! 2. `embedding_model.onnx` (speech-embedding Google, gelé) : fenêtres de
//!    76 trames mel `[N,76,32,1]` → vecteur 96-d ; UN pas tous les
//!    8 trames = 80 ms.
//! 3. classifieur (le seul qu'on entraîne) : `[1,16,96]` (16 pas ≈ 1,28 s
//!    de contexte) → score sigmoïde.

use anyhow::{bail, Context, Result};
use ort::session::Session;
use ort::value::Tensor;

pub const SR: usize = 16_000;
pub const MEL_BANDS: usize = 32;
/// Hop du mel-spectrogramme : 10 ms.
pub const MEL_HOP: usize = 160;
pub const EMB_WIN: usize = 76;
pub const EMB_STRIDE: usize = 8;
pub const EMB_DIM: usize = 96;
pub const CLF_STEPS: usize = 16;
pub const FEAT_LEN: usize = CLF_STEPS * EMB_DIM; // 1536
/// Trames mel couvertes par UNE fenêtre classifieur : 76 + 15×8 = 196.
pub const CLF_MEL_FRAMES: usize = EMB_WIN + (CLF_STEPS - 1) * EMB_STRIDE;
/// Longueur de clip d'entraînement : 2,0 s → 198 trames mel ≥ 196.
pub const CLIP_SAMPLES: usize = 2 * SR;

pub struct Oww {
    melspec: Session,
    embedding: Session,
}

fn tiny_session(path: &std::path::Path) -> Result<Session> {
    // Modèles minuscules (~1 Mo) : 1 thread suffit et évite le pool ort qui
    // spin-wait (piège R4).
    Ok(Session::builder()?
        .with_intra_threads(1)?
        .with_inter_threads(1)?
        .commit_from_file(path)
        .with_context(|| format!("chargement {}", path.display()))?)
}

impl Oww {
    /// `dir` : dossier contenant `melspectrogram.onnx` + `embedding_model.onnx`.
    /// `waly_voice::vad::init_onnxruntime()` doit avoir été appelé avant.
    pub fn new(dir: &std::path::Path) -> Result<Self> {
        Ok(Self {
            melspec: tiny_session(&dir.join("melspectrogram.onnx"))?,
            embedding: tiny_session(&dir.join("embedding_model.onnx"))?,
        })
    }

    /// Mel-spectrogramme du clip entier : `frames × 32` aplati, transformée
    /// openWakeWord appliquée. `audio` en f32 [-1,1] (l'échelle int16 est
    /// appliquée ici).
    pub fn melspec(&self, audio: &[f32]) -> Result<Vec<f32>> {
        if audio.len() < 640 {
            bail!("clip trop court pour le mel ({} éch.)", audio.len());
        }
        let scaled: Vec<f32> = audio.iter().map(|s| s * 32768.0).collect();
        let n = scaled.len();
        let input = Tensor::from_array(([1usize, n], scaled))?;
        let name = self.melspec.inputs[0].name.clone();
        let outputs = self
            .melspec
            .run(ort::inputs![name.as_str() => input]?)?;
        let out = outputs[0].try_extract_tensor::<f32>()?;
        Ok(out.iter().map(|v| v / 10.0 + 2.0).collect())
    }

    /// Embeddings de TOUTES les fenêtres de 76 trames (stride 8), en un seul
    /// run batché : `n_windows × 96` aplati.
    pub fn embeddings(&self, mel: &[f32]) -> Result<Vec<f32>> {
        let frames = mel.len() / MEL_BANDS;
        if frames < EMB_WIN {
            bail!("{frames} trames mel < fenêtre de {EMB_WIN}");
        }
        let n_win = (frames - EMB_WIN) / EMB_STRIDE + 1;
        let mut batch = Vec::with_capacity(n_win * EMB_WIN * MEL_BANDS);
        for w in 0..n_win {
            let start = w * EMB_STRIDE * MEL_BANDS;
            batch.extend_from_slice(&mel[start..start + EMB_WIN * MEL_BANDS]);
        }
        let input = Tensor::from_array((vec![n_win, EMB_WIN, MEL_BANDS, 1], batch))?;
        let name = self.embedding.inputs[0].name.clone();
        let outputs = self
            .embedding
            .run(ort::inputs![name.as_str() => input]?)?;
        let out = outputs[0].try_extract_tensor::<f32>()?;
        Ok(out.iter().copied().collect())
    }

    /// Feature 16×96 de la DERNIÈRE fenêtre classifieur (entraînement : le
    /// mot est placé en fin de clip).
    pub fn last_features(&self, audio: &[f32]) -> Result<Vec<f32>> {
        let mel = self.melspec(audio)?;
        let frames = mel.len() / MEL_BANDS;
        if frames < CLF_MEL_FRAMES {
            bail!("{frames} trames mel < {CLF_MEL_FRAMES} requises");
        }
        let tail = &mel[(frames - CLF_MEL_FRAMES) * MEL_BANDS..];
        let emb = self.embeddings(tail)?;
        debug_assert_eq!(emb.len(), FEAT_LEN);
        Ok(emb)
    }

    /// Toutes les fenêtres classifieur d'un clip (stride 1 embedding = 80 ms) :
    /// vecteur de features 1536, une par pas.
    pub fn all_features(&self, audio: &[f32]) -> Result<Vec<Vec<f32>>> {
        let mel = self.melspec(audio)?;
        let emb = self.embeddings(&mel)?;
        let n_emb = emb.len() / EMB_DIM;
        if n_emb < CLF_STEPS {
            bail!("{n_emb} embeddings < {CLF_STEPS} requis");
        }
        let mut rows = Vec::with_capacity(n_emb - CLF_STEPS + 1);
        for i in 0..=n_emb - CLF_STEPS {
            rows.push(emb[i * EMB_DIM..(i + CLF_STEPS) * EMB_DIM].to_vec());
        }
        Ok(rows)
    }
}

pub struct Classifier {
    session: Session,
}

impl Classifier {
    pub fn new(path: &std::path::Path) -> Result<Self> {
        Ok(Self { session: tiny_session(path)? })
    }

    /// Score sigmoïde d'UNE feature 16×96.
    pub fn score(&self, feat: &[f32]) -> Result<f32> {
        debug_assert_eq!(feat.len(), FEAT_LEN);
        let input = Tensor::from_array((vec![1usize, CLF_STEPS, EMB_DIM], feat.to_vec()))?;
        let name = self.session.inputs[0].name.clone();
        let outputs = self
            .session
            .run(ort::inputs![name.as_str() => input]?)?;
        let out = outputs[0].try_extract_tensor::<f32>()?;
        out.iter().next().copied().context("sortie classifieur vide")
    }
}
