//! Wake word « Waly » (R6b) — pipeline openWakeWord ONNX en streaming.
//!
//! Trois étages sur la MÊME onnxruntime.dll signée que le reste (piège n°3,
//! DLL jamais d'exe), verdicts au banc `lab/eveil-banc/README.md` :
//! 1. `melspectrogram.onnx` — audio 16 kHz À L'ÉCHELLE int16 (±32768),
//!    trames de 10 ms ; mesuré : `frames = N/160 − 3` → un chunk de
//!    [`CHUNK`] échantillons avec [`CONTEXT`] de contexte = 8 trames
//!    exactes. Transformée openWakeWord `x/10 + 2`.
//! 2. `embedding_model.onnx` (speech-embedding Google, gelé) — fenêtre de
//!    76 trames → 96-d, un pas par chunk (80 ms).
//! 3. `waly_wake.onnx` (le nôtre, banc v4, standardisation cuite dans le
//!    graphe) — contexte 16×96 → score sigmoïde.
//!
//! Coût mesuré : ~1,5 ms par pas de 80 ms (~1,4 % d'un cœur en continu) ;
//! 46 Mo de pic. La porte VAD coûterait PLUS cher que le pipeline (mesuré) —
//! on tourne toujours-actif, la décision fait le tri.
//!
//! L'audio de veille vit en RAM (anneau chez l'appelant), n'est JAMAIS
//! écrit — règle produit (plan R6b).

#[cfg(feature = "service")]
use ort::session::Session;
#[cfg(feature = "service")]
use ort::value::Tensor;

#[cfg(feature = "service")]
type Err = Box<dyn std::error::Error>;

/// Un pas de wake : 80 ms à 16 kHz.
pub const CHUNK: usize = 1280;
/// Contexte mel à préfixer au chunk (mesure banc : 1760 éch. = 8 trames).
pub const CONTEXT: usize = 480;
#[cfg(feature = "service")]
const MEL_BANDS: usize = 32;
#[cfg(feature = "service")]
const EMB_WIN: usize = 76;
#[cfg(feature = "service")]
const EMB_DIM: usize = 96;
#[cfg(feature = "service")]
const CLF_STEPS: usize = 16;

/// Décision pure (testable sans ort) : seuil + patience (pas consécutifs)
/// + réfractaire (pas de re-déclenchement en rafale).
#[derive(Debug)]
pub struct WakeDecision {
    pub seuil: f32,
    pub patience: u32,
    /// Pas de silence imposés après un déclenchement (25 pas = 2 s).
    pub refractaire: u32,
    consecutifs: u32,
    gel: u32,
}

impl WakeDecision {
    pub fn new(seuil: f32, patience: u32, refractaire: u32) -> Self {
        Self { seuil, patience, refractaire, consecutifs: 0, gel: 0 }
    }

    /// Nourrit un score, dit si l'éveil se déclenche à CE pas.
    pub fn push(&mut self, score: f32) -> bool {
        if self.gel > 0 {
            self.gel -= 1;
            self.consecutifs = 0;
            return false;
        }
        if score >= self.seuil {
            self.consecutifs += 1;
            if self.consecutifs >= self.patience {
                self.consecutifs = 0;
                self.gel = self.refractaire;
                return true;
            }
        } else {
            self.consecutifs = 0;
        }
        false
    }
}

#[cfg(feature = "service")]
pub struct WakeDetector {
    melspec: Session,
    embedding: Session,
    classifier: Session,
    /// Fin du chunk précédent (contexte mel).
    context: Vec<f32>,
    /// 76 dernières trames mel (glissant).
    mel_tail: Vec<f32>,
    /// 16 derniers embeddings (contexte classifieur).
    emb_hist: Vec<f32>,
    /// Trames accumulées avant d'avoir un contexte plein.
    chauffe: usize,
    pub decision: WakeDecision,
}

#[cfg(feature = "service")]
fn session(path: &std::path::Path) -> Result<Session, Err> {
    // 1 thread : modèles minuscules + piège du pool ort qui spin-wait (R4).
    Ok(Session::builder()?
        .with_intra_threads(1)?
        .with_inter_threads(1)?
        .commit_from_file(path)
        .map_err(|e| format!("{}: {e}", path.display()))?)
}

