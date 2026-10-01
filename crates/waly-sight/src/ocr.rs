//! OCR local — R5 chantier 1 (GATE 2 : option B, ONNX via `ort`).
//!
//! Pipeline PaddleOCR en deux temps, sur la même onnxruntime.dll signée que
//! YuNet/FER+ (load-dynamic — piège 3, aucune install système, choix B de
//! Michée) :
//!   1. DÉTECTION (DBNet, `det.onnx`) : carte de probabilité par pixel →
//!      seuil → composantes connexes → boîtes de texte (le texte d'écran est
//!      axis-aligné, donc boîte droite : pas besoin du contour/unclip
//!      d'OpenCV).
//!   2. RECONNAISSANCE (CRNN, `rec_en.onnx`, dico ASCII 95) : chaque boîte
//!      redimensionnée h=48 → séquence de classes → décodage CTC glouton.
//!
//! Modèle de reconnaissance = anglais/ASCII : EXCELLENT sur code, erreurs,
//! URL, UI (le cas dominant « lis-moi ça »). Le FRANÇAIS accentué est le point
//! faible ASSUMÉ — couvert par le VLM (GATE 3 : qwen3vl lit le français
//! parfaitement) via le routage OCR-first → VLM du chantier 2. Lever plus
//! tard = un modèle rec latin/FR (aucun ONNX propre trouvé au lancement R5).

use ort::session::Session;
use ort::value::Tensor;

use crate::face::init_onnxruntime;
use crate::screen::Shot;

/// Une zone de texte lue : contenu, boîte (x, y, w, h) en pixels source,
/// confiance moyenne du décodage CTC (0..1).
#[derive(Debug, Clone)]
pub struct Word {
    pub text: String,
    pub bbox: [i32; 4],
    pub conf: f32,
}

/// Résultat d'une lecture : texte remis en ordre de lecture (lignes séparées
/// par `\n`), zones brutes, confiance moyenne globale.
#[derive(Debug, Clone)]
pub struct OcrResult {
    pub text: String,
    pub words: Vec<Word>,
    pub mean_conf: f32,
}

// Détection : côté long borné (compromis rappel/latence, défaut PaddleOCR).
const DET_LIMIT: u32 = 960;
const DET_THRESHOLD: f32 = 0.3;
const MIN_BLOB: usize = 8; // pixels : filtre le bruit
// Reconnaissance : hauteur d'entrée fixe du PP-OCRv3 rec.
const REC_H: u32 = 48;
// Largeur max d'une ligne pour la reconnaissance. 640 suffisait aux lignes
// d'écran courtes, mais ÉCRASAIT une ligne pleine de page (PDF joint) ->
// charabia (vécu 2026-09-30). Le modèle rec accepte une largeur variable.
const REC_W_MAX: u32 = 3200;

pub struct Ocr {
    det: Session,
    det_in: String,
    det_out: String,
    rec: Session,
    rec_in: String,
    rec_out: String,
    /// Table de décodage CTC : index 0 = blanc, 1..=95 = dico, 96 = espace.
    labels: Vec<String>,
}

impl Ocr {
    pub fn load(det_path: &str, rec_path: &str, dict_path: &str) -> Result<Self, String> {
        init_onnxruntime()?;
        // OCR = à la demande (pas une boucle continue) → on peut prendre 4
        // threads intra pour la vitesse sans le spin-wait de la perception.
        let build = |p: &str| -> Result<Session, String> {
            Session::builder()
                .and_then(|b| b.with_intra_threads(4))
                .and_then(|b| b.with_inter_threads(1))
                .and_then(|b| b.commit_from_file(p))
                .map_err(|e| format!("chargement {p}: {e}"))
        };
        let det = build(det_path)?;
        let rec = build(rec_path)?;
        let det_in = det.inputs.first().map(|i| i.name.clone()).ok_or("det sans entrée")?;
        let rec_in = rec.inputs.first().map(|i| i.name.clone()).ok_or("rec sans entrée")?;
        let det_out = det.outputs.first().map(|o| o.name.clone()).ok_or("det sans sortie")?;
        let rec_out = rec.outputs.first().map(|o| o.name.clone()).ok_or("rec sans sortie")?;
        let dico = std::fs::read_to_string(dict_path)
            .map_err(|e| format!("lecture dico {dict_path}: {e}"))?;
        // [blanc] + dico + [espace] = classes CTC (contrat PaddleOCR use_space_char).
        let mut labels = Vec::with_capacity(100);
        labels.push(String::new()); // 0 = blanc CTC
        for l in dico.lines() {
            labels.push(l.to_string());
        }
        labels.push(" ".to_string());
        Ok(Self { det, det_in, det_out, rec, rec_in, rec_out, labels })
    }

