//! Embeddings 100 % locaux pour la mémoire sémantique (R2 chantier 4).
//!
//! Modèle : multilingual-e5-small int8 ONNX (384 dims, ~118 Mo,
//! `engines/models/e5-small-int8/`) sur la même onnxruntime.dll que le reste
//! du stack (load-dynamic, DLL signée — stratégie du piège 3). Tokenizer
//! XLM-R unigram lu par la crate `tokenizers` en PUR Rust.
//!
//! Contrat e5 : préfixer `query: ` pour une requête, `passage: ` pour un
//! document — les similarités cosinus n'ont de sens qu'avec ces préfixes.
//! Sortie : vecteur L2-normalisé (le produit scalaire EST le cosinus).

use ort::session::Session;
use ort::value::Tensor;
use tokenizers::Tokenizer;

pub const EMBED_DIM: usize = 384;

/// Initialise ONNX Runtime (une fois par processus). Même ordre de
/// résolution que waly-voice : env `ORT_DYLIB_PATH`, DLL à côté de l'exe,
/// DLL sherpa (1.24 — prioritaire : un seul onnxruntime.dll par processus,
/// sherpa exige l'API 24), enfin la signée Microsoft des modèles.
pub fn init_onnxruntime() -> Result<(), String> {
    static ORT_INIT: std::sync::Once = std::sync::Once::new();
    let mut result = Ok(());
    ORT_INIT.call_once(|| result = init_inner());
    result
}

fn init_inner() -> Result<(), String> {
    // Nom natif de la plateforme (onnxruntime.dll / libonnxruntime.so) et
    // chemins de chemins.rs (Windows : inchangés).
    let lib = crate::chemins::nom_lib("onnxruntime");
    let path = std::env::var("ORT_DYLIB_PATH")
        .ok()
        .or_else(|| {
            std::env::current_exe().ok().and_then(|exe| {
                let beside = exe.with_file_name(&lib);
                beside.exists().then(|| beside.to_string_lossy().into_owned())
            })
        })
        .or_else(|| {
            let sherpa = crate::chemins::lib_sherpa("onnxruntime");
            std::path::Path::new(&sherpa).exists().then_some(sherpa)
        })
        .unwrap_or_else(|| crate::chemins::modele(&lib));
    ort::init_from(&path).commit().map_err(|e| format!("init onnxruntime ({path}): {e}"))?;
    Ok(())
}

pub struct Embedder {
    session: Session,
    tokenizer: Tokenizer,
    needs_token_type: bool,
}

impl Embedder {
    /// Charge le modèle depuis un dossier contenant `model_quantized.onnx`
    /// et `tokenizer.json`. [`init_onnxruntime`] est appelé si besoin.
    pub fn load(dir: &str) -> Result<Self, String> {
        init_onnxruntime()?;
        let model = format!("{dir}/model_quantized.onnx");
        let session = Session::builder()
            .and_then(|b| b.commit_from_file(&model))
            .map_err(|e| format!("chargement {model}: {e}"))?;
        let needs_token_type =
            session.inputs.iter().any(|i| i.name == "token_type_ids");
        let tokenizer = Tokenizer::from_file(format!("{dir}/tokenizer.json"))
            .map_err(|e| format!("tokenizer.json: {e}"))?;
        Ok(Self { session, tokenizer, needs_token_type })
    }

    /// Embedding d'une REQUÊTE de recherche.
    pub fn embed_query(&mut self, text: &str) -> Result<Vec<f32>, String> {
        self.embed(&format!("query: {text}"))
    }

    /// Embedding d'un DOCUMENT indexé (souvenir, note).
    pub fn embed_passage(&mut self, text: &str) -> Result<Vec<f32>, String> {
        self.embed(&format!("passage: {text}"))
    }

    fn embed(&mut self, text: &str) -> Result<Vec<f32>, String> {
        let enc = self.tokenizer.encode(text, true).map_err(|e| e.to_string())?;
        // Borne de sécurité : la mémoire indexe des phrases, pas des romans.
        let n = enc.get_ids().len().min(512);
        let ids: Vec<i64> = enc.get_ids()[..n].iter().map(|&i| i as i64).collect();
        let mask: Vec<i64> = enc.get_attention_mask()[..n].iter().map(|&m| m as i64).collect();

        let input_ids = Tensor::from_array(([1usize, n], ids)).map_err(|e| e.to_string())?;
        let attention =
            Tensor::from_array(([1usize, n], mask.clone())).map_err(|e| e.to_string())?;
        let outputs = if self.needs_token_type {
            let token_type = Tensor::from_array(([1usize, n], vec![0i64; n]))
                .map_err(|e| e.to_string())?;
            self.session
                .run(
                    ort::inputs![
                        "input_ids" => input_ids,
                        "attention_mask" => attention,
                        "token_type_ids" => token_type,
                    ]
                    .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?
        } else {
            self.session
                .run(
                    ort::inputs![
                        "input_ids" => input_ids,
                        "attention_mask" => attention,
                    ]
                    .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?
        };

        // Mean pooling sur les positions actives, puis normalisation L2.
        let hidden = outputs["last_hidden_state"]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        let flat: Vec<f32> = hidden.iter().copied().collect();
        if flat.len() != n * EMBED_DIM {
            return Err(format!(
                "forme de sortie inattendue: {} valeurs pour {n}x{EMBED_DIM}",
                flat.len()
            ));
        }
        let mut pooled = vec![0.0f32; EMBED_DIM];
        let mut count = 0.0f32;
        for (t, &m) in mask.iter().enumerate() {
            if m == 0 {
                continue;
            }
            count += 1.0;
            for d in 0..EMBED_DIM {
                pooled[d] += flat[t * EMBED_DIM + d];
            }
        }
        if count > 0.0 {
            for v in &mut pooled {
                *v /= count;
            }
        }
        let norm = pooled.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
        for v in &mut pooled {
            *v /= norm;
        }
        Ok(pooled)
    }
}

/// Cosinus entre deux vecteurs déjà L2-normalisés.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Sérialise un vecteur au format JSON compact attendu par vec0.
pub fn to_vec_json(v: &[f32]) -> String {
    let mut s = String::with_capacity(v.len() * 10);
    s.push('[');
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!("{x:.6}"));
    }
    s.push(']');
    s
}

// Tests d'intégration (modèle réel requis sur la machine) : voir le banc
// `waly embed-bench` du binaire — les tests unitaires ici resteraient soit
// lents (118 Mo chargés par test), soit factices.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_et_serialisation() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert_eq!(to_vec_json(&[0.5, -0.25]), "[0.500000,-0.250000]");
    }
}
