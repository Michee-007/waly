//! Banc R6b « L'Éveil » — wake word « Waly » (openWakeWord ONNX).
//!
//! Sous-commandes :
//!   gen <out_dir> [prises_pos=30] [prises_neg=3]   corpus TTS (Pocket+Piper)
//!   features <in_dir> <out.f32> word|slide [K] [seed]   features 16×96
//!   bruit <out.f32> <count> [seed]                  features bruit/silence
//!   pipeline <wav> <clf.onnx>                       scores fenêtre par fenêtre
//!   latence                                          coût par pas de 80 ms
//!   evalue <pos.f32> <neg.f32> <clf.onnx>            FRR/FAR par seuil
//!   detecte <word.wav> <clf.onnx> <seuil>            latence de détection
//!   repos <secondes> [--sans-vad]                    CPU résident simulé

mod augment;
mod gen;
mod oww;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{anyhow, bail, Context, Result};
use augment::XorShift;
use oww::{Classifier, Oww, EMB_DIM, FEAT_LEN, SR};

fn modeles() -> PathBuf {
    PathBuf::from(
        std::env::var("WALY_OWW_DIR")
            .unwrap_or_else(|_| r"C:\waly\engines\models\openwakeword".into()),
    )
}

fn init_ort() -> Result<()> {
    waly_voice::vad::init_onnxruntime().map_err(|e| anyhow!("{e}"))
}

fn wavs_de(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).with_context(|| dir.display().to_string())? {
        let p = e?.path();
        if p.extension().map(|x| x == "wav").unwrap_or(false) {
            out.push(p);
        }
    }
    out.sort();
    if out.is_empty() {
        bail!("aucun wav dans {}", dir.display());
    }
    Ok(out)
}

fn ecrit_f32(path: &Path, rows: &[Vec<f32>]) -> Result<()> {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    for r in rows {
        debug_assert_eq!(r.len(), FEAT_LEN);
        for v in r {
            f.write_all(&v.to_le_bytes())?;
        }
    }
    f.flush()?;
    println!("{} : {} lignes de {}", path.display(), rows.len(), FEAT_LEN);
    Ok(())
}

fn lit_f32(path: &Path) -> Result<Vec<Vec<f32>>> {
    let raw = std::fs::read(path).with_context(|| path.display().to_string())?;
    if raw.len() % (FEAT_LEN * 4) != 0 {
        bail!("{} : taille non multiple de {}", path.display(), FEAT_LEN * 4);
    }
    Ok(raw
        .chunks_exact(FEAT_LEN * 4)
        .map(|row| {
            row.chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect()
        })
        .collect())
}

fn cmd_features(in_dir: &Path, out: &Path, mode: &str, k: usize, seed: u64) -> Result<()> {
    init_ort()?;
    let oww = Oww::new(&modeles())?;
    let mut rng = XorShift::new(seed);
    let mut rows = Vec::new();
    for wav in wavs_de(in_dir)? {
        let (audio, sr) = gen::read_wav_mono(&wav)?;
        if sr != SR as u32 {
            bail!("{} : {sr} Hz (attendu 16 k)", wav.display());
        }
        match mode {
            "word" => {
                for _ in 0..k {
                    let clip = augment::assemble_word(&audio, &mut rng);
                    rows.push(oww.last_features(&clip)?);
                }
            }
            "slide" => {
                // Phrase courte : silence devant pour atteindre les
                // 16 embeddings (1,96 s de mel) — sans avorter le dossier.
                // K passes : la 1re propre, les suivantes gain+bruit (les
                // fenêtres négatives doivent AUSSI exister en version
                // bruitée, sinon « bruité » devient un indice de positif).
                for pass in 0..k.max(1) {
                    let mut padded = vec![0.0f32; oww::CLIP_SAMPLES];
                    if pass == 0 {
                        padded.extend_from_slice(&audio);
                    } else {
                        let gain = rng.range(0.4, 1.3);
                        let rms = (audio.iter().map(|v| v * v).sum::<f32>()
                            / audio.len() as f32)
                            .sqrt();
                        let snr_db = rng.range(10.0, 30.0);
                        let n_rms = rms * gain / 10f32.powf(snr_db / 20.0);
                        let n = augment::noise(&mut rng, audio.len(), n_rms);
                        padded.extend(
                            audio
                                .iter()
                                .zip(n)
                                .map(|(a, b)| (a * gain + b).clamp(-1.0, 1.0)),
                        );
                    }
                    rows.extend(oww.all_features(&padded)?);
                }
            }
            m => bail!("mode inconnu {m}"),
        }
    }
    ecrit_f32(out, &rows)
}

