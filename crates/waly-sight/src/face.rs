//! Détection de visage YuNet (opencv_zoo 2023mar, 232 Ko) — R4 GATE D.
//!
//! La brique « présence » de la boucle rapide : visage(s) + 5 points
//! (yeux, nez, coins de bouche) qui serviront à l'attention au chantier 3.
//! Tourne sur la même onnxruntime.dll signée que le reste (load-dynamic).
//!
//! Décodage fidèle à l'implémentation OpenCV FaceDetectorYN : trois têtes
//! (strides 8/16/32), score = sqrt(cls × obj), bbox = (cellule + offset) ×
//! stride, exp() pour la taille, puis NMS.

use ort::session::Session;
use ort::value::Tensor;

/// Résolution d'entrée du modèle 2023mar publié : FIXE 640×640 (vérifié au
/// banc le 2026-07-07 — le modèle refuse toute autre taille). La frame
/// source est réduite en nearest-neighbor AVEC letterbox (échelle uniforme,
/// bandes noires) pour ne pas déformer les visages ; les coordonnées sont
/// remises à l'échelle source en sortie.
pub const INPUT_W: usize = 640;
pub const INPUT_H: usize = 640;
const STRIDES: [usize; 3] = [8, 16, 32];
const SCORE_THRESHOLD: f32 = 0.6;
const NMS_IOU: f32 = 0.3;

#[derive(Debug, Clone)]
pub struct Face {
    /// x, y, largeur, hauteur en pixels de l'image SOURCE.
    pub bbox: [f32; 4],
    pub score: f32,
    /// œil droit, œil gauche, nez, coin bouche droit, coin bouche gauche.
    pub kps: [[f32; 2]; 5],
}

impl Face {
    /// Décalage latéral du nez par rapport au milieu des yeux, normalisé
    /// par l'écart inter-oculaire. ~0 = tête de face, |r| grand = profil.
    pub fn ratio_lateral(&self) -> f32 {
        let (oeil_d, oeil_g, nez) = (self.kps[0], self.kps[1], self.kps[2]);
        let ecart = ((oeil_g[0] - oeil_d[0]).powi(2) + (oeil_g[1] - oeil_d[1]).powi(2))
            .sqrt()
            .max(1.0);
        let milieu_x = (oeil_d[0] + oeil_g[0]) / 2.0;
        (nez[0] - milieu_x) / ecart
    }

    /// Heuristique d'attention : la TÊTE est-elle orientée vers l'écran ?
    /// (Approximation assumée — ce n'est pas du suivi de regard.)
    pub fn vers_ecran(&self) -> bool {
        self.ratio_lateral().abs() < 0.35
    }
}

/// Résout onnxruntime.dll comme waly-core/waly-voice : env `ORT_DYLIB_PATH`,
/// DLL à côté de l'exe, DLL sherpa, enfin la signée Microsoft des modèles.
pub fn init_onnxruntime() -> Result<(), String> {
    static ORT_INIT: std::sync::Once = std::sync::Once::new();
    let mut result = Ok(());
    ORT_INIT.call_once(|| result = init_inner());
    result
}

/// Racine de Waly — MIROIR de `waly_core::chemins::racine` (waly-sight ne
/// dépend pas de waly-core) : `WALY_HOME` › Windows `C:\waly` ›
/// `$XDG_DATA_HOME/waly` › `$HOME/.local/share/waly`. Toute évolution de la
/// règle se fait AUX DEUX endroits.
pub(crate) fn racine() -> std::path::PathBuf {
    let var = |n: &str| std::env::var(n).ok().filter(|v| !v.trim().is_empty());
    if let Some(h) = var("WALY_HOME") {
        return h.into();
    }
    if cfg!(windows) {
        return std::path::PathBuf::from(r"C:\waly");
    }
    if let Some(x) = var("XDG_DATA_HOME") {
        return std::path::PathBuf::from(x).join("waly");
    }
    std::path::PathBuf::from(var("HOME").unwrap_or_else(|| ".".into()))
        .join(".local")
        .join("share")
        .join("waly")
}

