//! VAD Silero v5 via ONNX Runtime en chargement dynamique.
//!
//! Pourquoi load-dynamic : la onnxruntime.dll officielle est signée Microsoft
//! → passe Smart App Control, et une seule DLL sert à toutes les briques ONNX
//! à venir (Parakeet STT, embeddings). Aucun link statique.
//!
//! Contrat Silero v5 : trames de [`FRAME`] échantillons EXACTEMENT à 16 kHz
//! (32 ms), état récurrent [2,1,128] à recopier entre les appels, sortie =
//! probabilité de parole [0..1]. La segmentation pure est dans
//! [`crate::segment`].

use crate::segment::{FRAME, SAMPLE_RATE};
use ort::session::Session;
use ort::value::Tensor;

const STATE_LEN: usize = 2 * 1 * 128;
/// Contrat Silero v5 @16 kHz : 64 échantillons de CONTEXTE (fin de la trame
/// précédente) concaténés devant chaque trame → entrée [1, 512+64].
/// Sans ce contexte, les probabilités restent écrasées vers 0 (vécu).
const CONTEXT: usize = 64;

/// Initialise ONNX Runtime (à faire UNE fois par processus, avant tout
/// [`SileroVad::new`]). Cherche la DLL : env `ORT_DYLIB_PATH`, puis à côté de
/// l'exe, puis celle FOURNIE AVEC SHERPA (1.24), enfin la signée Microsoft.
///
/// ⚠ Windows ne charge qu'UN `onnxruntime.dll` par processus (résolution des
/// imports PAR NOM) : sherpa-onnx-c-api.dll (bâtie sur l'API 24) doit trouver
/// une ORT ≥ 1.24 — si on initialisait ort avec la 1.22 signée, sherpa
/// hériterait du mauvais module et refuserait de démarrer (vécu 2026-07-04).
/// D'où la priorité à la DLL bundlée sherpa dès qu'elle est présente.
pub fn init_onnxruntime() -> Result<(), Box<dyn std::error::Error>> {
    // Délègue à waly-core (même ordre de résolution de DLL) : UNE seule
    // init ort par processus — deux `Once` séparés (VAD ici, embedder côté
    // core) appelleraient `ort::init_from` deux fois.
    waly_core::embed::init_onnxruntime().map_err(Into::into)
}

pub struct SileroVad {
    session: Session,
    state: Vec<f32>,
    context: Vec<f32>,
}

impl SileroVad {
    /// Charge le modèle (`silero_vad.onnx`). [`init_onnxruntime`] doit avoir
    /// été appelé avant.
    pub fn new(model_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let session = Session::builder()?.commit_from_file(model_path)?;
        Ok(Self { session, state: vec![0.0; STATE_LEN], context: vec![0.0; CONTEXT] })
    }

    /// À appeler entre deux énoncés (l'état récurrent porte le contexte).
    pub fn reset(&mut self) {
        self.state.fill(0.0);
        self.context.fill(0.0);
    }

    /// Probabilité de parole pour une trame de [`FRAME`] échantillons
    /// (16 kHz, f32 dans [-1, 1]).
    pub fn process(&mut self, frame: &[f32]) -> Result<f32, Box<dyn std::error::Error>> {
        assert_eq!(frame.len(), FRAME, "Silero v5 exige des trames de {FRAME}");
        let mut with_context = Vec::with_capacity(CONTEXT + FRAME);
        with_context.extend_from_slice(&self.context);
        with_context.extend_from_slice(frame);
        self.context.copy_from_slice(&frame[FRAME - CONTEXT..]);
        let input = Tensor::from_array(([1usize, CONTEXT + FRAME], with_context))?;
        let state = Tensor::from_array(([2usize, 1, 128], self.state.clone()))?;
        let sr = Tensor::from_array(([1usize], vec![SAMPLE_RATE]))?;
        let outputs = self.session.run(ort::inputs![
            "input" => input,
            "state" => state,
            "sr" => sr,
        ]?)?;
        let prob = outputs["output"].try_extract_tensor::<f32>()?;
        let prob = *prob.iter().next().ok_or("sortie VAD vide")?;
        let new_state = outputs["stateN"].try_extract_tensor::<f32>()?;
        self.state.clear();
        self.state.extend(new_state.iter().copied());
        Ok(prob)
    }
}