fn cmd_bruit(out: &Path, count: usize, seed: u64) -> Result<()> {
    init_ort()?;
    let oww = Oww::new(&modeles())?;
    let mut rng = XorShift::new(seed);
    let mut rows = Vec::new();
    for i in 0..count {
        let clip = if i % 4 == 0 {
            vec![0.0f32; oww::CLIP_SAMPLES] // silence pur
        } else {
            let rms = rng.range(0.001, 0.15);
            augment::noise(&mut rng, oww::CLIP_SAMPLES, rms)
        };
        rows.push(oww.last_features(&clip)?);
    }
    ecrit_f32(out, &rows)
}

fn cmd_pipeline(wav: &Path, clf: &Path) -> Result<()> {
    init_ort()?;
    let oww = Oww::new(&modeles())?;
    let clf = Classifier::new(clf)?;
    let (audio, sr) = gen::read_wav_mono(wav)?;
    let audio = if sr != SR as u32 {
        waly_voice::pocket::resample_sinc(&audio, sr, SR as u32)
    } else {
        audio
    };
    // Clip court : silence devant/derrière pour que le mot tienne dans une
    // fenêtre classifieur complète (16 embeddings = 1,96 s de mel).
    let mut padded = vec![0.0f32; 2 * SR];
    padded.extend_from_slice(&audio);
    padded.extend(std::iter::repeat(0.0).take(SR / 2));
    let audio = padded;
    let feats = oww.all_features(&audio)?;
    let mut max = (0usize, f32::MIN);
    for (i, f) in feats.iter().enumerate() {
        let s = clf.score(f)?;
        if s > max.1 {
            max = (i, s);
        }
        if s > 0.3 {
            println!("  fenêtre {i} (t≈{:.2} s) : {s:.3}", (i * 8 + 76) as f32 / 100.0);
        }
    }
    println!(
        "{} : {} fenêtres, max {:.3} à t≈{:.2} s",
        wav.display(),
        feats.len(),
        max.1,
        (max.0 * 8 + 76) as f32 / 100.0
    );
    Ok(())
}

fn cmd_latence() -> Result<()> {
    init_ort()?;
    let oww = Oww::new(&modeles())?;
    let mut rng = XorShift::new(7);
    // Un pas de 80 ms au fil de l'eau : mel du chunk (1280 + 480 de
    // contexte), UN embedding, UN score.
    let clf_path = modeles().join("hey_jarvis_v0.1.onnx");
    let clf = Classifier::new(&clf_path)?;
    let chunk = augment::noise(&mut rng, 1760, 0.05);
    // frames = N/160 − 3 (mesuré) → 76 trames = (76+3)×160 = 12640 éch.
    let win = augment::noise(&mut rng, 12640, 0.05);
    let feat = vec![0.1f32; FEAT_LEN];

    let n = 500;
    let t0 = Instant::now();
    for _ in 0..n {
        let _ = oww.melspec(&chunk)?;
    }
    let mel_us = t0.elapsed().as_micros() as f64 / n as f64;
    let t0 = Instant::now();
    for _ in 0..n {
        let m = oww.melspec(&win)?;
        let _ = oww.embeddings(&m[..76 * 32])?;
    }
    let emb_us = t0.elapsed().as_micros() as f64 / n as f64;
    let t0 = Instant::now();
    for _ in 0..n {
        let _ = clf.score(&feat)?;
    }
    let clf_us = t0.elapsed().as_micros() as f64 / n as f64;
    let step = mel_us + emb_us + clf_us;
    println!("par pas de 80 ms : mel {mel_us:.0} µs + emb(+mel 760 ms) {emb_us:.0} µs + clf {clf_us:.0} µs = {step:.0} µs");
    println!("→ occupation d'un cœur : {:.2} %", step / 80_000.0 * 100.0);
    Ok(())
}

fn cmd_evalue(pos: &Path, neg: &Path, clf: &Path) -> Result<()> {
    init_ort()?;
    let clf = Classifier::new(clf)?;
    let score_all = |rows: &[Vec<f32>]| -> Result<Vec<f32>> {
        rows.iter().map(|r| clf.score(r)).collect()
    };
    let sp = score_all(&lit_f32(pos)?)?;
    let sn = score_all(&lit_f32(neg)?)?;
    println!("positifs {} | négatifs {}", sp.len(), sn.len());
    println!("seuil   FRR (rate le mot)   FAR (fausse alerte)");
    for th in [0.1f32, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9] {
        let frr = sp.iter().filter(|s| **s < th).count() as f64 / sp.len() as f64;
        let far = sn.iter().filter(|s| **s >= th).count() as f64 / sn.len().max(1) as f64;
        println!("{th:.2}     {:6.2} %             {:6.3} %", frr * 100.0, far * 100.0);
    }
    Ok(())
}