    /// Chemins par défaut (surcharge WALY_OCR_DIR).
    pub fn load_default() -> Result<Self, String> {
        let dir = std::env::var("WALY_OCR_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| crate::face::racine().join("engines").join("models").join("ocr"));
        // Séparateurs natifs (Linux : « \ » n'est pas un séparateur).
        let f = |n: &str| dir.join(n).to_string_lossy().into_owned();
        Self::load(&f("det.onnx"), &f("rec_en.onnx"), &f("en_dict.txt"))
    }

    /// Lit tout le texte d'un cliché RGB8.
    pub fn read(&mut self, shot: &Shot) -> Result<OcrResult, String> {
        let boxes = self.detect(shot)?;
        let mut words = Vec::with_capacity(boxes.len());
        for b in boxes {
            let (t, c) = self.recognize(shot, b)?;
            let t = t.trim().to_string();
            if !t.is_empty() {
                words.push(Word { text: t, bbox: b, conf: c });
            }
        }
        // Ordre de lecture : par ligne (bucket vertical) puis gauche→droite.
        let tol = median_height(&words).max(8);
        words.sort_by_key(|w| (w.bbox[1] / tol, w.bbox[0]));
        let mut text = String::new();
        let mut last_row: Option<i32> = None;
        for w in &words {
            let row = w.bbox[1] / tol;
            match last_row {
                Some(r) if r == row => text.push(' '),
                Some(_) => text.push('\n'),
                None => {}
            }
            text.push_str(&w.text);
            last_row = Some(row);
        }
        let mean_conf = if words.is_empty() {
            0.0
        } else {
            words.iter().map(|w| w.conf).sum::<f32>() / words.len() as f32
        };
        Ok(OcrResult { text, words, mean_conf })
    }

    /// DÉTECTION → boîtes (x, y, w, h) en pixels SOURCE.
    fn detect(&mut self, shot: &Shot) -> Result<Vec<[i32; 4]>, String> {
        let src = image::RgbImage::from_raw(shot.width, shot.height, shot.rgb.clone())
            .ok_or("cliché RGB invalide")?;
        // Réduire côté long ≤ DET_LIMIT, dimensions multiples de 32.
        let long = shot.width.max(shot.height) as f32;
        let scale = (DET_LIMIT as f32 / long).min(1.0);
        let rw = (((shot.width as f32 * scale) as u32) / 32).max(1) * 32;
        let rh = (((shot.height as f32 * scale) as u32) / 32).max(1) * 32;
        let resized =
            image::imageops::resize(&src, rw, rh, image::imageops::FilterType::Triangle);
        // Normalisation ImageNet (contrat PaddleOCR det), RGB, NCHW.
        let mean = [0.485f32, 0.456, 0.406];
        let std = [0.229f32, 0.224, 0.225];
        let hw = (rw * rh) as usize;
        let mut data = vec![0f32; 3 * hw];
        for y in 0..rh {
            for x in 0..rw {
                let p = resized.get_pixel(x, y);
                let d = (y * rw + x) as usize;
                for c in 0..3 {
                    data[c * hw + d] = (p[c] as f32 / 255.0 - mean[c]) / std[c];
                }
            }
        }
        let input = Tensor::from_array(([1usize, 3, rh as usize, rw as usize], data))
            .map_err(|e| e.to_string())?;
        let outputs = self
            .det
            .run(ort::inputs![self.det_in.as_str() => input].map_err(|e| e.to_string())?)
            .map_err(|e| format!("run det: {e}"))?;
        let view = outputs
            .get(self.det_out.as_str())
            .ok_or("sortie det absente")?
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("extraction det: {e}"))?;
        let shape = view.shape().to_vec();
        // [1,1,H,W] — lire H,W du modèle (peut différer de l'entrée).
        let (oh, ow) = (shape[shape.len() - 2], shape[shape.len() - 1]);
        let prob: Vec<f32> = view.iter().copied().collect();
        // Masque binaire → composantes connexes (8-voisins) → boîtes.
        let mask: Vec<bool> = prob.iter().map(|&v| v > DET_THRESHOLD).collect();
        let boxes = connected_boxes(&mask, ow, oh, MIN_BLOB);
        // Remise à l'échelle source + léger unclip (DB prédit un masque rétréci).
        let sx = shot.width as f32 / ow as f32;
        let sy = shot.height as f32 / oh as f32;
        let mut out = Vec::with_capacity(boxes.len());
        for (x0, y0, x1, y1) in boxes {
            let bh = (y1 - y0 + 1) as f32;
            let pad = (bh * 0.15).round();
            let mx0 = ((x0 as f32 - pad) * sx).floor().max(0.0) as i32;
            let my0 = ((y0 as f32 - pad) * sy).floor().max(0.0) as i32;
            let mx1 = (((x1 as f32 + pad) * sx).ceil() as i32).min(shot.width as i32 - 1);
            let my1 = (((y1 as f32 + pad) * sy).ceil() as i32).min(shot.height as i32 - 1);
            if mx1 > mx0 && my1 > my0 {
                out.push([mx0, my0, mx1 - mx0 + 1, my1 - my0 + 1]);
            }
        }
        Ok(out)
    }

