//! waly-sight — banc R4 (GATES C+D du plan).
//!
//! Sous-commandes :
//!   cams                          — énumère les caméras Media Foundation
//!   snap [N] [sortie.png]         — capture une frame de la caméra N (défaut 0)
//!   bench-face <image.png> [n]    — n passes YuNet sur une image (défaut 50)
//!   bench-live [N] [secondes]     — boucle capture+détection en direct
//!   watch [N] [secondes]          — service de perception : ÉVÉNEMENTS live
//!
//! Modèle : C:\waly\engines\models\yunet\face_detection_yunet_2023mar.onnx
//! (surcharge : WALY_YUNET).

use std::time::Instant;

use waly_sight::face::FaceDetector;

/// Sel de reroll SAC (piège 3) : un `touch` ne change pas le hash d'un
/// build reproductible → verdict identique. Incrémenter cette constante
/// suffit à produire un binaire neuf quand SAC bloque.
const SAC_REROLL: u32 = 1;

fn model_path() -> String {
    std::env::var("WALY_YUNET").unwrap_or_else(|_| {
        r"C:\waly\engines\models\yunet\face_detection_yunet_2023mar.onnx".into()
    })
}

fn main() {
    std::hint::black_box(SAC_REROLL);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let result = match cmd {
        "cams" => cams(),
        "snap" => snap(&args[1..]),
        "snap-screen" => snap_screen(&args[1..]),
        "ocr" => ocr_cmd(&args[1..]),
        "bench-face" => bench_face(&args[1..]),
        "bench-live" => bench_live(&args[1..]),
        "watch" => watch(&args[1..]),
        _ => {
            eprintln!("usage: waly-sight cams | snap [N] [out.png] | snap-screen [window|full] [out.png] | bench-face <img> [n] | bench-live [N] [secs] | watch [N] [secs]");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("erreur: {e}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn cams() -> Result<(), String> {
    for line in waly_sight::camera::list()? {
        println!("{line}");
    }
    Ok(())
}

#[cfg(windows)]
fn snap(args: &[String]) -> Result<(), String> {
    let index: u32 = args.first().and_then(|a| a.parse().ok()).unwrap_or(0);
    let out = args.get(1).cloned().unwrap_or_else(|| "snap.png".into());
    let t0 = Instant::now();
    let mut cam = waly_sight::camera::Cam::open(index)?;
    let t_open = t0.elapsed();
    println!("format négocié: {}", cam.format());
    // Laisser l'auto-exposition se poser avant la frame utile.
    for _ in 0..10 {
        cam.grab()?;
    }
    let t1 = Instant::now();
    let frame = cam.grab()?;
    let t_grab = t1.elapsed();
    image::save_buffer(
        &out,
        &frame.rgb,
        frame.width,
        frame.height,
        image::ColorType::Rgb8,
    )
    .map_err(|e| format!("écriture {out}: {e}"))?;
    println!(
        "{}x{} → {out} | ouverture {:.0} ms, frame {:.1} ms",
        frame.width,
        frame.height,
        t_open.as_secs_f64() * 1000.0,
        t_grab.as_secs_f64() * 1000.0
    );
    Ok(())
}

/// R5 GATE 1 : capture d'écran sous SAC + latence + fenêtre vs écran entier.
/// `snap-screen [window|full] [out.png]` — défaut `full`.
#[cfg(windows)]
fn snap_screen(args: &[String]) -> Result<(), String> {
    waly_sight::screen::rendre_conscient_dpi();
    let cible = args.first().map(String::as_str).unwrap_or("full");
    let (label, out_defaut) = match cible {
        "window" | "win" | "fenetre" => ("fenêtre active", "screen-window.png"),
        _ => ("écran entier", "screen-full.png"),
    };
    let out = args.get(1).cloned().unwrap_or_else(|| out_defaut.into());
    // Une première capture chauffe le chemin GDI (allocations, DIB).
    let _ = capture(cible);
    let t = Instant::now();
    let shot = capture(cible)?;
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    image::save_buffer(&out, &shot.rgb, shot.width, shot.height, image::ColorType::Rgb8)
        .map_err(|e| format!("écriture {out}: {e}"))?;
    println!(
        "{label}: {}x{} → {out} | capture {:.1} ms | {:.1} Mpx",
        shot.width,
        shot.height,
        ms,
        (shot.width as f64 * shot.height as f64) / 1e6
    );
    Ok(())
}

#[cfg(windows)]
fn capture(cible: &str) -> Result<waly_sight::screen::Shot, String> {
    match cible {
        "window" | "win" | "fenetre" => waly_sight::screen::capture_active_window(),
        _ => waly_sight::screen::capture_screen(),
    }
}

#[cfg(not(windows))]
fn snap_screen(_: &[String]) -> Result<(), String> {
    Err("capture d'écran : Windows seulement".into())
}

/// R5 chantier 1 : OCR-first. `ocr [image.png | full | window]` — défaut :
/// capture l'écran entier. Imprime le texte lu, la latence et la confiance.
fn ocr_cmd(args: &[String]) -> Result<(), String> {
    use waly_sight::ocr::Ocr;
    use waly_sight::screen::Shot;
    let cible = args.first().map(String::as_str).unwrap_or("full");
    // Source : un PNG fourni, sinon une capture d'écran live.
    let shot: Shot = if cible.ends_with(".png") {
        let img = image::open(cible).map_err(|e| format!("lecture {cible}: {e}"))?.to_rgb8();
        Shot { width: img.width(), height: img.height(), rgb: img.into_raw() }
    } else {
        #[cfg(windows)]
        {
            waly_sight::screen::rendre_conscient_dpi();
            capture(cible)?
        }
        #[cfg(not(windows))]
        {
            return Err("capture d'écran : Windows seulement (fournir un .png)".into());
        }
    };
    let t0 = Instant::now();
    let mut ocr = Ocr::load_default()?;
    let t_load = t0.elapsed();
    let t1 = Instant::now();
    let res = ocr.read(&shot)?;
    let t_read = t1.elapsed();
    println!(
        "cliché {}x{} | chargement {:.0} ms | lecture {:.0} ms | {} zones | conf moy {:.2}",
        shot.width,
        shot.height,
        t_load.as_secs_f64() * 1000.0,
        t_read.as_secs_f64() * 1000.0,
        res.words.len(),
        res.mean_conf
    );
    println!("--- texte lu ---\n{}", res.text);
    Ok(())
}

fn bench_face(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("bench-face <image.png> [n]")?;
    let n: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(50);
    let img = image::open(path).map_err(|e| format!("lecture {path}: {e}"))?.to_rgb8();
    let (w, h) = (img.width(), img.height());
    let rgb = img.into_raw();

    let t0 = Instant::now();
    let mut det = FaceDetector::load(&model_path())?;
    println!("chargement YuNet: {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0);

    let mut times = Vec::with_capacity(n);
    let mut faces = Vec::new();
    for _ in 0..n {
        let t = Instant::now();
        faces = det.detect(&rgb, w, h)?;
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    println!(
        "{n} passes sur {w}x{h} : méd {:.1} ms | p90 {:.1} ms | min {:.1} ms",
        times[n / 2],
        times[n * 9 / 10],
        times[0]
    );
    println!("{} visage(s) :", faces.len());
    for f in &faces {
        println!(
            "  score {:.2} bbox [{:.0},{:.0} {:.0}x{:.0}] yeux ({:.0},{:.0})/({:.0},{:.0})",
            f.score,
            f.bbox[0],
            f.bbox[1],
            f.bbox[2],
            f.bbox[3],
            f.kps[0][0],
            f.kps[0][1],
            f.kps[1][0],
            f.kps[1][1]
        );
    }
    Ok(())
}

#[cfg(windows)]
fn bench_live(args: &[String]) -> Result<(), String> {
    let index: u32 = args.first().and_then(|a| a.parse().ok()).unwrap_or(0);
    let secs: u64 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(10);
    let mut cam = waly_sight::camera::Cam::open(index)?;
    println!("format négocié: {}", cam.format());
    let mut det = FaceDetector::load(&model_path())?;
    // Chauffe : auto-exposition + première inférence (allocations ort).
    for _ in 0..5 {
        let f = cam.grab()?;
        det.detect(&f.rgb, f.width, f.height)?;
    }
    let mut grab_ms = Vec::new();
    let mut det_ms = Vec::new();
    let mut with_face = 0usize;
    let start = Instant::now();
    while start.elapsed().as_secs() < secs {
        let t = Instant::now();
        let f = cam.grab()?;
        grab_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        let faces = det.detect(&f.rgb, f.width, f.height)?;
        det_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        if !faces.is_empty() {
            with_face += 1;
        }
    }
    let n = det_ms.len();
    grab_ms.sort_by(f64::total_cmp);
    det_ms.sort_by(f64::total_cmp);
    println!(
        "{n} frames en {secs} s ({:.1} fps) | capture méd {:.1} ms | détection méd {:.1} ms (p90 {:.1}) | visage sur {}/{n}",
        n as f64 / secs as f64,
        grab_ms[n / 2],
        det_ms[n / 2],
        det_ms[n * 9 / 10],
        with_face
    );
    Ok(())
}

#[cfg(windows)]
fn watch(args: &[String]) -> Result<(), String> {
    use waly_sight::perception::{demarrer, Config};
    let index: u32 = args.first().and_then(|a| a.parse().ok()).unwrap_or(0);
    let secs: u64 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(20);
    let config = Config::default();
    println!(
        "perception {} Hz pendant {secs} s (caméra {index}) — bouge, regarde ailleurs…",
        config.cadence_hz
    );
    let t0 = Instant::now();
    let (rx, percepteur) = demarrer(index, config, &model_path())?;
    println!("démarrée en {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0);
    let debut = Instant::now();
    while debut.elapsed().as_secs() < secs {
        match rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(ev) => println!(
                "[{:6.2} s] {}",
                debut.elapsed().as_secs_f64(),
                serde_json::to_string(&ev).unwrap_or_else(|e| e.to_string())
            ),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let stats = percepteur.arreter();
    println!(
        "stats: {} cycles à {:.1} Hz effectifs | capture méd {:.1} ms | visage {:.1} ms",
        stats.cycles,
        stats.cadence_effective_hz,
        stats.grab_ms_med,
        stats.detect_ms_med
    );
    Ok(())
}

#[cfg(not(windows))]
fn watch(_: &[String]) -> Result<(), String> {
    Err("caméra : Windows seulement".into())
}

#[cfg(not(windows))]
fn cams() -> Result<(), String> {
    Err("caméra : Windows seulement".into())
}
#[cfg(not(windows))]
fn snap(_: &[String]) -> Result<(), String> {
    Err("caméra : Windows seulement".into())
}
#[cfg(not(windows))]
fn bench_live(_: &[String]) -> Result<(), String> {
    Err("caméra : Windows seulement".into())
}