fn cmd_detecte(word: &Path, clf: &Path, seuil: f32) -> Result<()> {
    init_ort()?;
    let oww = Oww::new(&modeles())?;
    let clf = Classifier::new(clf)?;
    let (w, sr) = gen::read_wav_mono(word)?;
    if sr != SR as u32 {
        bail!("wav 16 kHz requis");
    }
    // 3 s de contexte calme + le mot + 2 s de queue : à quel pas le score
    // franchit-il le seuil (patience 2 pas) après la FIN du mot ?
    let mut rng = XorShift::new(3);
    let mut clip = augment::noise(&mut rng, 3 * SR, 0.01);
    let word_end = clip.len() + w.len();
    clip.extend_from_slice(&w);
    clip.extend(augment::noise(&mut rng, 2 * SR, 0.01));
    let feats = oww.all_features(&clip)?;
    let mut consecutifs = 0;
    for (i, f) in feats.iter().enumerate() {
        let s = clf.score(f)?;
        consecutifs = if s >= seuil { consecutifs + 1 } else { 0 };
        if consecutifs >= 2 {
            // La fenêtre i couvre les trames mel [i*8, i*8+196) → fin à
            // (i*8+196)*10 ms d'audio.
            let t_detect = ((i * 8 + 196) * SR / 100) as isize;
            let delta_ms = (t_detect - word_end as isize) as f64 / SR as f64 * 1000.0;
            println!(
                "détection au pas {i} : {delta_ms:+.0} ms après la fin du mot (seuil {seuil}, patience 2)"
            );
            return Ok(());
        }
    }
    println!("PAS de détection (seuil {seuil})");
    Ok(())
}

/// FAR séquentiel honnête : tous les wav d'un dossier mis bout à bout
/// (comme un flux de parole continu), déclencheur à seuil + patience 2 pas
/// + refractaire 2 s → alertes par HEURE de parole.
fn cmd_far(dir: &Path, clf_path: &Path, seuil: f32) -> Result<()> {
    init_ort()?;
    let oww = Oww::new(&modeles())?;
    let clf = Classifier::new(clf_path)?;
    let mut flux: Vec<f32> = Vec::new();
    for wav in wavs_de(dir)? {
        let (a, sr) = gen::read_wav_mono(&wav)?;
        if sr != SR as u32 {
            bail!("{} : {sr} Hz", wav.display());
        }
        flux.extend_from_slice(&a);
        flux.extend(std::iter::repeat(0.0).take(SR / 4));
    }
    let heures = flux.len() as f64 / SR as f64 / 3600.0;
    let feats = oww.all_features(&flux)?;
    let mut alertes = 0usize;
    let mut consecutifs = 0usize;
    let mut refractaire_jusqua = 0usize;
    for (i, f) in feats.iter().enumerate() {
        if i < refractaire_jusqua {
            continue;
        }
        let s = clf.score(f)?;
        consecutifs = if s >= seuil { consecutifs + 1 } else { 0 };
        if consecutifs >= 2 {
            alertes += 1;
            consecutifs = 0;
            refractaire_jusqua = i + 25; // 2 s
        }
    }
    println!(
        "{} : {:.2} min de parole, {alertes} alertes (seuil {seuil}, patience 2) → {:.1} alertes/h de parole continue",
        dir.display(),
        heures * 60.0,
        alertes as f64 / heures.max(1e-9)
    );
    Ok(())
}

/// FRR séquentiel (miroir du runtime) : chaque wav = UN énoncé « Waly » ;
/// détecté si le score franchit le seuil sur 2 pas consécutifs quelque part.
fn cmd_frr(dir: &Path, clf_path: &Path, seuil: f32) -> Result<()> {
    init_ort()?;
    let oww = Oww::new(&modeles())?;
    let clf = Classifier::new(clf_path)?;
    let mut total = 0usize;
    let mut detectes = 0usize;
    for wav in wavs_de(dir)? {
        let (w, sr) = gen::read_wav_mono(&wav)?;
        if sr != SR as u32 {
            bail!("{} : {sr} Hz", wav.display());
        }
        let mut clip = vec![0.0f32; 2 * SR];
        clip.extend_from_slice(&w);
        clip.extend(std::iter::repeat(0.0).take(SR / 2));
        let feats = oww.all_features(&clip)?;
        let mut consecutifs = 0;
        let mut vu = false;
        for f in &feats {
            consecutifs = if clf.score(f)? >= seuil { consecutifs + 1 } else { 0 };
            if consecutifs >= 2 {
                vu = true;
                break;
            }
        }
        total += 1;
        if vu {
            detectes += 1;
        }
    }
    println!(
        "{} : {detectes}/{total} énoncés détectés (seuil {seuil}, patience 2) → FRR séquentiel {:.1} %",
        dir.display(),
        (1.0 - detectes as f64 / total.max(1) as f64) * 100.0
    );
    Ok(())
}