#[cfg(feature = "service")]
impl WakeDetector {
    /// `dir` : `engines/models/openwakeword` (melspectrogram + embedding +
    /// waly_wake). [`crate::vad::init_onnxruntime`] doit avoir été appelé.
    pub fn new(dir: &std::path::Path, seuil: f32) -> Result<Self, Err> {
        Ok(Self {
            melspec: session(&dir.join("melspectrogram.onnx"))?,
            embedding: session(&dir.join("embedding_model.onnx"))?,
            classifier: session(&dir.join("waly_wake.onnx"))?,
            context: vec![0.0; CONTEXT],
            mel_tail: Vec::with_capacity(EMB_WIN * MEL_BANDS),
            emb_hist: vec![0.0; CLF_STEPS * EMB_DIM],
            chauffe: 0,
            decision: WakeDecision::new(seuil, 2, 25),
        })
    }

    /// Un pas de 80 ms : `chunk` = [`CHUNK`] échantillons 16 kHz f32 [-1,1].
    /// Retourne Some(score) au déclenchement.
    pub fn push(&mut self, chunk: &[f32]) -> Result<Option<f32>, Err> {
        assert_eq!(chunk.len(), CHUNK, "wake : chunks de {CHUNK} exigés");
        // Mel du chunk avec contexte (échelle int16 + transformée oww).
        let mut with_ctx = Vec::with_capacity(CONTEXT + CHUNK);
        with_ctx.extend_from_slice(&self.context);
        with_ctx.extend_from_slice(chunk);
        self.context.copy_from_slice(&chunk[CHUNK - CONTEXT..]);
        let scaled: Vec<f32> = with_ctx.iter().map(|s| s * 32768.0).collect();
        let n = scaled.len();
        let input = Tensor::from_array(([1usize, n], scaled))?;
        let name = self.melspec.inputs[0].name.clone();
        let out = self.melspec.run(ort::inputs![name.as_str() => input]?)?;
        let mel = out[0].try_extract_tensor::<f32>()?;
        let mel: Vec<f32> = mel.iter().map(|v| v / 10.0 + 2.0).collect();
        let frames = mel.len() / MEL_BANDS;
        let take = frames.min(8);
        self.mel_tail.extend_from_slice(&mel[(frames - take) * MEL_BANDS..]);
        let len = self.mel_tail.len();
        if len > EMB_WIN * MEL_BANDS {
            self.mel_tail.drain(..len - EMB_WIN * MEL_BANDS);
        }
        if self.mel_tail.len() < EMB_WIN * MEL_BANDS {
            return Ok(None); // chauffe (~610 ms)
        }
        // Un embedding par pas.
        let input =
            Tensor::from_array((vec![1usize, EMB_WIN, MEL_BANDS, 1], self.mel_tail.clone()))?;
        let name = self.embedding.inputs[0].name.clone();
        let out = self.embedding.run(ort::inputs![name.as_str() => input]?)?;
        let emb = out[0].try_extract_tensor::<f32>()?;
        self.emb_hist.drain(..EMB_DIM);
        self.emb_hist.extend(emb.iter().copied());
        // Le contexte classifieur doit être plein de VRAIS embeddings avant
        // de scorer (16 pas = 1,28 s après la chauffe mel).
        if self.chauffe < CLF_STEPS {
            self.chauffe += 1;
            return Ok(None);
        }
        let input =
            Tensor::from_array((vec![1usize, CLF_STEPS, EMB_DIM], self.emb_hist.clone()))?;
        let name = self.classifier.inputs[0].name.clone();
        let out = self.classifier.run(ort::inputs![name.as_str() => input]?)?;
        let score = out[0]
            .try_extract_tensor::<f32>()?
            .iter()
            .next()
            .copied()
            .ok_or("wake : sortie classifieur vide")?;
        Ok(if self.decision.push(score) { Some(score) } else { None })
    }
}

#[cfg(test)]
mod tests {
    use super::WakeDecision;

    #[test]
    fn patience_exigee() {
        let mut d = WakeDecision::new(0.9, 2, 25);
        assert!(!d.push(0.95)); // 1 pas chaud : pas encore
        assert!(!d.push(0.5)); // retombé : compteur remis
        assert!(!d.push(0.95));
        assert!(d.push(0.95)); // 2 consécutifs : éveil
    }

    #[test]
    fn refractaire_apres_declenchement() {
        let mut d = WakeDecision::new(0.9, 2, 3);
        assert!(!d.push(0.95));
        assert!(d.push(0.95));
        // Gel : même des scores hauts ne redéclenchent pas.
        assert!(!d.push(0.99));
        assert!(!d.push(0.99));
        assert!(!d.push(0.99));
        // Gel purgé : la patience se recompte depuis zéro.
        assert!(!d.push(0.99));
        assert!(d.push(0.99));
    }

    #[test]
    fn sous_le_seuil_jamais() {
        let mut d = WakeDecision::new(0.9, 2, 25);
        for _ in 0..100 {
            assert!(!d.push(0.89));
        }
    }
}