fn init_inner() -> Result<(), String> {
    // Nom natif (onnxruntime.dll / libonnxruntime.so) ; Windows inchangé.
    let lib = format!("{}onnxruntime{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX);
    let engines = racine().join("engines");
    let path = std::env::var("ORT_DYLIB_PATH")
        .ok()
        .or_else(|| {
            std::env::current_exe().ok().and_then(|exe| {
                let beside = exe.with_file_name(&lib);
                beside.exists().then(|| beside.to_string_lossy().into_owned())
            })
        })
        .or_else(|| {
            let sherpa = engines.join("sherpa").join("lib").join(&lib);
            sherpa.exists().then(|| sherpa.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| engines.join("models").join(&lib).to_string_lossy().into_owned());
    ort::init_from(&path).commit().map_err(|e| format!("init onnxruntime ({path}): {e}"))?;
    Ok(())
}

pub struct FaceDetector {
    session: Session,
    input_name: String,
}

impl FaceDetector {
    pub fn load(model_path: &str) -> Result<Self, String> {
        init_onnxruntime()?;
        // Threads bridés : les modèles sont minuscules et la boucle tourne
        // en continu — le pool par défaut d'ort SPIN-WAIT entre les
        // inférences et brûlait ~2 cœurs à vide (mesuré 2026-07-07).
        let session = Session::builder()
            .and_then(|b| b.with_intra_threads(2))
            .and_then(|b| b.with_inter_threads(1))
            .and_then(|b| b.commit_from_file(model_path))
            .map_err(|e| format!("chargement {model_path}: {e}"))?;
        let input_name = session
            .inputs
            .first()
            .map(|i| i.name.clone())
            .ok_or_else(|| "modèle sans entrée".to_string())?;
        Ok(Self { session, input_name })
    }

    /// Détecte les visages sur une frame RGB8 entrelacée de taille libre.
    pub fn detect(&mut self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<Face>, String> {
        let (w, h) = (width as usize, height as usize);
        if rgb.len() < w * h * 3 {
            return Err(format!("frame RGB tronquée: {} octets pour {w}x{h}", rgb.len()));
        }
        // Letterbox nearest (échelle uniforme, reste noir) + RGB→BGR
        // (contrat OpenCV) + HWC→CHW, valeurs 0..255.
        let hw = INPUT_W * INPUT_H;
        let mut data = vec![0f32; 3 * hw];
        let scale = (INPUT_W as f32 / w as f32).min(INPUT_H as f32 / h as f32);
        let (dst_w, dst_h) =
            ((w as f32 * scale) as usize, (h as f32 * scale) as usize);
        for y in 0..dst_h.min(INPUT_H) {
            let src_y = (((y as f32 + 0.5) / scale) as usize).min(h - 1);
            for x in 0..dst_w.min(INPUT_W) {
                let src_x = (((x as f32 + 0.5) / scale) as usize).min(w - 1);
                let p = (src_y * w + src_x) * 3;
                let d = y * INPUT_W + x;
                data[d] = rgb[p + 2] as f32; // B
                data[hw + d] = rgb[p + 1] as f32; // G
                data[2 * hw + d] = rgb[p] as f32; // R
            }
        }
        // Remise à l'échelle source : une seule échelle (letterbox).
        let sx = 1.0 / scale;
        let sy = 1.0 / scale;
        let input = Tensor::from_array(([1usize, 3, INPUT_H, INPUT_W], data))
            .map_err(|e| e.to_string())?;
        let outputs = self
            .session
            .run(
                ort::inputs![self.input_name.as_str() => input].map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;

        let mut faces: Vec<Face> = Vec::new();
        for &stride in &STRIDES {
            let cols = INPUT_W / stride;
            let rows = INPUT_H / stride;
            let n = cols * rows;
            let cls = extract(&outputs, &format!("cls_{stride}"), n)?;
            let obj = extract(&outputs, &format!("obj_{stride}"), n)?;
            let bbox = extract(&outputs, &format!("bbox_{stride}"), 4 * n)?;
            let kps = extract(&outputs, &format!("kps_{stride}"), 10 * n)?;
            for i in 0..n {
                let score = (cls[i].clamp(0.0, 1.0) * obj[i].clamp(0.0, 1.0)).sqrt();
                if score < SCORE_THRESHOLD {
                    continue;
                }
                let (col, row) = ((i % cols) as f32, (i / cols) as f32);
                let s = stride as f32;
                let cx = (col + bbox[4 * i]) * s;
                let cy = (row + bbox[4 * i + 1]) * s;
                let bw = bbox[4 * i + 2].exp() * s;
                let bh = bbox[4 * i + 3].exp() * s;
                let mut face = Face {
                    bbox: [(cx - bw / 2.0) * sx, (cy - bh / 2.0) * sy, bw * sx, bh * sy],
                    score,
                    kps: [[0.0; 2]; 5],
                };
                for k in 0..5 {
                    face.kps[k] = [
                        (col + kps[10 * i + 2 * k]) * s * sx,
                        (row + kps[10 * i + 2 * k + 1]) * s * sy,
                    ];
                }
                faces.push(face);
            }
        }
        Ok(nms(faces))
    }
}

fn extract(
    outputs: &ort::session::SessionOutputs<'_, '_>,
    name: &str,
    expected: usize,
) -> Result<Vec<f32>, String> {
    let view = outputs
        .get(name)
        .ok_or_else(|| format!("sortie {name} absente du modèle"))?
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("extraction {name}: {e}"))?;
    let flat: Vec<f32> = view.iter().copied().collect();
    if flat.len() != expected {
        return Err(format!("{name}: {} valeurs, {expected} attendues", flat.len()));
    }
    Ok(flat)
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let x1 = a[0].max(b[0]);
    let y1 = a[1].max(b[1]);
    let x2 = (a[0] + a[2]).min(b[0] + b[2]);
    let y2 = (a[1] + a[3]).min(b[1] + b[3]);
    let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    let union = a[2] * a[3] + b[2] * b[3] - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

fn nms(mut faces: Vec<Face>) -> Vec<Face> {
    faces.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Face> = Vec::new();
    for f in faces {
        if kept.iter().all(|k| iou(&k.bbox, &f.bbox) < NMS_IOU) {
            kept.push(f);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nms_garde_le_meilleur_des_recouvrants() {
        let a = Face { bbox: [10.0, 10.0, 100.0, 100.0], score: 0.9, kps: [[0.0; 2]; 5] };
        let b = Face { bbox: [15.0, 12.0, 100.0, 100.0], score: 0.7, kps: [[0.0; 2]; 5] };
        let c = Face { bbox: [300.0, 300.0, 80.0, 80.0], score: 0.8, kps: [[0.0; 2]; 5] };
        let kept = nms(vec![b.clone(), a.clone(), c.clone()]);
        assert_eq!(kept.len(), 2);
        assert!((kept[0].score - 0.9).abs() < 1e-6);
        assert!((kept[1].score - 0.8).abs() < 1e-6);
    }

    #[test]
    fn iou_disjoints_nulle() {
        assert_eq!(iou(&[0.0, 0.0, 10.0, 10.0], &[100.0, 100.0, 10.0, 10.0]), 0.0);
    }

    #[test]
    fn grilles_strides_tombent_juste() {
        for s in STRIDES {
            assert_eq!(INPUT_W % s, 0);
            assert_eq!(INPUT_H % s, 0);
        }
    }
}