fn cmd_repos(secondes: usize, avec_vad: bool) -> Result<()> {
    init_ort()?;
    let oww = Oww::new(&modeles())?;
    let clf = Classifier::new(&modeles().join("hey_jarvis_v0.1.onnx"))?;
    let mut vad = if avec_vad {
        Some(
            waly_voice::vad::SileroVad::new(r"C:\waly\engines\models\silero_vad.onnx")
                .map_err(|e| anyhow!("{e}"))?,
        )
    } else {
        None
    };
    let mut rng = XorShift::new(11);

    // Deux bandes de `secondes` : silence (bruit de fond -50 dB) puis
    // parole continue (bruit fort — pire cas : VAD toujours ouvert).
    for (nom, rms) in [("silence", 0.002f32), ("parole continue", 0.08)] {
        let mut mel_tail: Vec<f32> = Vec::new(); // 76 trames glissantes
        let mut emb_hist: Vec<f32> = vec![0.0; FEAT_LEN];
        let mut compute = std::time::Duration::ZERO;
        let chunks = secondes * SR / 1280;
        let mut declenches = 0usize;
        for _ in 0..chunks {
            let chunk = augment::noise(&mut rng, 1280, rms);
            let t0 = Instant::now();
            // VAD d'abord (512 éch. × 2 : on échantillonne 2 trames par chunk)
            let parle = match vad.as_mut() {
                Some(v) => {
                    let a = v.process(&chunk[..512]).map_err(|e| anyhow!("{e}"))? > 0.3;
                    let b = v.process(&chunk[512..1024]).map_err(|e| anyhow!("{e}"))? > 0.3;
                    a || b
                }
                None => true,
            };
            if parle {
                // mel du chunk avec 480 éch. de contexte simulé
                let mut with_ctx = vec![0.0f32; 480];
                with_ctx.extend_from_slice(&chunk);
                let m = oww.melspec(&with_ctx)?;
                let frames = m.len() / 32;
                let take = frames.min(8);
                mel_tail.extend_from_slice(&m[(frames - take) * 32..]);
                let len = mel_tail.len();
                if len > 76 * 32 {
                    mel_tail.drain(..len - 76 * 32);
                }
                if mel_tail.len() == 76 * 32 {
                    let e = oww.embeddings(&mel_tail)?;
                    emb_hist.drain(..EMB_DIM);
                    emb_hist.extend_from_slice(&e[..EMB_DIM]);
                    if clf.score(&emb_hist)? > 0.5 {
                        declenches += 1;
                    }
                }
            }
            compute += t0.elapsed();
        }
        let cpu_pct = compute.as_secs_f64() / secondes as f64 * 100.0;
        println!(
            "{nom} ({secondes} s, vad={}) : calcul {:.2} s → {:.2} % d'un cœur ({declenches} scores > 0,5)",
            avec_vad,
            compute.as_secs_f64(),
            cpu_pct
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| -> Result<&str> {
        args.get(i).map(|s| s.as_str()).context("argument manquant")
    };
    match arg(0)? {
        "gen" => {
            let pos = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
            let neg = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(3);
            gen::gen(Path::new(arg(1)?), pos, neg)
        }
        "features" => {
            let k = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(8);
            let seed = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(1);
            cmd_features(Path::new(arg(1)?), Path::new(arg(2)?), arg(3)?, k, seed)
        }
        "bruit" => {
            let n = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(500);
            let seed = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(9);
            cmd_bruit(Path::new(arg(1)?), n, seed)
        }
        "gen-pitch" => {
            let prises = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(15);
            gen::gen_pitch(Path::new(arg(1)?), prises)
        }
        "gen-extra" => {
            let prises = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(15);
            gen::gen_extra(Path::new(arg(1)?), prises)
        }
        "frr" => {
            let seuil = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.85);
            cmd_frr(Path::new(arg(1)?), Path::new(arg(2)?), seuil)
        }
        "far" => {
            let seuil = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.5);
            cmd_far(Path::new(arg(1)?), Path::new(arg(2)?), seuil)
        }
        "pipeline" => cmd_pipeline(Path::new(arg(1)?), Path::new(arg(2)?)),
        "latence" => cmd_latence(),
        "evalue" => cmd_evalue(Path::new(arg(1)?), Path::new(arg(2)?), Path::new(arg(3)?)),
        "detecte" => {
            let seuil = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.5);
            cmd_detecte(Path::new(arg(1)?), Path::new(arg(2)?), seuil)
        }
        "repos" => {
            let s = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);
            let sans_vad = args.iter().any(|a| a == "--sans-vad");
            cmd_repos(s, !sans_vad)
        }
        c => bail!("commande inconnue {c}"),
    }
}