    /// RECONNAISSANCE d'une boîte → (texte, confiance).
    fn recognize(&mut self, shot: &Shot, b: [i32; 4]) -> Result<(String, f32), String> {
        let (bx, by, bw, bh) = (b[0] as u32, b[1] as u32, b[2] as u32, b[3] as u32);
        let src = image::RgbImage::from_raw(shot.width, shot.height, shot.rgb.clone())
            .ok_or("cliché RGB invalide")?;
        let crop = image::imageops::crop_imm(&src, bx, by, bw, bh).to_image();
        // h=48, largeur proportionnelle bornée.
        let rw = (((REC_H as f32) * bw as f32 / bh.max(1) as f32).round() as u32)
            .clamp(8, REC_W_MAX);
        let r = image::imageops::resize(&crop, rw, REC_H, image::imageops::FilterType::Triangle);
        // Normalisation rec PaddleOCR : (v/255 - 0.5)/0.5, RGB, NCHW.
        let hw = (rw * REC_H) as usize;
        let mut data = vec![0f32; 3 * hw];
        for y in 0..REC_H {
            for x in 0..rw {
                let p = r.get_pixel(x, y);
                let d = (y * rw + x) as usize;
                for c in 0..3 {
                    data[c * hw + d] = p[c] as f32 / 127.5 - 1.0;
                }
            }
        }
        let input =
            Tensor::from_array(([1usize, 3, REC_H as usize, rw as usize], data))
                .map_err(|e| e.to_string())?;
        let outputs = self
            .rec
            .run(ort::inputs![self.rec_in.as_str() => input].map_err(|e| e.to_string())?)
            .map_err(|e| format!("run rec: {e}"))?;
        let view = outputs
            .get(self.rec_out.as_str())
            .ok_or("sortie rec absente")?
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("extraction rec: {e}"))?;
        let shape = view.shape().to_vec();
        // [1, T, C] — softmax déjà appliqué (sortie softmax_2.tmp_0).
        let (t_len, classes) = (shape[shape.len() - 2], shape[shape.len() - 1]);
        let flat: Vec<f32> = view.iter().copied().collect();
        // CTC glouton : argmax/timestep, effacer répétitions et blanc.
        let mut text = String::new();
        let mut prev = usize::MAX;
        let mut conf_sum = 0.0f32;
        let mut conf_n = 0u32;
        for t in 0..t_len {
            let row = &flat[t * classes..(t + 1) * classes];
            let (mut arg, mut best) = (0usize, row[0]);
            for (i, &v) in row.iter().enumerate() {
                if v > best {
                    best = v;
                    arg = i;
                }
            }
            if arg != 0 && arg != prev {
                if let Some(ch) = self.labels.get(arg) {
                    text.push_str(ch);
                    conf_sum += best;
                    conf_n += 1;
                }
            }
            prev = arg;
        }
        let conf = if conf_n > 0 { conf_sum / conf_n as f32 } else { 0.0 };
        Ok((text, conf))
    }
}

fn median_height(words: &[Word]) -> i32 {
    if words.is_empty() {
        return 8;
    }
    let mut hs: Vec<i32> = words.iter().map(|w| w.bbox[3]).collect();
    hs.sort_unstable();
    (hs[hs.len() / 2] / 2).max(4)
}

/// Composantes connexes 8-voisins d'un masque booléen → boîtes (x0,y0,x1,y1).
fn connected_boxes(mask: &[bool], w: usize, h: usize, min_blob: usize) -> Vec<(i32, i32, i32, i32)> {
    let mut seen = vec![false; w * h];
    let mut boxes = Vec::new();
    let mut stack: Vec<(i32, i32)> = Vec::new();
    for sy in 0..h {
        for sx in 0..w {
            let idx = sy * w + sx;
            if !mask[idx] || seen[idx] {
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1) = (sx as i32, sy as i32, sx as i32, sy as i32);
            let mut count = 0usize;
            seen[idx] = true;
            stack.push((sx as i32, sy as i32));
            while let Some((cx, cy)) = stack.pop() {
                count += 1;
                x0 = x0.min(cx);
                y0 = y0.min(cy);
                x1 = x1.max(cx);
                y1 = y1.max(cy);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let nx = cx + dx;
                        let ny = cy + dy;
                        if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                            continue;
                        }
                        let ni = ny as usize * w + nx as usize;
                        if mask[ni] && !seen[ni] {
                            seen[ni] = true;
                            stack.push((nx, ny));
                        }
                    }
                }
            }
            if count >= min_blob {
                boxes.push((x0, y0, x1, y1));
            }
        }
    }
    boxes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deux_blobs_disjoints() {
        // Grille 10x3 : un blob à gauche (x0..1), un à droite (x8..9).
        let (w, h) = (10usize, 3usize);
        let mut m = vec![false; w * h];
        for y in 0..h {
            for x in [0usize, 1, 8, 9] {
                m[y * w + x] = true;
            }
        }
        let mut b = connected_boxes(&m, w, h, 3);
        b.sort_by_key(|k| k.0);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0], (0, 0, 1, 2));
        assert_eq!(b[1], (8, 0, 9, 2));
    }

    #[test]
    fn bruit_sous_seuil_ignore() {
        let (w, h) = (5usize, 5usize);
        let mut m = vec![false; w * h];
        m[0] = true; // blob de 1 pixel
        assert!(connected_boxes(&m, w, h, 8).is_empty());
    }
}
