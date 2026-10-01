//! Service voix Waly — outils de diagnostic audio (v0 de R1).
//!
//! Sous-commandes :
//!   devices             — scanner les périphériques d'entrée (JAMAIS d'index en dur)
//!   vu [secs]           — VU-mètre de diagnostic micro (hérité de l'ancien monde)
//!   record <out.wav> [secs] [nom-de-device]
//!                       — enregistre en mono 16 kHz s16 (format Whisper/Silero)
//!
//! Messages console volontairement en ASCII : la console Windows (CP850/1252)
//! mange les accents des sorties UTF-8.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const TARGET_RATE: u32 = 16_000;

/// Verrou insensible à l'empoisonnement. Les callbacks cpal tournent sur le
/// thread audio temps réel : si un détenteur du mutex panique, un
/// `lock().unwrap()` propagerait la panique DANS le callback audio. Les
/// données protégées (échantillons) restent utilisables telles quelles.
fn lock_audio<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sel de build : SAC (Smart App Control) juge chaque binaire par son hash,
/// verdicts imprevisibles (piege 3 de CLAUDE.md). Si un build se fait bloquer
/// (erreur 4551), incrementer ce sel et rebuilder — nouveau hash, nouveau
/// verdict. Retire du binaire signe final.
#[used]
static SAC_BUILD_SALT: u32 = 3;

/// Piège 3 : Smart App Control rend son verdict PAR binaire — incrémenter
/// pour changer le hash quand un build est bloqué (CodeIntegrity 3033/3077).
const SAC_REROLL: u32 = 5;

fn main() {
    std::hint::black_box(SAC_REROLL);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("devices") => cmd_devices(),
        Some("vu") => cmd_vu(parse_secs(args.get(1), 8)),
        Some("vad") => cmd_vad(parse_secs(args.get(1), 15)),
        Some("vadfile") => match args.get(1) {
            Some(wav) => cmd_vadfile(wav),
            None => {
                eprintln!("usage: waly-voice vadfile <fichier.wav>");
                std::process::exit(2);
            }
        },
        Some("stt") => match args.get(1) {
            Some(wav) => cmd_stt(wav),
            None => {
                eprintln!("usage: waly-voice stt <fichier.wav>");
                std::process::exit(2);
            }
        },
        Some("say") => {
            let text = args[1..].join(" ");
            if text.is_empty() {
                eprintln!("usage: waly-voice say <texte...>");
                std::process::exit(2);
            }
            cmd_say(&text)
        }
        Some("text") => match args.get(1) {
            Some(t) => cmd_text(t),
            None => {
                eprintln!("usage: waly-voice text \"message\"");
                std::process::exit(2);
            }
        },
        Some("turn") => match args.get(1) {
            Some(wav) => cmd_turn(wav),
            None => {
                eprintln!("usage: waly-voice turn <fichier.wav>");
                std::process::exit(2);
            }
        },
        Some("pocket") => {
            if args.len() >= 4 {
                cmd_pocket(&args[1], &args[2], &args[3..].join(" "))
            } else {
                eprintln!("usage: waly-voice pocket <ref.wav> <out.wav> <texte...>");
                std::process::exit(2);
            }
        }
        Some("piper") => {
            if args.len() >= 5 {
                cmd_piper(&args[1], args[2].parse().unwrap_or(0), &args[3], &args[4..].join(" "))
            } else {
                eprintln!("usage: waly-voice piper <voix> <sid> <out.wav> <texte...>");
                std::process::exit(2);
            }
        }
        Some("talk") => cmd_talk(parse_secs(args.get(1), 0)),
        Some("veille") => cmd_veille(),
        Some("dicter") => cmd_dicter(),
        Some("record") => match args.get(1) {
            Some(out) => cmd_record(
                out,
                parse_secs(args.get(2), 6),
                args.get(3).map(String::as_str),
                parse_secs(args.get(4), TARGET_RATE as u64) as u32,
            ),
            None => {
                eprintln!("usage: waly-voice record <out.wav> [secs] [nom-de-device] [rate]");
                std::process::exit(2);
            }
        },
        _ => {
            eprintln!(
                "usage: waly-voice <devices|vu [secs]|vad [secs]|record <out.wav> [secs] [device]\n\
                 \x20                 |stt <wav>|say <texte>|pocket <ref.wav> <out.wav> <texte>\n\
                 \x20                 |turn <wav>|text \"message\"|talk [secs]|veille>"
            );
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("erreur: {e}");
        std::process::exit(1);
    }
}

fn parse_secs(arg: Option<&String>, default: u64) -> u64 {
    arg.and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn cmd_devices() -> Result<(), Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let default_name = host.default_input_device().and_then(|d| d.name().ok());
    println!("Peripheriques d'entree ({:?}):", host.id());
    for device in host.input_devices()? {
        let name = device.name().unwrap_or_else(|_| "<sans nom>".into());
        let star = if Some(&name) == default_name.as_ref() { " *defaut*" } else { "" };
        match device.default_input_config() {
            Ok(cfg) => println!(
                "  [{}] canaux={} sr={} format={:?}{}",
                name,
                cfg.channels(),
                cfg.sample_rate().0,
                cfg.sample_format(),
                star
            ),
            Err(e) => println!("  [{}] (config indisponible: {}){}", name, e, star),
        }
    }
    Ok(())
}

/// Ouvre le device demande (sous-chaine du nom, insensible a la casse) ou le
/// defaut systeme. Retourne (device, config par defaut).
fn open_input(
    wanted: Option<&str>,
) -> Result<(cpal::Device, cpal::SupportedStreamConfig), Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let device = match wanted {
        Some(pat) => {
            let pat = pat.to_lowercase();
            host.input_devices()?
                .find(|d| d.name().map(|n| n.to_lowercase().contains(&pat)).unwrap_or(false))
                .ok_or_else(|| format!("aucun device d'entree ne contient '{pat}'"))?
        }
        None => host
            .default_input_device()
            .ok_or("aucun device d'entree par defaut — lancer 'waly-voice devices'")?,
    };
    let config = device.default_input_config()?;
    println!(
        "Micro: [{}] canaux={} sr={} format={:?}",
        device.name().unwrap_or_default(),
        config.channels(),
        config.sample_rate().0,
        config.sample_format()
    );
    Ok((device, config))
}

/// Lance la capture ; les echantillons (mono f32, cadence native) s'accumulent
/// dans le tampon partage. Le stream vit tant que la valeur retournee vit.
fn start_capture(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    sink: Arc<Mutex<Vec<f32>>>,
    dead: Arc<AtomicBool>,
) -> Result<cpal::Stream, Box<dyn std::error::Error>> {
    let channels = config.channels() as usize;
    // Un stream d'entree qui meurt (micro debranche, device vole par une
    // autre appli) ne produit plus RIEN : sans ce drapeau, la boucle de
    // conversation tournerait indefiniment dans le silence.
    let err_fn = move |e| {
        eprintln!("erreur de flux audio: {e}");
        dead.store(true, Ordering::Relaxed);
    };
    let stream_config: cpal::StreamConfig = config.config();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| push_mono(&sink, data, channels),
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            &stream_config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                let f: Vec<f32> = data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                push_mono(&sink, &f, channels);
            },
            err_fn,
            None,
        )?,
        other => return Err(format!("format d'echantillon non gere: {other:?}").into()),
    };
    stream.play()?;
    Ok(stream)
}

fn push_mono(sink: &Arc<Mutex<Vec<f32>>>, data: &[f32], channels: usize) {
    let mut buf = lock_audio(sink);
    if channels <= 1 {
        buf.extend_from_slice(data);
    } else {
        buf.extend(data.chunks_exact(channels).map(|fr| fr.iter().sum::<f32>() / channels as f32));
    }
}

/// VU-metre : une barre RMS toutes les 100 ms. Diagnostic de l'ancien monde,
/// indispensable ici : le device par defaut peut etre mort (piege connu).
fn cmd_vu(secs: u64) -> Result<(), Box<dyn std::error::Error>> {
    let (device, config) = open_input(None)?;
    let sink = Arc::new(Mutex::new(Vec::<f32>::new()));
    let _stream =
        start_capture(&device, &config, Arc::clone(&sink), Arc::new(AtomicBool::new(false)))?;
    println!("Parle dans le micro ({secs} s)...");
    let start = Instant::now();
    let mut peak_global: f32 = 0.0;
    while start.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_millis(100));
        let chunk: Vec<f32> = std::mem::take(&mut *lock_audio(&sink));
        if chunk.is_empty() {
            println!("  (aucun echantillon recu !)");
            continue;
        }
        let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
        let db = 20.0 * rms.max(1e-6).log10();
        peak_global = peak_global.max(rms);
        let bars = (((db + 60.0) / 60.0).clamp(0.0, 1.0) * 40.0) as usize;
        println!("  [{:<40}] {:>6.1} dB", "#".repeat(bars), db);
    }
    if peak_global < 1e-4 {
        println!("VERDICT: silence total — device mort ou muet, essayer un autre (waly-voice devices)");
    } else {
        println!("VERDICT: micro operationnel (pic RMS {:.4})", peak_global);
    }
    Ok(())
}

/// Enregistre le micro. `rate` : 16 kHz par defaut (VAD/STT) ; passer 24000
/// pour une REFERENCE DE VOIX Pocket (resample sinc, garde les aigus).
fn cmd_record(
    out: &str,
    secs: u64,
    device: Option<&str>,
    rate: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let (device, config) = open_input(device)?;
    let native_rate = config.sample_rate().0;
    let sink = Arc::new(Mutex::new(Vec::<f32>::new()));
    let stream =
        start_capture(&device, &config, Arc::clone(&sink), Arc::new(AtomicBool::new(false)))?;
    println!("Enregistrement {secs} s... parle maintenant !");
    std::thread::sleep(Duration::from_secs(secs));
    drop(stream);
    let samples = std::mem::take(&mut *lock_audio(&sink));
    if samples.is_empty() {
        return Err("aucun echantillon capture".into());
    }
    let resampled = waly_voice::pocket::resample_sinc(&samples, native_rate, rate);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(out, spec)?;
    for s in &resampled {
        writer.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
    }
    writer.finalize()?;
    let peak = resampled.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    println!(
        "Ecrit: {} ({:.1} s a {} Hz mono, pic {:.3}{})",
        out,
        resampled.len() as f32 / rate as f32,
        rate,
        peak,
        if peak < 0.01 { " — ATTENTION: quasi silence" } else { "" }
    );
    Ok(())
}

/// Diagnostic : passe un WAV 16 kHz dans Silero et affiche les probabilites.
fn cmd_vadfile(wav_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    use waly_voice::segment::{SpeechEdge, SpeechSegmenter, FRAME};
    use waly_voice::vad::{self, SileroVad};

    vad::init_onnxruntime()?;
    let model = std::env::var("WALY_SILERO_PATH")
        .unwrap_or_else(|_| waly_core::chemins::modele("silero_vad.onnx"));
    let mut silero = SileroVad::new(&model)?;
    let mut segmenter = SpeechSegmenter::new(800);

    let mut reader = hound::WavReader::open(wav_path)?;
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .map(|&s| s as f32 / i16::MAX as f32)
        .collect();
    let mut max_prob: f32 = 0.0;
    let mut edges = 0;
    for (i, frame) in samples.chunks_exact(FRAME).enumerate() {
        let prob = silero.process(frame)?;
        max_prob = max_prob.max(prob);
        let edge = segmenter.push(prob);
        if edge != SpeechEdge::None {
            edges += 1;
            println!("  trame {i} ({:.2}s): {:?} (p={prob:.2})", i as f32 * 0.032, edge);
        } else if i % 16 == 0 {
            println!("  {:.2}s p={prob:.2}", i as f32 * 0.032);
        }
    }
    println!("prob max = {max_prob:.3} | transitions = {edges}");
    Ok(())
}

/// DICTEE (UI 2026-09-14, comme le micro de la saisie de Claude) : le micro
/// s'allume au lancement et s'eteint des qu'une ligne arrive sur stdin (ou
/// apres 120 s), puis Parakeet transcrit EN LOCAL. Parakeet se charge PENDANT
/// que le micro capte : on peut parler tout de suite. Protocole stdout :
/// `ECOUTE` (le micro capte), puis `TEXTE <json>` ou `ERREUR <json>`.
/// L'audio vit en RAM, n'est jamais ecrit.
fn cmd_dicter() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{BufRead, Write};
    fn dire(ligne: &str) {
        let mut o = std::io::stdout();
        let _ = writeln!(o, "{ligne}");
        let _ = o.flush();
    }
    let res = (|| -> Result<String, Box<dyn std::error::Error>> {
        let (device, config) = open_input(None)?;
        let native_rate = config.sample_rate().0;
        let sink = Arc::new(Mutex::new(Vec::<f32>::new()));
        let dead = Arc::new(AtomicBool::new(false));
        let stream = start_capture(&device, &config, Arc::clone(&sink), Arc::clone(&dead))?;
        dire("ECOUTE");
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            let mut l = String::new();
            let _ = std::io::stdin().lock().read_line(&mut l); // ligne OU fin de flux
            let _ = tx.send(());
        });
        let dll = std::env::var("WALY_SHERPA_DLL")
            .unwrap_or_else(|_| waly_core::chemins::lib_sherpa("sherpa-onnx-c-api"));
        let model_dir = std::env::var("WALY_PARAKEET_DIR")
            .unwrap_or_else(|_| waly_core::chemins::modele("parakeet-tdt-0.6b-v3-int8"));
        let stt = waly_voice::stt::ParakeetStt::new(&dll, &model_dir, 4)?;
        let _ = rx.recv_timeout(Duration::from_secs(120));
        drop(stream); // micro ETEINT avant de transcrire
        let brut = std::mem::take(&mut *lock_audio(&sink));
        if brut.is_empty() {
            return Err(if dead.load(Ordering::Relaxed) {
                "le micro ne répond pas".into()
            } else {
                "aucun son capté".into()
            });
        }
        let audio = waly_voice::pocket::resample_sinc(&brut, native_rate, TARGET_RATE);
        // Silence : Parakeet invente parfois sur du bruit — rien a transcrire.
        if audio.iter().fold(0.0f32, |m, s| m.max(s.abs())) < 0.01 {
            return Ok(String::new());
        }
        let mut texte = String::new();
        for morceau in audio.chunks(TARGET_RATE as usize * 25) {
            if morceau.len() < TARGET_RATE as usize / 4 {
                continue;
            }
            let t = stt.transcribe(morceau, TARGET_RATE as i32)?;
            if !t.is_empty() {
                if !texte.is_empty() {
                    texte.push(' ');
                }
                texte.push_str(&t);
            }
        }
        Ok(texte)
    })();
    match res {
        Ok(t) => dire(&format!("TEXTE {}", serde_json::to_string(&t)?)),
        Err(e) => dire(&format!("ERREUR {}", serde_json::to_string(&e.to_string())?)),
    }
    Ok(())
}

/// STT Parakeet sur un fichier WAV (16 kHz mono s16) via sherpa-onnx.
fn cmd_stt(wav_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    use waly_voice::stt::ParakeetStt;

    let dll = std::env::var("WALY_SHERPA_DLL")
        .unwrap_or_else(|_| waly_core::chemins::lib_sherpa("sherpa-onnx-c-api"));
    let model_dir = std::env::var("WALY_PARAKEET_DIR")
        .unwrap_or_else(|_| waly_core::chemins::modele("parakeet-tdt-0.6b-v3-int8"));

    let mut reader = hound::WavReader::open(wav_path)?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .map(|&s| s as f32 / i16::MAX as f32)
            .collect(),
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
    };
    println!(
        "WAV: {:.1} s a {} Hz ({} canaux)",
        samples.len() as f32 / (spec.sample_rate * spec.channels as u32) as f32,
        spec.sample_rate,
        spec.channels
    );

    let t0 = Instant::now();
    let stt = ParakeetStt::new(&dll, &model_dir, 4)?;
    println!("Parakeet charge en {:.2} s", t0.elapsed().as_secs_f64());

    for passe in 1..=3 {
        let t0 = Instant::now();
        let text = stt.transcribe(&samples, spec.sample_rate as i32)?;
        println!("passe {passe}: {:.3} s -> {text}", t0.elapsed().as_secs_f64());
    }
    Ok(())
}

/// VAD Silero en direct : ecoute le micro, affiche la probabilite de parole
/// et les transitions debut/fin de tour (hysteresis 800 ms).
fn cmd_vad(secs: u64) -> Result<(), Box<dyn std::error::Error>> {
    use waly_voice::segment::{SpeechEdge, SpeechSegmenter, FRAME};
    use waly_voice::vad::{self, SileroVad};

    vad::init_onnxruntime()?;
    let model = std::env::var("WALY_SILERO_PATH")
        .unwrap_or_else(|_| waly_core::chemins::modele("silero_vad.onnx"));
    let mut silero = SileroVad::new(&model)?;
    let mut segmenter = SpeechSegmenter::new(800);
    println!("Silero v5 charge ({model}). Parle, puis laisse 800 ms de silence ({secs} s au total)...");

    let (device, config) = open_input(None)?;
    let native_rate = config.sample_rate().0;
    let sink = Arc::new(Mutex::new(Vec::<f32>::new()));
    let _stream =
        start_capture(&device, &config, Arc::clone(&sink), Arc::new(AtomicBool::new(false)))?;

    let start = Instant::now();
    let mut pending: Vec<f32> = Vec::new(); // trames 16 kHz en attente
    let mut frames_seen: u64 = 0;
    while start.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_millis(50));
        let chunk: Vec<f32> = std::mem::take(&mut *lock_audio(&sink));
        pending.extend(resample_linear(&chunk, native_rate, TARGET_RATE));
        while pending.len() >= FRAME {
            let frame: Vec<f32> = pending.drain(..FRAME).collect();
            let prob = silero.process(&frame)?;
            frames_seen += 1;
            match segmenter.push(prob) {
                SpeechEdge::Started => println!(
                    "  [{:6.2}s] DEBUT de parole (p={prob:.2})",
                    start.elapsed().as_secs_f32()
                ),
                SpeechEdge::Ended => println!(
                    "  [{:6.2}s] FIN de tour (800 ms de silence)",
                    start.elapsed().as_secs_f32()
                ),
                SpeechEdge::None => {
                    // Une ligne d'etat toutes les ~16 trames (0,5 s)
                    if frames_seen % 16 == 0 {
                        let bars = (prob * 30.0) as usize;
                        println!("  p={prob:.2} [{:<30}]{}", "#".repeat(bars),
                            if segmenter.speaking() { " <parole>" } else { "" });
                    }
                }
            }
        }
    }
    Ok(())
}

// ── Sortie audio : lecteur interruptible ─────────────────────────────────────

/// File d'echantillons mono jouee par un stream cpal ; `clear()` = barge-in.
struct Player {
    queue: Arc<Mutex<std::collections::VecDeque<f32>>>,
    out_rate: u32,
    /// Ce qui est REELLEMENT sorti des haut-parleurs (mono, out_rate), au
    /// moment ou ca sort — alimente la garde anti-echo. Borne a ~3 s si
    /// personne ne le draine.
    tap: Arc<Mutex<Vec<f32>>>,
    _stream: cpal::Stream,
}

impl Player {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("aucun peripherique de sortie")?;
        let config = device.default_output_config()?;
        let out_rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        let queue = Arc::new(Mutex::new(std::collections::VecDeque::<f32>::new()));
        let tap = Arc::new(Mutex::new(Vec::<f32>::new()));
        let tap_cap = out_rate as usize * 3;
        let err_fn = |e| eprintln!("erreur sortie audio: {e}");
        // La capture gere F32 et I16 : la sortie doit faire pareil (certains
        // peripheriques Windows n'exposent que l'entier 16 bits).
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                let (q, t) = (Arc::clone(&queue), Arc::clone(&tap));
                device.build_output_stream(
                    &config.config(),
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        let played = drain_out_queue(&q, &t, tap_cap, data.len() / channels);
                        for (frame, s) in data.chunks_mut(channels).zip(&played) {
                            for out in frame.iter_mut() {
                                *out = *s;
                            }
                        }
                    },
                    err_fn,
                    None,
                )?
            }
            cpal::SampleFormat::I16 => {
                let (q, t) = (Arc::clone(&queue), Arc::clone(&tap));
                device.build_output_stream(
                    &config.config(),
                    move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                        let played = drain_out_queue(&q, &t, tap_cap, data.len() / channels);
                        for (frame, s) in data.chunks_mut(channels).zip(&played) {
                            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                            for out in frame.iter_mut() {
                                *out = v;
                            }
                        }
                    },
                    err_fn,
                    None,
                )?
            }
            other => return Err(format!("format de sortie non gere: {other:?}").into()),
        };
        stream.play()?;
        Ok(Self { queue, out_rate, tap, _stream: stream })
    }

    /// Ajoute de l'audio (mono, `rate` Hz) a la file, reechantillonne.
    /// Par petits blocs : ne jamais tenir le verrou longtemps face au
    /// callback audio (un extend de plusieurs secondes = underrun).
    fn push(&self, samples: &[f32], rate: u32) {
        let resampled = resample_linear(samples, rate, self.out_rate);
        for part in resampled.chunks(4096) {
            lock_audio(&self.queue).extend(part.iter().copied());
        }
    }

    fn is_idle(&self) -> bool {
        lock_audio(&self.queue).is_empty()
    }

    /// Barge-in : vide la file immediatement.
    fn clear(&self) {
        lock_audio(&self.queue).clear();
    }
}

/// Depile `n_frames` echantillons mono de la file de lecture et les copie
/// dans le tap anti-echo (borne). Partage par les callbacks F32 et I16.
fn drain_out_queue(
    q: &Mutex<std::collections::VecDeque<f32>>,
    t: &Mutex<Vec<f32>>,
    tap_cap: usize,
    n_frames: usize,
) -> Vec<f32> {
    let mut queue = lock_audio(q);
    let mut played = Vec::with_capacity(n_frames);
    for _ in 0..n_frames {
        played.push(queue.pop_front().unwrap_or(0.0));
    }
    drop(queue);
    let mut tap = lock_audio(t);
    tap.extend_from_slice(&played);
    if tap.len() > tap_cap {
        let cut = tap.len() - tap_cap;
        tap.drain(..cut);
    }
    played
}

// ── Cascade : LLM streaming -> clauses -> TTS -> lecteur ────────────────────

const SYSTEM_PROMPT: &str = "Tu es Waly, assistant vocal francais. Reponds de facon breve et naturelle, comme a l'oral : une a trois phrases COURTES, sans listes ni markdown. Tu CONNAIS l'heure et la date : l'horodatage [jour date, heure] devant chaque message utilisateur est l'heure locale REELLE — quand on te demande l'heure, donne FIDELEMENT celle du DERNIER message, en toutes lettres, a la minute pres (00:21 = minuit vingt et une, jamais d'arrondi), et deduis le moment correctement (0h-6h : la nuit, pas le matin). Ne recopie jamais l'horodatage tel quel et n'en mentionne jamais l'existence. Ta ponctuation est expressive quand c'est justifie (? ! ...) car ta voix la suit, mais ton style reste SIMPLE et DIRECT : pas de metaphores poetiques, pas de comparaisons fleuries, pas d'emojis en serie, et tu n'inventes jamais de citations. IMPORTANT : tu as des outils LOCAUX (memoire, notes, taches, rappels, heure, fichiers - lire, lister, chercher, creer) - utilise-les sans les commenter. Quand ton utilisateur te demande de retenir/noter quelque chose ou te confie une info durable (rendez-vous, preference, fait), appelle memoriser IMMEDIATEMENT puis confirme en une phrase - ne demande JAMAIS la permission de retenir et ne pretends jamais avoir agi sans avoir appele l'outil. Pour un rappel, calcule la date exacte AAAA-MM-JJ HH:MM depuis l'heure que tu connais. Tu n'as NI meteo, NI web, NI corps : propose uniquement ce que TOI tu peux faire ici (discuter, expliquer, retenir, raconter, lire ou creer des fichiers), jamais des objets ou sorties physiques. Si on te demande une information en temps reel, dis-le honnetement en une phrase ; ne promets JAMAIS de chercher, verifier ou tenir au courant. /no_think";

/// Horodatage local en francais : « vendredi 4 juillet 2026, 14:32 ».
/// Prefixe au message utilisateur (PAS au prompt systeme : lui reste
/// identique octet pour octet pour le cache de prefixe FLM).
fn french_timestamp() -> String {
    use chrono::{Datelike, Local, Timelike};
    const JOURS: [&str; 7] = ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"];
    const MOIS: [&str; 12] = [
        "janvier", "fevrier", "mars", "avril", "mai", "juin",
        "juillet", "aout", "septembre", "octobre", "novembre", "decembre",
    ];
    let now = Local::now();
    format!(
        "{} {} {} {}, {:02}:{:02}",
        JOURS[now.weekday().num_days_from_monday() as usize],
        now.day(),
        MOIS[now.month0() as usize],
        now.year(),
        now.hour(),
        now.minute()
    )
}

/// Moteur TTS de la cascade : Piper (rapide, defaut) ou Pocket NATIF
/// (qualite + clonage de voix, opt-in via WALY_TTS=pocket ; pipeline maison
/// sur ort, seul capable des bundles schema 2 dont le francais). Un SEUL
/// charge a la fois (budget memoire signe : Pocket 24l ~800 Mo residents,
/// modeles + etat voix f32).
enum Tts {
    Piper(waly_voice::tts::PiperTts),
    Pocket(waly_voice::pocket::PocketNative),
}

impl Tts {
    fn synth(&self, text: &str, speed: f32) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        match self {
            Tts::Piper(t) => t.synth(text, speed),
            Tts::Pocket(t) => t.synth(text, speed),
        }
    }
    /// Synthese STREAMING uniforme (R-V) : Pocket streame vraiment (premier
    /// chunk pendant la generation, chunk vide = sonde d'interruption) ;
    /// Piper rend son audio en un chunk (90 ms par clause, streamer serait
    /// du bruit). `speed` : suivi par Piper ; SANS EFFET sur Pocket (modele
    /// de langage audio, pas de length_scale) — la prosodie pocket vient du
    /// modele lui-meme + la ponctuation, les pauses restent au lecteur.
    /// Retourne Ok(false) si `on_audio` a demande l'arret (barge-in).
    fn synth_stream(
        &self,
        text: &str,
        speed: f32,
        on_audio: &mut dyn FnMut(&[f32]) -> bool,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        match self {
            Tts::Piper(t) => {
                let samples = t.synth(text, speed)?;
                Ok(on_audio(&samples))
            }
            Tts::Pocket(t) => t.synth_stream(text, on_audio),
        }
    }
    fn sample_rate(&self) -> u32 {
        match self {
            Tts::Piper(t) => t.sample_rate(),
            Tts::Pocket(t) => t.sample_rate(),
        }
    }
}

struct Cascade {
    stt: waly_voice::stt::ParakeetStt,
    tts: Tts,
    /// Client du CŒUR (waly-core) : streaming + tool-calling natif.
    llm: waly_core::llm::LlmClient,
    player: Player,
    history: Vec<(String, String)>,
    /// Fenetre de contexte PERSISTANTE entre les tours (discipline
    /// append-only, GATE A R4.5) : le cache FLM ne survit que si chaque tour
    /// ETEND la conversation precedente verbatim. Rebatie aux paliers
    /// seulement (rebuild_messages).
    messages: Vec<waly_core::llm::Msg>,
    /// Curseur du delta visuel « depuis ton dernier tour » (R4.5 ch. 2) :
    /// id du dernier evenement du journal deja raconte. Au demarrage =
    /// dernier id existant (le 1er tour raconte depuis le debut de l'appel).
    vu_jusqu_a: i64,
    /// Phrase d'un tour ANNULE avant le premier mot, a recoller au tour
    /// suivant (vecu 2026-07-08, appel avec un tiers qui parle : chaque
    /// tour se faisait tuer par la parole suivante et la phrase etait
    /// JETEE — en serie, « Waly ne repond plus »).
    transcript_en_attente: String,
    /// Pouls publie au desktop (R4.5 ch. 4) : l'eclipse de l'ecran d'appel
    /// respire avec la voix. None hors mode appel.
    pouls: Option<std::sync::Arc<PoulsVoix>>,
    /// Micro coupe (R5) : le pop-up ecran le bascule, un thread le lit sur
    /// /mic du desktop. Quand true, la boucle audio JETTE le son (Waly ne
    /// respecte pas le mute systeme — il lit le device brut).
    muted: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Le cerveau R2 : mémoire SQLite + outils + approbations.
    conn: std::rc::Rc<rusqlite::Connection>,
    registry: waly_core::tools::Registry,
}

fn cascade_init() -> Result<Cascade, Box<dyn std::error::Error>> {
    let dll = std::env::var("WALY_SHERPA_DLL")
        .unwrap_or_else(|_| waly_core::chemins::lib_sherpa("sherpa-onnx-c-api"));
    let parakeet_dir = std::env::var("WALY_PARAKEET_DIR")
        .unwrap_or_else(|_| waly_core::chemins::modele("parakeet-tdt-0.6b-v3-int8"));
    let cfg = waly_voice::VoiceConfig::default();
    cfg.validate().map_err(|e| format!("config voix invalide: {e}"))?;

    let t0 = Instant::now();
    let stt = waly_voice::stt::ParakeetStt::new(&dll, &parakeet_dir, 4)?;
    println!("Parakeet charge ({:.1} s)", t0.elapsed().as_secs_f64());

    let t0 = Instant::now();
    // VERDICT R-V GRAVE (Michee, 2026-07-20) : Pocket est LA voix de Waly —
    // fabien (masculine, defaut) / developpeuse (feminine, WALY_TTS_SPEAKER=0
    // ou lanceur -Voix femme). Pierre juge « trop robotique ». Piper reste le
    // moteur de SECOURS explicite : WALY_TTS=piper.
    let tts = if std::env::var("WALY_TTS").as_deref() != Ok("piper") {
        let model_dir = std::env::var("WALY_POCKET_DIR")
            .unwrap_or_else(|_| waly_core::chemins::modele("pocket-tts-fr-24l"));
        waly_voice::vad::init_onnxruntime()?;
        let threads = std::env::var("WALY_POCKET_THREADS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4);
        let mut p = waly_voice::pocket::PocketNative::new(&model_dir, threads)?;
        // Le clonage par WAV de reference est LE chemin (les etats
        // safetensors officiels : abandonnes, voix anglophones). La voix
        // feminine reutilise le canal WALY_TTS_SPEAKER=0 du lanceur.
        let femme = std::env::var("WALY_TTS_SPEAKER").as_deref() == Ok("0");
        let ref_wav = std::env::var("WALY_TTS_REF").unwrap_or_else(|_| {
            if femme {
                waly_core::chemins::engines_fichier("voices-fr/developpeuse.wav")
            } else {
                waly_core::chemins::engines_fichier("voices-fr/fabien.wav")
            }
        });
        let (reference, ref_rate) = read_wav_mono(&ref_wav)?;
        p.set_voice(&reference, ref_rate)?;
        let voice_label = ref_wav;
        println!(
            "Pocket TTS natif charge ({:.1} s, {} Hz, voix: {})",
            t0.elapsed().as_secs_f64(),
            p.sample_rate(),
            voice_label
        );
        Tts::Pocket(p)
    } else {
        let piper_dir = waly_core::chemins::modele(&format!("vits-piper-{}", cfg.tts_voice));
        let mut p = waly_voice::tts::PiperTts::new(&dll, &piper_dir, &cfg.tts_voice, 2)?;
        // Locuteur : pierre (1, defaut) ou jessica (0, la feminine de secours)
        // via WALY_TTS_SPEAKER / lanceur -Voix femme.
        let speaker = std::env::var("WALY_TTS_SPEAKER")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(cfg.tts_speaker);
        p.set_speaker(speaker);
        println!(
            "Piper {} (locuteur {}) charge ({:.1} s, {} Hz)",
            cfg.tts_voice,
            if speaker == 1 { "pierre" } else { "jessica" },
            t0.elapsed().as_secs_f64(),
            p.sample_rate()
        );
        Tts::Piper(p)
    };
    // Modele : meme verite que le desktop et le CLI (WALY_MODEL › waly.toml
    // [llm] modele › defaut) — la config de la lib voix ne lit pas waly.toml.
    let mut llm = waly_core::llm::LlmClient::new(
        "127.0.0.1",
        waly_core::llm::port_par_defaut(),
        &waly_core::llm::modele_par_defaut(),
    );
    // 160 : un assistant VOCAL n'a pas droit aux tirades (tirades de 11 s
    // vecues le 2026-07-05 a 256 tokens).
    llm.max_tokens = 160;
    let player = Player::new()?;

    // Le cerveau R2 : base memoire/approbations + outils coeur. L'embedder
    // partage l'init ort du VAD (un seul Once, via waly-core).
    let db_path = waly_core::store::chemin_par_defaut();
    if db_path != ":memory:" {
        if let Some(dir) = std::path::Path::new(&db_path).parent() {
            std::fs::create_dir_all(dir).ok();
        }
    }
    let conn = std::rc::Rc::new(
        waly_core::store::open(&db_path).map_err(|e| format!("base {db_path}: {e}"))?,
    );
    let embed_dir = std::env::var("WALY_EMBED_DIR")
        .unwrap_or_else(|_| waly_core::chemins::modele("e5-small-int8"));
    let embedder = match waly_core::embed::Embedder::load(&embed_dir) {
        Ok(e) => Some(std::rc::Rc::new(std::cell::RefCell::new(e))),
        Err(e) => {
            eprintln!("(memoire semantique indisponible: {e})");
            None
        }
    };
    let mut registry = waly_core::tools::Registry::new();
    waly_core::native_tools::register_core_tools(&mut registry, conn.clone(), embedder);
    waly_core::fichiers::register_fichier_tools(
        &mut registry,
        waly_core::fichiers::PolitiqueFichiers::defaut(),
    );
    // Mode appel (R4) : le desktop possede la camera et expose le cliche sur
    // un port loopback — l'outil regarder n'existe a la voix QUE dans ce cas
    // (pas d'outil fantome en standalone).
    // Honnetete (2026-09-14) : le port d'appel est passe a TOUTES les voix
    // (voix seule, veille) — l'outil n'existe que si la CAMERA tourne.
    if let Some(port) = appel_port().filter(|_| camera()) {
        waly_core::native_tools::register_vision_tool(
            &mut registry,
            Box::new(move || {
                let jpeg = appel_get(port, "/cliche")?;
                Ok(waly_core::native_tools::ImageCapturee {
                    data_url: format!(
                        "data:image/jpeg;base64,{}",
                        waly_core::native_tools::base64(&jpeg)
                    ),
                    largeur: 0,
                    hauteur: 0,
                })
            }),
        );
        println!("Mode appel : outil regarder branche (service desktop port {port})");
    }
    // Mode ECRAN (R5) : l'outil regarder_ecran lit /cliche-ecran du desktop
    // (image seule — l'OCR reste cote desktop pour le chat tape ; la voix fait
    // de la comprehension visuelle, ce qu'elle demande a l'oral).
    if mode_ecran() {
        if let Some(port) = appel_port() {
            waly_core::native_tools::register_vision_ecran_tool(
                &mut registry,
                Box::new(move |_cadrage| {
                    let jpeg = appel_get(port, "/cliche-ecran")?;
                    Ok(waly_core::native_tools::CaptureEcran {
                        image: waly_core::native_tools::ImageCapturee {
                            data_url: format!(
                                "data:image/jpeg;base64,{}",
                                waly_core::native_tools::base64(&jpeg)
                            ),
                            largeur: 0,
                            hauteur: 0,
                        },
                        texte_ocr: String::new(),
                        conf: 0.0,
                    })
                }),
            );
            println!("Mode ecran : outil regarder_ecran branche (/cliche-ecran port {port})");
        }
    }

    // Warmup LLM : fait APRES la construction de la cascade (prechauffage du
    // PREFIXE systeme+outils, cf. fin de cascade_init) — il remplace l'ancien
    // warmup "ok", qui chargeait le modele sans mettre le prompt en cache.
    // Premiere synthese TTS payee ici.
    let _ = tts.synth("Un.", 1.0);

    // Reprendre le fil des sessions precedentes : les derniers echanges
    // persistes reamorcent la fenetre (le reste est en base, et les
    // souvenirs durables vivent dans user_memory de toute facon).
    let history =
        waly_core::store::recent_messages_in(&conn, session(), 8).unwrap_or_default();
    if !history.is_empty() {
        println!("Contexte repris : {} messages des sessions precedentes", history.len());
    }

    // WALY_VU_CURSOR : surcharge de banc (rejouer le delta depuis un id).
    let vu_jusqu_a = std::env::var("WALY_VU_CURSOR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| {
            waly_core::store::visual_memory_last_id(&conn, waly_core::store::MAIN_SESSION)
                .unwrap_or(0)
        });
    // Mute micro (R5) : un thread sonde /mic du desktop (~250 ms) et met a jour
    // l'atomique que la boucle audio lit. Dans TOUS les modes lances par le
    // desktop : l'appel voix/video a aussi son bouton « Couper le micro »
    // (il ne coupait rien hors mode ecran — corrige 2026-09-29).
    let muted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    if let Some(port) = appel_port() {
        let m = muted.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if let Ok(body) = appel_get(port, "/mic") {
                m.store(body.first() == Some(&b'1'), std::sync::atomic::Ordering::Relaxed);
            }
        });
    }
    let mut c = Cascade {
        stt,
        tts,
        llm,
        player,
        history,
        messages: Vec::new(),
        vu_jusqu_a,
        transcript_en_attente: String::new(),
        pouls: appel_port().map(demarrer_pouls),
        muted,
        conn,
        registry,
    };
    rebuild_messages(&mut c);
    // Prechauffage du PREFIXE (2026-09-10) : sur Ollama, le 1er tour payait
    // 20+ s de prefill [systeme + outils] (mesure : 23,1 s puis 3,4-3,6 s) ;
    // en appel, l'utilisateur reparlait avant la reponse et le barge-in
    // annulait tout (journal du 04-09 : « tour annule avant le premier mot »).
    // Paye ici, une fois, AVANT d'ecouter. Non-streaming, lu jusqu'a EOF
    // (piege 7) ; charge aussi le modele (remplace l'ancien warmup).
    if let Some(sys) = c.messages.first().cloned() {
        let t0 = Instant::now();
        match c.llm.prechauffer_prefixe(&sys, &c.registry.specs()) {
            Ok(()) => println!("LLM chaud, prefixe en cache ({:.1} s)", t0.elapsed().as_secs_f64()),
            Err(e) => eprintln!("attention, prechauffage LLM rate: {e}"),
        }
    }
    Ok(c)
}

/// (Re)batit la fenetre persistante : systeme STABLE (base + mode appel si
/// actif + souvenirs + consigne d'attentes) + les 8 derniers echanges de
/// l'historique compact. C'est le PALIER assume — le tour qui suit re-paie
/// son prefill entier (cache FLM append-only, GATE A R4.5). A n'appeler
/// qu'aux bornes : demarrage, fenetre > 25, resynchronisation apres erreur.
fn rebuild_messages(c: &mut Cascade) {
    let mut base = String::from(SYSTEM_PROMPT);
    // Honnetete (vecu 2026-09-14 : voix seule, camera eteinte, Waly decrivait
    // une piece inventee) : sans camera ni ecran, il ne voit RIEN.
    base.push_str(
        " Quand l'en-tete [ ... ] dit « camera eteinte », tu ne vois RIEN : ne \
         decris jamais une scene, une personne ou un ecran, et ne pretends pas \
         avoir regarde — dis simplement que ta camera est eteinte.",
    );
    // Le delta visuel (ch. 2) doit etre UTILISABLE : sans cette permission
    // explicite, la consigne « ne mentionne jamais l'en-tete » fait ignorer
    // tout le crochet (vecu au banc : « je suis la, je t'ecoute » face a un
    // depart/retour raconte dans l'en-tete). Camera allumee seulement.
    if camera() {
        base.push_str(
            " Si l'en-tete [ ... ] d'un message contient « depuis ton dernier \
             tour », c'est ce que tu as VU entre-temps : reagis-y naturellement \
             quand c'est pertinent (une absence, un retour — « je t'ai vu \
             sortir », « te revoila »), sans reciter l'en-tete ni les heures \
             entre parentheses.",
        );
    }
    if camera() && !mode_ecran() {
        base.push_str(
            " MODE APPEL VIDEO : quand l'en-tete [ ... ] du message dit \
             « en appel video », ta camera est active et tu es capable de \
             voir. L'en-tete ne te dit QUE la presence de ton utilisateur, son \
             attention et son air — il ne decrit PAS la scene : pour dire ce \
             que tu vois, appelle D'ABORD l'outil regarder et decris l'image \
             recue ; ne devine jamais une scene sans l'avoir appelee au tour \
             courant, et ne commente ni l'outil ni l'en-tete.",
        );
    }
    if mode_ecran() {
        base.push_str(
            " MODE ECRAN : tu regardes l'ecran de ton utilisateur et tu es capable de \
             le VOIR. Quand il dit « regarde mon ecran » (ou pour dire ce qui \
             est affiche), l'image de l'ecran t'est fournie : decris ce que tu \
             vois BRIEVEMENT et utilement (l'application, le contenu, une \
             erreur). ⚠ IMPORTANT : le petit POP-UP Waly (icones, « Ecran », \
             « Stop ») et le CADRE lumineux autour de l'ecran sont TA PROPRE \
             interface — IGNORE-les completement, ne les decris JAMAIS. Decris \
             le TRAVAIL de ton utilisateur : ses applications (VS Code, navigateur, \
             terminal, document…), leur contenu. Ne devine jamais sans avoir \
             regarde au tour courant, et ne commente ni l'outil ni l'en-tete.",
        );
    }
    c.messages.clear();
    let mut system = waly_core::prompt::inject_context(&base, &c.conn);
    // Journal visuel (R4.5 ch. 1) : ce que la camera a vu recemment. PAS en
    // mode ecran (retour Michee 2026-07-10 : il ancrait Waly sur de vieilles
    // scenes ; la vision ecran est fraiche par tour).
    if !mode_ecran() {
        system.push_str(&waly_core::prompt::memoire_visuelle(
            &c.conn,
            waly_core::store::MAIN_SESSION,
        ));
    }
    // Conscience du huis clos (R6a) : au rebuild (append-only), etat REEL du
    // sceau — surtout a l'oral, ou Michee demande « cherche X sur internet ».
    system.push_str(waly_core::prompt::conscience_sceau(waly_core::sceau::actif()));
    c.messages.push(waly_core::llm::Msg::System(system));
    let start = c.history.len().saturating_sub(8);
    for (role, content) in &c.history[start..] {
        c.messages.push(match role.as_str() {
            "user" => waly_core::llm::Msg::User(content.clone()),
            _ => waly_core::llm::Msg::Assistant(content.clone()),
        });
    }
}

/// Port du service d'appel du desktop (pose au spawn compagnon). Absent =
/// voix standalone, pas de camera, pas de conscience d'appel.
fn appel_port() -> Option<u16> {
    std::env::var("WALY_APPEL_PORT").ok()?.parse().ok()
}

/// Mode ECRAN (R5) : la voix voit l'ECRAN (via /cliche-ecran) au lieu de la
/// camera. Pose par le desktop au spawn (`WALY_ECRAN_MODE=1`).
fn mode_ecran() -> bool {
    std::env::var("WALY_ECRAN_MODE").as_deref() == Ok("1")
}

/// La CAMERA tourne vraiment (appel video) : pose par le desktop au spawn
/// (`WALY_CAMERA=1`). Voix seule, veille, ecran : non.
fn camera() -> bool {
    std::env::var("WALY_CAMERA").as_deref() == Ok("1")
}

/// Session ou ranger les MESSAGES parles. UI 2026-09-14 (decision Michee) :
/// plus de « Fil principal » — la voix ecrit dans la conversation OUVERTE dans
/// l'app, demandee au desktop a chaque ecriture (`GET /session`), ce qui suit
/// aussi la veille reveillee et un changement de conversation. Sans desktop :
/// `WALY_SESSION`, sinon la session par defaut. Le JOURNAL visuel
/// (souvenirs/delta) reste GLOBAL sur MAIN_SESSION.
fn session() -> i64 {
    if let Some(port) = appel_port() {
        if let Ok(b) = appel_get(port, "/session") {
            if let Ok(id) = String::from_utf8_lossy(&b).trim().parse::<i64>() {
                return id;
            }
        }
    }
    std::env::var("WALY_SESSION")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(waly_core::store::MAIN_SESSION)
}

/// Le pouls de la voix (R4.5 ch. 4) : etat de tour + niveau audio, publies
/// au desktop pour l'eclipse. Atomiques (ecrits depuis la cascade, lus par
/// le thread editeur) — etats : 0 repos, 1 ecoute, 2 reflexion, 3 parole.
struct PoulsVoix {
    etat: std::sync::atomic::AtomicU8,
    niveau_milli: std::sync::atomic::AtomicU32,
}

impl PoulsVoix {
    fn poser(&self, etat: u8, niveau: f32) {
        use std::sync::atomic::Ordering;
        self.etat.store(etat, Ordering::Relaxed);
        self.niveau_milli
            .store((niveau.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
    }
}

/// Lance le thread editeur du pouls : POST /pouls toutes les ~120 ms,
/// fire-and-forget — un rate ne touche JAMAIS la boucle audio (c'est la
/// raison du thread dedie).
fn demarrer_pouls(port: u16) -> std::sync::Arc<PoulsVoix> {
    use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
    let p = std::sync::Arc::new(PoulsVoix {
        etat: AtomicU8::new(0),
        niveau_milli: AtomicU32::new(0),
    });
    let t = p.clone();
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        loop {
            std::thread::sleep(Duration::from_millis(120));
            let etat = match t.etat.load(Ordering::Relaxed) {
                1 => "ecoute",
                2 => "reflexion",
                3 => "parole",
                _ => "repos",
            };
            let niveau = t.niveau_milli.load(Ordering::Relaxed) as f32 / 1000.0;
            let corps = format!("{{\"etat\":\"{etat}\",\"niveau\":{niveau:.3}}}");
            let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else {
                continue;
            };
            let _ = write!(
                s,
                "POST /pouls HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{corps}",
                corps.len()
            );
            let mut b = [0u8; 64];
            let _ = s.read(&mut b); // reponse 204 lue, connexion rendue
        }
    });
    p
}

/// RMS d'un bloc d'echantillons f32 (niveau du pouls).
fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt()
}

/// GET minuscule sur le service d'appel (loopback). Reponse lue jusqu'a EOF
/// (le serveur ferme apres ecriture) — JAMAIS de read_timeout (piege
/// Winsock n°7).
fn appel_get(port: u16, path: &str) -> Result<Vec<u8>, String> {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port))
        .map_err(|e| format!("service d'appel injoignable: {e}"))?;
    write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let pos = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("reponse sans en-tetes")?;
    let head = String::from_utf8_lossy(&buf[..pos]);
    if !head.starts_with("HTTP/1.1 200") {
        return Err(format!("service d'appel: {}", head.lines().next().unwrap_or("?")));
    }
    Ok(buf[pos + 4..].to_vec())
}

/// La CONSCIENCE D'APPEL (retour Michee 2026-07-08 : « il ne sait pas qu'il
/// est en visio ») — forme COMPACTE pour l'en-tete frais du message (la
/// semantique « tu es capable de voir » vit dans le prompt systeme STABLE,
/// pose par rebuild_messages ; ici seulement l'etat de l'instant). None hors
/// appel ou service muet.
fn conscience_appel() -> Option<String> {
    let port = appel_port()?;
    let body = appel_get(port, "/etat").ok()?;
    let v: serde_json::Value = serde_json::from_slice(&body).ok()?;
    if !v["actif"].as_bool().unwrap_or(false) {
        return None;
    }
    Some(if v["present"].as_bool().unwrap_or(false) {
        let mut t = String::from("en appel video : tu vois ton utilisateur");
        match v["vers_ecran"].as_bool() {
            Some(true) => t.push_str(", attentif a l'ecran"),
            Some(false) => t.push_str(", le regard ailleurs"),
            None => {}
        }
        t
    } else {
        "en appel video : ton utilisateur est hors du champ de la camera".to_string()
    })
}

/// Journal de bord du tour parle (append) : la console du compagnon est
/// INVISIBLE (spawn sans fenetre par le desktop) — chaque tour laisse une
/// trace diagnosticable (vecu 2026-07-08 : « image floue » hallucine en
/// appel, irreproductible faute de log). WALY_VOICE_LOG pour surcharger.
fn journal(ligne: &str) {
    use std::io::Write;
    let path = std::env::var("WALY_VOICE_LOG")
        .unwrap_or_else(|_| waly_core::chemins::data_fichier("waly-voice.log"));
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "[{}] {ligne}", french_timestamp());
    }
}

/// Deroule un tour a partir d'un transcript : LLM streaming -> clauses ->
/// TTS -> lecteur. `interrupted` est sondee pendant le flux (barge-in).
/// Retourne la reponse complete (vide si interrompue tot).
fn respond(
    c: &mut Cascade,
    transcript: &str,
    clock: &mut waly_voice::TurnClock,
    interrupted: impl FnMut() -> bool,
) -> Result<String, Box<dyn std::error::Error>> {
    use std::cell::RefCell;
    use waly_voice::{clean_for_tts, has_speech, ClauseSplitter};

    // Discipline append-only (GATE A R4.5) : le systeme et l'historique ne
    // bougent PAS — le frais du tour (conscience d'appel, attentes) part
    // dans l'en-tete du message utilisateur, en QUEUE de conversation.
    // Conscience : ecran (fixe) en mode ecran, sinon presence camera de l'appel.
    let conscience = if mode_ecran() {
        Some("tu regardes l'ecran de ton utilisateur".to_string())
    } else if camera() {
        conscience_appel()
    } else {
        None
    };
    let en_appel = camera() && conscience.is_some();
    // Honnetete : sans camera ni ecran, l'en-tete le DIT.
    let conscience =
        conscience.or_else(|| Some("camera eteinte : tu ne vois rien en ce moment".to_string()));
    let frais = waly_core::prompt::en_tete_frais(&c.conn, conscience.as_deref());
    // Delta visuel « depuis ton dernier tour » (R4.5 ch. 2) — curseur
    // restaure si le tour est annule avant le premier mot.
    let curseur_avant = c.vu_jusqu_a;
    // En mode ECRAN : pas de delta (il portait les moments/inferences qui
    // ancraient Waly sur de vieilles scenes — la vision ecran est fraiche par
    // tour, cf. retour Michee 2026-07-10). Camera : delta normal.
    let (delta, curseur) = if mode_ecran() || !en_appel {
        (String::new(), c.vu_jusqu_a)
    } else {
        waly_core::prompt::delta_visuel(&c.conn, waly_core::store::MAIN_SESSION, c.vu_jusqu_a)
    };
    c.vu_jusqu_a = curseur;
    // Pouls : le tour commence — Waly « reflechit » (l'eclipse le montre).
    if let Some(p) = &c.pouls {
        p.poser(2, 0.15);
    }
    let user_entry = format!("[{}{frais}{delta}] {}", french_timestamp(), transcript);
    journal(&format!("user: {user_entry}"));
    c.history.push(("user".into(), user_entry.clone()));
    let base = c.messages.len();
    // Raccourci d'intention vision (parite desktop) : l'image est JOINTE au
    // message et le tour se joue a ZERO outil — fiabilite (le 4B repondait
    // parfois de memoire sans re-regarder) + latence (un seul tour modele).
    let mut vision = false;
    // B1 (2026-09-11) : en mode ECRAN, l'ecran se LIT d'abord en TEXTE (arbre
    // d'accessibilite, /lecture-ecran du desktop) — tour texte au lieu d'un
    // tour visuel (86 s sur la machine de reference, vision deleguee). L'image
    // ne sert que si la demande est visuelle ou si l'arbre est pauvre.
    let mut lu_en_texte = false;
    if mode_ecran() && !waly_core::chat::intention_visuelle(transcript) {
        if let Some(port) = appel_port() {
            let lu = appel_get(port, "/lecture-ecran").and_then(|b| {
                serde_json::from_slice::<serde_json::Value>(&b).map_err(|e| e.to_string())
            });
            match lu {
                Ok(v) if v["riche"].as_bool() == Some(true) => {
                    waly_core::mains_ecran::degrader_lectures(&mut c.messages);
                    let texte = format!(
                        "[{}{frais}] {}\n(Reponds d'apres CETTE lecture de l'ecran, faite \
                         maintenant — jamais un souvenir. N'invente rien. Bref, a l'oral.)\n{transcript}",
                        french_timestamp(),
                        waly_core::mains_ecran::bloc_lecture_seule(
                            v["fenetre"].as_str().unwrap_or(""),
                            v["texte"].as_str().unwrap_or(""),
                        )
                    );
                    c.messages.push(waly_core::llm::Msg::User(texte));
                    journal("ecran: lecture TEXTE fraiche (arbre d'accessibilite)");
                    lu_en_texte = true;
                }
                Ok(_) => journal("ecran: arbre pauvre, repli image"),
                Err(e) => journal(&format!("lecture-ecran ratee ({e}), repli image")),
            }
        }
    }
    // R5 REVU (retour Michee 2026-07-10) : en mode ECRAN, CHAQUE tour joint une
    // image FRAICHE de l'ecran — Waly repond d'apres CE QU'IL VOIT MAINTENANT,
    // jamais d'un souvenir. Vision temps reel = fraiche a chaque question.
    if mode_ecran() && !lu_en_texte {
        if let Some(port) = appel_port() {
            match appel_get(port, "/cliche-ecran") {
                Ok(jpeg) => {
                    let data_url = format!(
                        "data:image/jpeg;base64,{}",
                        waly_core::native_tools::base64(&jpeg)
                    );
                    waly_core::chat::degrader_images(&mut c.messages);
                    let texte = format!(
                        "[{}{frais}] (Tu vois l'ecran de ton utilisateur MAINTENANT sur l'image jointe. \
                         Reponds d'apres CETTE image (jamais un souvenir). ⚠ N'INVENTE RIEN : \
                         ne cite un nom de fichier ou un texte precis que si tu le LIS vraiment \
                         sur l'image ; sinon dis « un fichier » sans le nommer, ou « je ne \
                         distingue pas le detail ». Ignore le pop-up et le cadre Waly, decris \
                         son travail. Bref.)\n{transcript}",
                        french_timestamp()
                    );
                    c.messages.push(waly_core::llm::Msg::UserImage { texte, data_url });
                    journal("ecran: image FRAICHE jointe (chaque tour)");
                    vision = true;
                }
                Err(e) => journal(&format!("cliche-ecran rate ({e}), tour normal")),
            }
        }
    }
    if !vision && en_appel && !mode_ecran() && waly_core::chat::intention_visuelle(transcript) {
        if let Some(port) = appel_port() {
            match appel_get(port, "/cliche") {
                Ok(jpeg) => {
                    let data_url = format!(
                        "data:image/jpeg;base64,{}",
                        waly_core::native_tools::base64(&jpeg)
                    );
                    waly_core::chat::degrader_images(&mut c.messages);
                    c.messages
                        .push(waly_core::llm::Msg::UserImage { texte: user_entry.clone(), data_url });
                    journal("raccourci vision: image jointe au message (zero outil)");
                    vision = true;
                }
                Err(e) => journal(&format!("raccourci vision rate ({e}), tour normal")),
            }
        }
    }
    if !vision && !lu_en_texte {
        c.messages.push(waly_core::llm::Msg::User(user_entry.clone()));
    }

    let clock = RefCell::new(clock);
    // Partagee entre le callback LLM et la synthese streaming (une clause
    // Pocket dure des secondes : la sonde d'interruption doit vivre DANS la
    // synthese, pas seulement entre deux deltas LLM).
    let interrupted = RefCell::new(interrupted);
    // Piper lit ce qu'on lui donne -> couper tot (virgule) gagne du percu.
    // Pocket est un modele de langage : les fragments le font halluciner
    // (sons parasites, mots avales) -> phrases entieres uniquement.
    let splitter = RefCell::new(match &c.tts {
        Tts::Pocket(_) => ClauseSplitter::sentences_only(),
        Tts::Piper(_) => ClauseSplitter::new(),
    });
    let think = RefCell::new(waly_voice::llm::ThinkFilter::new());
    let bracket = RefCell::new(waly_voice::llm::BracketFilter::new());
    let first_token = RefCell::new(true);
    let first_audio = RefCell::new(true);

    let pouls_speak = c.pouls.clone();
    // Parle une clause en STREAMING : les chunks partent au lecteur au fil
    // de la synthese (Pocket : premier son pendant la generation ; Piper :
    // un chunk unique). Pendant la synthese, la sonde d'interruption vit
    // dans le callback (chunk vide) — le barge-in coupe une clause Pocket
    // en cours au lieu d'attendre sa fin.
    let speak = |clause: &str| {
        // Prosodie sur la clause BRUTE : les emojis et la ponctuation du LLM
        // sont des indices d'emotion, AVANT que clean_for_tts les retire.
        let prosody = waly_voice::prosody::analyze(clause);
        let text = clean_for_tts(clause);
        if !has_speech(&text) {
            return;
        }
        if *first_audio.borrow() {
            clock.borrow_mut().mark_first_clause();
        }
        let sr = c.tts.sample_rate();
        // Respiration avant une pensee — jamais avant le premier son
        // du tour (la latence percue prime).
        if prosody.pre_pause_ms > 0 && !*first_audio.borrow() {
            c.player.push(&vec![0.0; sr as usize * prosody.pre_pause_ms as usize / 1000], sr);
        }
        // Tampon d'avance (Pocket seulement) : le RTF mesure oscille autour
        // de 0,9-1,1 — pousser le premier chunk immediatement expose a
        // l'underrun en fin de phrase (trou au milieu d'un mot). On retient
        // ~350 ms avant le premier push ; la generation etant ~temps reel,
        // ce credit couvre la gigue. WALY_POCKET_LEAD_MS pour les bancs.
        let lead_samples = match &c.tts {
            Tts::Pocket(_) => {
                let ms: usize = std::env::var("WALY_POCKET_LEAD_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(350);
                sr as usize * ms / 1000
            }
            Tts::Piper(_) => 0,
        };
        let mut lead_buf: Vec<f32> = Vec::new();
        let mut lead_done = lead_samples == 0;
        let push_out = |chunk: &[f32]| {
            if let Some(p) = &pouls_speak {
                p.poser(3, (rms(chunk) * 4.0).min(1.0));
            }
            c.player.push(chunk, sr);
            if *first_audio.borrow() {
                clock.borrow_mut().mark_first_audio();
                *first_audio.borrow_mut() = false;
            }
        };
        let result = c.tts.synth_stream(&text, prosody.speed, &mut |chunk: &[f32]| {
            if chunk.is_empty() {
                // Sonde d'interruption pendant la generation (~80 ms).
                return !(interrupted.borrow_mut())();
            }
            if !lead_done {
                lead_buf.extend_from_slice(chunk);
                if lead_buf.len() >= lead_samples {
                    let buf = std::mem::take(&mut lead_buf);
                    push_out(&buf);
                    lead_done = true;
                }
                return true;
            }
            push_out(chunk);
            true
        });
        // Clause plus courte que le tampon d'avance : la jouer quand meme.
        if !lead_buf.is_empty() {
            push_out(&lead_buf);
        }
        match result {
            Ok(true) => {
                if prosody.post_pause_ms > 0 {
                    c.player.push(&vec![0.0; sr as usize * prosody.post_pause_ms as usize / 1000], sr);
                }
            }
            // Interrompue (barge-in) : le callback LLM verra la meme
            // interruption au prochain delta et arretera le tour.
            Ok(false) => {}
            // Clause perdue pour l'oreille mais pas pour l'historique (elle
            // reste dans `full`) : signaler clairement le trou.
            Err(e) => eprintln!("TTS rate, clause sautee ({e}): {text}"),
        }
    };

    let received = RefCell::new(String::new());
    // Journal visuel : un tour qui a VU laisse sa description (R4.5 ch. 1).
    let vu_outil = std::cell::Cell::new(false);
    // Boucle agentique du CŒUR : le texte streame vers les clauses/TTS, les
    // rounds d'outils (memoire, heure, approbations) se jouent entre deux.
    // Tour vision (image jointe) : ZERO round — le modele decrit, sans
    // re-prefill du bloc d'outils (meme discipline que le desktop).
    let rounds = if vision { 0 } else { waly_core::chat::MAX_TOOL_ROUNDS };
    let reply = waly_core::chat::run_turn_stream_with(
        &c.llm,
        &c.registry,
        &mut c.messages,
        Some(&c.conn),
        rounds,
        |delta| {
            // delta vide = sonde d'interruption pendant une attente reseau
            // (TTFT) : on ne marque PAS le premier token dessus.
            if delta.is_empty() {
                return !(interrupted.borrow_mut())();
            }
            received.borrow_mut().push_str(delta);
            if *first_token.borrow() {
                clock.borrow_mut().mark_llm_first_token();
                *first_token.borrow_mut() = false;
            }
            let audible = think.borrow_mut().feed(delta);
            let audible =
                if audible.is_empty() { audible } else { bracket.borrow_mut().feed(&audible) };
            if !audible.is_empty() {
                for cl in splitter.borrow_mut().push(&audible) {
                    speak(&cl);
                }
            }
            !(interrupted.borrow_mut())()
        },
        |tool, result| {
            println!("  [outil {tool}] {result}");
            // Seule une VRAIE capture compte comme « vu » (journal visuel).
            if tool == "regarder" && !result.starts_with("erreur") {
                vu_outil.set(true);
            }
            let court: String = result.chars().take(200).collect();
            journal(&format!("outil {tool}: {court}"));
        },
    );
    let streamed = match reply {
        Ok(r) => r,
        Err(e) => {
            journal(&format!("ERREUR tour: {e}"));
            // Le debut de reponse a peut-etre DEJA ete prononce : l'inscrire
            // dans l'historique, sinon le modele ne saura pas qu'il l'a dit.
            let partial = received.borrow().trim().to_owned();
            if !partial.is_empty() {
                c.history.push(("assistant".into(), waly_voice::compact_reply(&partial, 220)));
            }
            // L'echec a pu laisser des paires outils orphelines dans la
            // fenetre : resynchroniser depuis l'historique (palier assume).
            rebuild_messages(c);
            return Err(e.into());
        }
    };
    let full = streamed.text;

    if let Some(rest) = splitter.borrow_mut().flush() {
        speak(&rest);
    }
    clock.borrow_mut().mark_llm_done();
    if streamed.truncated && !full.trim().is_empty() {
        // EOF serveur avant [DONE] : la phrase entendue etait peut-etre
        // coupee — le dire, plutot que de passer pour une fin normale.
        println!("  (attention: reponse tronquee, FLM a ferme le flux avant [DONE])");
    }
    // Retirer un horodatage singe de l'historique aussi, sinon le modele
    // voit ses propres reponses timbrees et l'habitude se renforce.
    let full = {
        let t = full.trim_start();
        match (t.starts_with('['), t.find(']')) {
            (true, Some(pos)) if pos < 80 => t[pos + 1..].trim_start().to_owned(),
            _ => full,
        }
    };
    journal(&format!(
        "waly{}: {}",
        if streamed.interrupted { " (interrompu)" } else { "" },
        if full.is_empty() { "(tour annule avant le premier mot)" } else { &full }
    ));
    if !full.is_empty() {
        // Historique compact (reprise/rebuild) : premieres phrases seulement.
        // La fenetre LIVE, elle, garde la reponse entiere poussee par le
        // core — sous cache append-only elle ne se re-paie plus.
        let compact = waly_voice::compact_reply(&full, 220);
        c.history.push(("assistant".into(), compact.clone()));
        // Persistance inter-sessions (vecu 2026-07-05 : « rappelle-moi ce
        // que je t'ai dit tout a l'heure » -> le redemarrage avait tout
        // efface, seul le bin texte ecrivait en base). Les timestamps des
        // messages user restent dans le contenu : ils datent l'echange.
        waly_core::store::append_message_in(&c.conn, session(), "user", &user_entry).ok();
        waly_core::store::append_message_in(&c.conn, session(), "assistant", &compact).ok();
        if vision || vu_outil.get() {
            waly_core::store::visual_memory_add(
                &c.conn,
                waly_core::store::MAIN_SESSION,
                "vu",
                &waly_core::prompt::compacte(&full, 240),
            )
            .ok();
        }
    } else {
        // Tour annule avant le premier token (l'utilisateur parlait encore) :
        // retirer le message utilisateur ORPHELIN pousse en tete de fonction
        // — de l'historique ET de la fenetre live (avec les eventuelles
        // paires outils du tour avorte). Vecu 2026-07-05 : chaque annulation
        // laissait un fragment (« Mm. », « je explique »...) que le modele
        // finissait par RECITER.
        c.history.pop();
        c.messages.truncate(base);
        // Le delta n'a pas ete entendu : le re-raconter au prochain tour.
        c.vu_jusqu_a = curseur_avant;
    }
    // Borner l'historique : seuls les 8 derniers messages partent au rebuild,
    // inutile de garder le reste en RAM sur une session illimitee.
    if c.history.len() > 16 {
        let cut = c.history.len() - 16;
        c.history.drain(..cut);
    }
    // Palier de fenetre : au-dela de 25 messages live (tool-calls compris),
    // rebatir depuis l'historique compact — UN plein prefill assume, au lieu
    // d'un glissement qui casserait le cache A CHAQUE tour.
    if c.messages.len() > 25 {
        rebuild_messages(c);
    }
    Ok(full)
}

/// Attend la fin de lecture, en sondant le barge-in.
fn drain_player(
    c: &Cascade,
    mut interrupted: impl FnMut() -> bool,
) -> bool {
    while !c.player.is_idle() {
        if let Some(p) = &c.pouls {
            // Etat seulement : le NIVEAU par clause pose par speak() reste.
            p.etat.store(3, std::sync::atomic::Ordering::Relaxed);
        }
        if interrupted() {
            c.player.clear();
            if let Some(p) = &c.pouls {
                p.poser(0, 0.0);
            }
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if let Some(p) = &c.pouls {
        p.poser(0, 0.0); // fin de lecture : repos
    }
    false
}

/// Deroule un tour committe : affichage, LLM, lecture, rapport. Utilise par
/// les deux chemins de fin de tour de `talk` (semantique et filet 800 ms).
fn run_turn(
    c: &mut Cascade,
    transcript: &str,
    clock: &mut waly_voice::TurnClock,
    barge: impl Fn() -> bool,
) {
    // Fusion des tours annules : la phrase d'un tour tue avant le premier
    // mot se RECOLLE au tour suivant au lieu d'etre jetee (la parole hachee
    // par le barge-in redevient UNE question). Bornee a ~600 chars.
    let transcript = if c.transcript_en_attente.is_empty() {
        transcript.to_string()
    } else {
        let t = format!("{} {}", c.transcript_en_attente, transcript);
        c.transcript_en_attente.clear();
        t
    };
    let transcript = transcript.trim();
    if !waly_voice::has_speech(transcript) {
        return;
    }
    println!("Toi : {transcript}");
    match respond(c, transcript, clock, &barge) {
        Ok(reply) if reply.trim().is_empty() => {
            // Generation annulee avant le premier token : l'utilisateur
            // parlait encore — sa phrase est GARDEE et se recollera au
            // tour que sa voix (re-injectee dans le VAD) va former.
            println!("  (tour annule: tu parlais encore — phrase gardee)");
            c.transcript_en_attente = waly_core::prompt::compacte(transcript, 600);
        }
        Ok(reply) => {
            println!("Waly: {}", reply.trim());
            if drain_player(c, &barge) {
                println!("  (interrompu)");
            }
            println!("  {}", clock.summary());
        }
        Err(e) => {
            let msg = e.to_string();
            eprintln!("tour rate: {msg}");
            if msg.contains("10054") || msg.contains("injoignable") {
                eprintln!("  (le moteur LLM semble mort - relancer C:\\waly\\engines\\start-waly-voice.ps1)");
            }
        }
    }
}

/// Lit un WAV mono (i16 ou f32) et retourne (echantillons f32, cadence).
fn read_wav_mono(path: &str) -> Result<(Vec<f32>, u32), Box<dyn std::error::Error>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .map(|&s| s as f32 / i16::MAX as f32)
            .collect(),
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
    };
    let mono = if spec.channels <= 1 {
        raw
    } else {
        let ch = spec.channels as usize;
        raw.chunks_exact(ch).map(|fr| fr.iter().sum::<f32>() / ch as f32).collect()
    };
    Ok((mono, spec.sample_rate))
}

/// Bench + ecoute Pocket TTS natif. <voix> : un WAV de reference (clonage),
/// un .safetensors (etat officiel), ou un NOM de voix officielle (cosette,
/// javert...) resolu dans models\pocket-voices\french_24l.
fn cmd_pocket(voice: &str, out: &str, text: &str) -> Result<(), Box<dyn std::error::Error>> {
    let model_dir = std::env::var("WALY_POCKET_DIR")
        .unwrap_or_else(|_| waly_core::chemins::modele("pocket-tts-fr-24l"));

    waly_voice::vad::init_onnxruntime()?;
    let t0 = Instant::now();
    // RTF 0,92 mesure a 4 threads (R-V, fp32 flow+decodeur, 5 pas) : marge
    // fine sous 1,0 -> nombre de threads reglable pour les bancs.
    let threads = std::env::var("WALY_POCKET_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    let mut tts = waly_voice::pocket::PocketNative::new(&model_dir, threads)?;
    println!(
        "Pocket natif charge en {:.2} s ({} Hz, {} threads)",
        t0.elapsed().as_secs_f64(),
        tts.sample_rate(),
        threads
    );

    let t0 = Instant::now();
    if voice.ends_with(".safetensors") {
        tts.set_voice_state(std::path::Path::new(voice))?;
        println!("Etat de voix charge en {:.2} s ({voice})", t0.elapsed().as_secs_f64());
    } else if voice.ends_with(".wav") {
        let (reference, ref_rate) = read_wav_mono(voice)?;
        println!(
            "Reference: {voice} ({:.1} s a {ref_rate} Hz)",
            reference.len() as f32 / ref_rate as f32
        );
        tts.set_voice(&reference, ref_rate)?;
        println!("Voix conditionnee en {:.2} s", t0.elapsed().as_secs_f64());
    } else {
        // Nom de voix officielle.
        let path = waly_core::chemins::modele(&format!("pocket-voices/french_24l/{voice}.safetensors"));
        tts.set_voice_state(std::path::Path::new(&path))?;
        println!("Voix officielle '{voice}' chargee en {:.2} s", t0.elapsed().as_secs_f64());
    }

    let cleaned = waly_voice::clean_for_tts(text);
    let mut samples: Vec<f32> = Vec::new();
    for passe in 1..=2 {
        samples.clear();
        let t0 = Instant::now();
        let mut ttfa: Option<f64> = None;
        tts.synth_stream(&cleaned, &mut |chunk: &[f32]| {
            if !chunk.is_empty() && ttfa.is_none() {
                ttfa = Some(t0.elapsed().as_secs_f64());
            }
            samples.extend_from_slice(chunk);
            true
        })?;
        let dt = t0.elapsed().as_secs_f64();
        let audio_s = samples.len() as f64 / tts.sample_rate() as f64;
        println!(
            "passe {passe}: premier chunk {:.3} s | total {:.3} s pour {:.2} s audio (RTF {:.2})",
            ttfa.unwrap_or(dt),
            dt,
            audio_s,
            dt / audio_s.max(0.001)
        );
    }

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: tts.sample_rate(),
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(out, spec)?;
    for s in &samples {
        writer.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
    }
    writer.finalize()?;
    println!("Ecrit: {out}");

    let player = Player::new()?;
    player.push(&samples, tts.sample_rate());
    while !player.is_idle() {
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(200));
    Ok(())
}

/// Bench + ecoute Piper : <voix> (ex. fr_FR-tom-medium), <sid> (locuteur,
/// 0 par defaut), synthese, ecrit <out.wav> et joue.
fn cmd_piper(
    voice: &str,
    sid: i32,
    out: &str,
    text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let dll = std::env::var("WALY_SHERPA_DLL")
        .unwrap_or_else(|_| waly_core::chemins::lib_sherpa("sherpa-onnx-c-api"));
    let piper_dir = waly_core::chemins::modele(&format!("vits-piper-{voice}"));
    let t0 = Instant::now();
    let mut tts = waly_voice::tts::PiperTts::new(&dll, &piper_dir, voice, 2)?;
    tts.set_speaker(sid);
    println!("Piper {voice} sid {sid} charge en {:.2} s ({} Hz)", t0.elapsed().as_secs_f64(), tts.sample_rate());
    // Meme prosodie que la cascade : humeur detectee sur le texte brut.
    let prosody = waly_voice::prosody::analyze(text);
    println!("humeur: {:?} (vitesse {})", prosody.mood, prosody.speed);
    let cleaned = waly_voice::clean_for_tts(text);
    let t0 = Instant::now();
    let samples = tts.synth(&cleaned, prosody.speed)?;
    println!(
        "synthese {:.3} s pour {:.2} s audio",
        t0.elapsed().as_secs_f64(),
        samples.len() as f32 / tts.sample_rate() as f32
    );
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: tts.sample_rate(),
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(out, spec)?;
    for s in &samples {
        writer.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
    }
    writer.finalize()?;
    println!("Ecrit: {out}");
    let player = Player::new()?;
    player.push(&samples, tts.sample_rate());
    while !player.is_idle() {
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(200));
    Ok(())
}

fn cmd_say(text: &str) -> Result<(), Box<dyn std::error::Error>> {
    let dll = std::env::var("WALY_SHERPA_DLL")
        .unwrap_or_else(|_| waly_core::chemins::lib_sherpa("sherpa-onnx-c-api"));
    let cfg = waly_voice::VoiceConfig::default();
    let piper_dir = waly_core::chemins::modele(&format!("vits-piper-{}", cfg.tts_voice));
    let t0 = Instant::now();
    let mut tts = waly_voice::tts::PiperTts::new(&dll, &piper_dir, &cfg.tts_voice, 2)?;
    tts.set_speaker(cfg.tts_speaker);
    println!("Piper charge en {:.2} s ({} Hz)", t0.elapsed().as_secs_f64(), tts.sample_rate());
    let t0 = Instant::now();
    let cleaned = waly_voice::clean_for_tts(text);
    let samples = tts.synth(&cleaned, 1.0)?;
    println!(
        "synthese {:.3} s pour {:.2} s audio",
        t0.elapsed().as_secs_f64(),
        samples.len() as f32 / tts.sample_rate() as f32
    );
    let player = Player::new()?;
    player.push(&samples, tts.sample_rate());
    while !player.is_idle() {
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(200)); // vider le tampon materiel
    Ok(())
}

/// Un tour complet depuis un WAV (sans micro) : mesure le budget de bout en
/// bout comme si la fin de parole venait d'arriver. Simule l'endpointing
/// semantique de `talk` : transcript complet -> silence court + STT recouvert.
fn cmd_turn(wav_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut c = cascade_init()?;
    let mut reader = hound::WavReader::open(wav_path)?;
    let spec = reader.spec();
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .map(|&s| s as f32 / i16::MAX as f32)
        .collect();

    let cfg = waly_voice::VoiceConfig::default();
    let mut clock = waly_voice::TurnClock::new(Duration::from_millis(cfg.end_of_turn_silence_ms));
    clock.mark_speech_end();
    let t0 = Instant::now();
    let transcript = c.stt.transcribe(&samples, spec.sample_rate as i32)?;
    // Simuler l'endpointing semantique de `talk` : le STT speculatif est paye
    // PENDANT le silence dans les trois chemins (transcript reutilise), seule
    // l'echeance change selon le verdict.
    {
        use waly_voice::endpoint::Completeness::*;
        let stt_overlap = Duration::from_millis(cfg.endpoint_fast_ms) + t0.elapsed();
        let (silence, label) = match waly_voice::endpoint::assess(&transcript) {
            Complete => (stt_overlap, "complet, commit anticipe"),
            LikelyComplete => (
                stt_overlap.max(Duration::from_millis(cfg.endpoint_period_ms)),
                "point final, echeance intermediaire",
            ),
            Incomplete => (
                Duration::from_millis(cfg.end_of_turn_silence_ms),
                "en suspens, filet long",
            ),
        };
        clock.end_of_turn_silence = silence;
        clock.mark_speech_end();
        println!("(endpoint semantique: {label})");
    }
    clock.mark_stt_final();
    println!("Transcript: {transcript}");
    let reply = respond(&mut c, &transcript, &mut clock, || false)?;
    println!("Waly: {reply}");
    drain_player(&c, || false);
    println!("{}", clock.summary());
    Ok(())
}

/// Tour de DEBUG sans micro ni STT : le transcript est donne en argument,
/// tout le reste (streaming, outils dont regarder en mode appel, prosodie,
/// TTS) est le chemin reel de `talk`. Sert a reproduire un tour parle
/// exact au banc — ne notifiez pas Parakeet.
fn cmd_text(transcript: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut c = cascade_init()?;
    let cfg = waly_voice::VoiceConfig::default();
    let mut clock = waly_voice::TurnClock::new(Duration::from_millis(cfg.end_of_turn_silence_ms));
    clock.mark_speech_end();
    clock.mark_stt_final();
    println!("Transcript (fourni): {transcript}");
    let reply = respond(&mut c, transcript, &mut clock, || false)?;
    println!("Waly: {reply}");
    drain_player(&c, || false);
    println!("{}", clock.summary());
    Ok(())
}

/// Boucle de conversation complete : VAD -> Parakeet -> LLM -> Piper,
/// barge-in par VAD Silero dedie + garde anti-echo (AEC v1, aec.rs) pour
/// l'usage sans casque.
/// Fin de tour semantique (R1.5) : STT speculatif a `endpoint_fast_ms` de
/// silence, commit immediat si le transcript est complet, filet 800 ms sinon.
fn cmd_talk(secs: u64) -> Result<(), Box<dyn std::error::Error>> {
    waly_voice::vad::init_onnxruntime()?;
    let c = cascade_init()?;
    talk_loop(secs, c, Vec::new(), None)
}

/// Signal d'eveil au desktop (POST /eveil sur le port d'appel, best effort) :
/// l'eclipse « Souffle » repond en < 500 ms pendant que la cascade charge.
/// Sans desktop (WALY_APPEL_PORT absent), silencieux.
fn signale_eveil() {
    poste_simple("/eveil");
}

/// Retrait d'un eveil OPTIMISTE refute par la confirmation STT (terrain
/// 2026-07-21 : le film sature le score wake a 0,98+ — le seuil ne suffit
/// plus, c'est le STT qui tranche). L'UI referme la page voix ouverte.
fn signale_eveil_annule() {
    poste_simple("/eveil-annule");
}

fn poste_simple(chemin: &str) {
    let Some(port) = appel_port() else { return };
    let req = format!(
        "POST {chemin} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let _ = std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(300),
    )
    .and_then(|mut s| {
        use std::io::Write;
        s.write_all(req.as_bytes())
    });
}

/// La transcription contient-elle un appel a « Waly » ? Parakeet ecrit
/// selon la prise : Waly, Wali, Wally, Ouali, parfois « valise »-frontiere —
/// on cherche la syllabe d'attaque + la finale en i/y, tolerants.
fn transcript_appelle_waly(t: &str) -> bool {
    let t = t.to_lowercase();
    ["waly", "wali", "wally", "ouali", "ouély", "walie", "wailly"]
        .iter()
        .any(|m| t.contains(m))
}

/// Un « Waly » NU se transcrit en RIEN (limite Parakeet gravee : mot isole
/// < 1 s avale, pistes fermees avec mesures) — la transcription vide ne
/// suffit donc pas a refuter. Departage par PROFIL D'ENERGIE : un appel
/// isole est UNE rafale courte entouree de calme ; la musique/parole de
/// film qui sature le score est de l'energie continue.
fn ressemble_appel_isole(seg: &[f32]) -> bool {
    let win = TARGET_RATE as usize * 30 / 1000;
    if seg.len() < win * 10 {
        return false;
    }
    let rms: Vec<f32> = seg
        .chunks(win)
        .map(|c| (c.iter().map(|v| v * v).sum::<f32>() / c.len().max(1) as f32).sqrt())
        .collect();
    let max = rms.iter().cloned().fold(0.0f32, f32::max);
    if max < 0.01 {
        return false; // rien d'audible : pas un appel
    }
    let th = max * 0.25;
    let actives = rms.iter().filter(|r| **r > th).count();
    let frac = actives as f32 / rms.len() as f32;
    // Un mot isole occupe 4-45 % des fenetres (0,15-1,6 s sur ~3,5 s) ;
    // au-dela, c'est un fond sonore continu.
    (0.04..=0.45).contains(&frac)
}

/// Decimation MOYENNEE pour la veille (terrain 2026-07-21) :
/// `resample_linear` decime SANS filtre anti-repliement (48 k -> 16 k =
/// point-sampling) — le spectre replie produisait des scores wake satures
/// (0,99-1,00) sur de la simple parole ambiante, un domaine que le
/// classifieur n'a jamais vu (le banc travaillait sur des WAV 16 k
/// propres). Moyenner chaque bloc = filtre boxcar + decimation : grossier
/// mais suffisant pour retrouver le domaine du banc. Ratio non entier ->
/// linear (mieux que rien, mics 44,1 k rares ici).
fn decimate_moyenne(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return input.to_vec();
    }
    if from % to != 0 {
        return resample_linear(input, from, to);
    }
    let k = (from / to) as usize;
    input.chunks_exact(k).map(|b| b.iter().sum::<f32>() / k as f32).collect()
}

/// Mode veille (R6b « L'Eveil ») : ecoute LEGERE du wake word « Waly » —
/// capture + pipeline openWakeWord seuls (~50 Mo, ~1,4 % d'un coeur,
/// verdicts au banc lab/eveil-banc). Ni Parakeet, ni Pocket, ni FLM tant
/// que le nom n'a pas ete dit. Au reveil : /eveil au desktop, cascade
/// chargee PENDANT que le flux micro continue (la phrase dite juste apres
/// « Waly » part en rattrapage), conversation directe, puis RETOUR en
/// veille apres WALY_VEILLE_TIMEOUT s de silence (defaut 300).
/// L'audio de veille vit en RAM (sink + anneau), n'est JAMAIS ecrit.
fn cmd_veille() -> Result<(), Box<dyn std::error::Error>> {
    use waly_voice::wake::{WakeDetector, CHUNK};
    waly_voice::vad::init_onnxruntime()?;
    let dir = std::env::var("WALY_WAKE_DIR")
        .unwrap_or_else(|_| waly_core::chemins::modele("openwakeword"));
    let seuil: f32 = std::env::var("WALY_WAKE_SEUIL")
        .ok()
        .and_then(|v| v.parse().ok())
        // 0,9 : point de fonctionnement du banc (FAR inter-moteur ~0, FRR
        // rattrape par l'enrollment) — a ajuster au terrain.
        .unwrap_or(0.9);
    let timeout = Duration::from_secs(
        std::env::var("WALY_VEILLE_TIMEOUT").ok().and_then(|v| v.parse().ok()).unwrap_or(300),
    );
    let cfg = waly_voice::VoiceConfig::default();
    loop {
        let mut wake = WakeDetector::new(std::path::Path::new(&dir), seuil)?;
        let (device, config) = open_input(cfg.input_device_name.as_deref())?;
        let native_rate = config.sample_rate().0;
        let sink = Arc::new(Mutex::new(Vec::<f32>::new()));
        let input_dead = Arc::new(AtomicBool::new(false));
        let stream =
            start_capture(&device, &config, Arc::clone(&sink), Arc::clone(&input_dead))?;
        println!("waly-voice veille : dis « Waly » (seuil {seuil})");
        journal("veille: ecoute legere demarree");
        let mut pend: Vec<f32> = Vec::new();
        // Mini-anneau 16 kHz des 3 dernieres secondes : au reveil, c'est LA
        // matiere de la confirmation STT (le « Waly » lui-meme y est).
        let mut anneau: Vec<f32> = Vec::new();
        let cap_anneau = TARGET_RATE as usize * 3;
        let mut score_eveil = 0.0f32;
        'ecoute: loop {
            std::thread::sleep(Duration::from_millis(30));
            if input_dead.load(Ordering::Relaxed) {
                return Err("le flux micro est mort en veille — relancer waly-voice".into());
            }
            let chunk: Vec<f32> = std::mem::take(&mut *lock_audio(&sink));
            if chunk.is_empty() {
                continue;
            }
            let c16 = decimate_moyenne(&chunk, native_rate, TARGET_RATE);
            anneau.extend_from_slice(&c16);
            if anneau.len() > cap_anneau {
                let cut = anneau.len() - cap_anneau;
                anneau.drain(..cut);
            }
            pend.extend(c16);
            // Borne : si la boucle prend du retard, on garde 2 s au plus.
            if pend.len() > TARGET_RATE as usize * 2 {
                let cut = pend.len() - TARGET_RATE as usize * 2;
                pend.drain(..cut);
            }
            while pend.len() >= CHUNK {
                let step: Vec<f32> = pend.drain(..CHUNK).collect();
                match wake.push(&step) {
                    Ok(Some(score)) => {
                        score_eveil = score;
                        break 'ecoute;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("wake: {e} (pas ignore)");
                    }
                }
            }
        }
        println!("EVEIL (score {score_eveil:.2}) — chargement de la cascade...");
        journal(&format!("eveil: score {score_eveil:.2}"));
        signale_eveil();
        drop(wake); // libere les sessions wake avant de charger la cascade
        // Le flux micro RESTE OUVERT pendant le chargement : tout ce qui se
        // dit s'accumule dans sink et part en rattrapage — la phrase apres
        // « Waly » n'est jamais perdue (decision GATE 3).
        // ⚠ Un echec de cascade (vecu 2026-07-21 : RAM exsangue, Pocket ne
        // charge pas) ne doit PAS tuer le processus : on journalise et on
        // RETOURNE en ecoute legere — l'eveil suivant retentera.
        match cascade_init() {
            Ok(c) => {
                let brut: Vec<f32> = std::mem::take(&mut *lock_audio(&sink));
                let rattrapage = resample_linear(&brut, native_rate, TARGET_RATE);
                drop(stream); // talk_loop rouvre sa propre capture (quelques ms)
                // CONFIRMATION STT (terrain 2026-07-21) : le film sature le
                // score wake (0,98+) — Parakeet, charge de toute façon,
                // tranche : « Waly » doit s'entendre dans l'anneau pre-eveil
                // (+ le tout debut du rattrapage). Refute -> eveil annule
                // cote UI, retour a l'ecoute, cascade rendue.
                let mut segment = std::mem::take(&mut anneau);
                segment.extend(rattrapage.iter().take(TARGET_RATE as usize).copied());
                let entendu = match c.stt.transcribe(&segment, TARGET_RATE as i32) {
                    Ok(t) => t,
                    Err(e) => {
                        journal(&format!("eveil: STT confirmation en erreur ({e})"));
                        String::new()
                    }
                };
                // Verdict a trois voies : « waly » entendu -> confirme ;
                // AUTRE parole entendue -> refute (film/dialogue) ; RIEN
                // entendu -> profil d'energie (le « Waly » nu est avale par
                // Parakeet — limite gravee).
                let confirme = if transcript_appelle_waly(&entendu) {
                    journal(&format!("eveil confirme (STT: « {entendu} »)"));
                    true
                } else if !entendu.trim().is_empty() {
                    journal(&format!("eveil REFUTE par le STT (« {entendu} »)"));
                    false
                } else if ressemble_appel_isole(&segment) {
                    journal("eveil confirme (STT muet, profil d'appel isole)");
                    true
                } else {
                    journal("eveil REFUTE (STT muet, fond sonore continu)");
                    false
                };
                if !confirme {
                    println!("eveil refute");
                    signale_eveil_annule();
                    continue;
                }
                if let Err(e) = talk_loop(0, c, rattrapage, Some(timeout)) {
                    journal(&format!("veille: conversation interrompue: {e}"));
                    eprintln!("veille: conversation interrompue: {e}");
                }
            }
            Err(e) => {
                journal(&format!("veille: cascade impossible ({e}) — retour ecoute"));
                eprintln!("veille: cascade impossible: {e}");
                std::thread::sleep(Duration::from_secs(5));
            }
        }
        // Retour : la cascade (si elle a vecu) est liberee, on repart en
        // ecoute legere (le processus garde ~la RAM de veille).
    }
}

/// Boucle de conversation complete (ex-corps de cmd_talk). `rattrapage` :
/// audio 16 kHz capture PENDANT le chargement de la cascade (veille R6b),
/// injecte comme s'il venait d'etre parle — la phrase dite juste apres
/// « Waly » n'est jamais perdue. `veille_timeout` : au-dela de ce silence
/// total (rien dit, rien joue), retour Ok(()) — cmd_veille reprend l'ecoute
/// legere et la cascade est liberee (budget RAM de veille).
fn talk_loop(
    secs: u64,
    mut c: Cascade,
    rattrapage: Vec<f32>,
    veille_timeout: Option<Duration>,
) -> Result<(), Box<dyn std::error::Error>> {
    use waly_voice::endpoint::{assess, AdaptiveEndpointer, EndpointEdge};
    use waly_voice::segment::FRAME;
    use waly_voice::vad::{self, SileroVad};

    vad::init_onnxruntime()?;
    let model = std::env::var("WALY_SILERO_PATH")
        .unwrap_or_else(|_| waly_core::chemins::modele("silero_vad.onnx"));
    let mut silero = SileroVad::new(&model)?;
    let cfg = waly_voice::VoiceConfig::default();
    let mut ep = AdaptiveEndpointer::new(
        cfg.endpoint_fast_ms,
        cfg.endpoint_period_ms,
        cfg.end_of_turn_silence_ms,
    );

    let (device, config) = open_input(cfg.input_device_name.as_deref())?;
    let native_rate = config.sample_rate().0;
    let sink = Arc::new(Mutex::new(Vec::<f32>::new()));
    let input_dead = Arc::new(AtomicBool::new(false));
    let _stream = start_capture(&device, &config, Arc::clone(&sink), Arc::clone(&input_dead))?;

    // Detection d'interruption pendant que Waly genere/parle : un SECOND VAD
    // Silero dedie au flux micro. Les seuils RMS (0,05 puis 0,04/0,02) ne
    // declenchaient pas en voix conversationnelle — le NIVEAU varie trop
    // selon la voix/distance/micro, la PROBABILITE de parole non.
    // Pendant la lecture : seuil plus exigeant + plancher d'energie (sans
    // AEC, l'echo haut-parleur est de la parole pour Silero — OK au casque).
    // Les echantillons consommes sont GARDES (2 s glissantes) et reinjectes
    // dans le VAD principal apres interruption : la phrase qui interrompt
    // n'est pas perdue. `(barge: p max ...)` par tour = diagnostic terrain.
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    // Garde anti-echo (AEC v1) : permet l'usage SANS casque. Le lecteur
    // fournit ce qui sort reellement (tap) ; toute « voix » au micro dont
    // l'enveloppe est correlee au son joue est un echo, pas l'utilisateur.
    let gate = Rc::new(RefCell::new(waly_voice::aec::EchoGate::new()));
    let out_rate = c.player.out_rate;
    let tap = Arc::clone(&c.player.tap);
    let barge_win = (native_rate as usize * 30) / 1000;
    let stolen: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let barged = Rc::new(Cell::new(false));
    let barge_pmax = Rc::new(Cell::new(0.0f32));
    let sink_for_barge = Arc::clone(&sink);
    let stolen_for_barge = Arc::clone(&stolen);
    let out_queue = Arc::clone(&c.player.queue);
    let barged_in = Rc::clone(&barged);
    let pmax_in = Rc::clone(&barge_pmax);
    // Etat du detecteur PARTAGE avec la boucle principale : il doit etre
    // remis a zero a chaque fin de tour, sinon un compteur chaud ou l'etat
    // RNN de Silero survivent et le tour suivant s'auto-annule a la
    // premiere sonde (vecu : reponses vides en 0,03 s apres une interruption).
    let barge_vad = Rc::new(RefCell::new(SileroVad::new(&model)?));
    let barge_pend = Rc::new(RefCell::new(Vec::<f32>::new()));
    let hot = Rc::new(Cell::new(0u32));
    let vad_in = Rc::clone(&barge_vad);
    let pend_in = Rc::clone(&barge_pend);
    let hot_in = Rc::clone(&hot);
    let tap_in = Arc::clone(&tap);
    let gate_in = Rc::clone(&gate);
    let barge = move || {
        let chunk: Vec<f32> = std::mem::take(&mut *lock_audio(&sink_for_barge));
        if chunk.is_empty() {
            return barged_in.get();
        }
        {
            let mut kept = lock_audio(&stolen_for_barge);
            kept.extend_from_slice(&chunk);
            let cap = native_rate as usize * 2;
            if kept.len() > cap {
                let cut = kept.len() - cap;
                kept.drain(..cut);
            }
        }
        let loud = max_windowed_rms(&chunk, barge_win);
        let chunk16k = resample_linear(&chunk, native_rate, TARGET_RATE);
        // Garde anti-echo : nourrir (son joue + micro) puis interroger.
        {
            let played: Vec<f32> = std::mem::take(&mut *lock_audio(&tap_in));
            let mut g = gate_in.borrow_mut();
            if !played.is_empty() {
                g.push_played(&resample_linear(&played, out_rate, TARGET_RATE));
            }
            g.observe_mic(&chunk16k);
        }
        let echo = gate_in.borrow().mic_is_echo();
        let mut pend = pend_in.borrow_mut();
        pend.extend(chunk16k);
        let mut vad = vad_in.borrow_mut();
        let playing = !lock_audio(&out_queue).is_empty();
        // Lecture : 3 trames (~100 ms) a p>0,7 + un peu d'energie ;
        // TTFT (rien ne joue) : 2 trames a p>0,5. Jamais sur un echo.
        let (need, thr) = if playing { (3, 0.7f32) } else { (2, 0.5f32) };
        while pend.len() >= FRAME {
            let frame: Vec<f32> = pend.drain(..FRAME).collect();
            let Ok(p) = vad.process(&frame) else { continue };
            pmax_in.set(pmax_in.get().max(p));
            if p > thr && (!playing || loud > 0.015) && !echo {
                hot_in.set(hot_in.get() + 1);
                if hot_in.get() >= need {
                    barged_in.set(true);
                }
            } else {
                hot_in.set(0);
            }
        }
        barged_in.get()
    };

    if secs == 0 {
        println!("waly-voice talk : conversation illimitee (Ctrl+C pour sortir)...");
    } else {
        println!("waly-voice talk : parle ({} s max, Ctrl+C pour sortir)...", secs);
    }
    let start = Instant::now();
    // Rattrapage veille : les echantillons captes pendant le chargement
    // partent en tete de file — le VAD/endpointer les traite comme s'ils
    // venaient d'etre dits.
    let mut pend16k: Vec<f32> = rattrapage;
    let mut derniere_activite = Instant::now();
    let mut preroll: Vec<f32> = Vec::new();
    let mut utterance: Vec<f32> = Vec::new();
    // Trames vraiment parlees (p > 0,5) : garde anti-bruit — un souffle
    // declenche le VAD puis Parakeet hallucine (de l'anglais, vecu).
    let mut speech_frames: u32 = 0;
    const MIN_SPEECH_FRAMES: u32 = 12; // ~0,38 s de parole effective
    // Transcript speculatif en attente + speech_frames au moment du STT :
    // si rien de nouveau n'est parle d'ici le filet 800 ms, on le reutilise.
    let mut spec: Option<(String, u32)> = None;
    let mut clock = waly_voice::TurnClock::new(Duration::from_millis(cfg.end_of_turn_silence_ms));

    // Erreurs transitoires du VAD principal : on saute la trame au lieu de
    // tuer la session, mais on ne spamme pas la console.
    let mut vad_errs: u32 = 0;
    // Rappels : verifies quand Waly est au repos (jamais couper un tour ni
    // sa propre parole). L'annonce passe par le lecteur comme une reponse.
    let mut last_reminder_check = Instant::now();
    // Moments proactifs (R4.5 ch. 3) : au repos, dire ce que la perception a
    // remarque de FORT (kind 'moment' UNIQUEMENT — retours, visiteurs,
    // scenes decrites), cooldown 3 min, jamais de backlog — le silence
    // domine (lecon ProAssist). Curseur independant de celui du delta.
    let mut moment_dit = waly_core::store::visual_memory_last_id(
        &c.conn,
        waly_core::store::MAIN_SESSION,
    )
    .unwrap_or(0);
    let mut dernier_moment_dit = Instant::now();

    while secs == 0 || start.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_millis(30));
        if input_dead.load(Ordering::Relaxed) {
            return Err("le flux micro est mort (device debranche ou pris par une autre \
                        application) — relancer waly-voice"
                .into());
        }
        // Retour en veille (R6b) : la reference d'activite est LA PAROLE DE
        // WALY (lecteur non vide), pas la parole ambiante — vecu terrain
        // 2026-07-21 : un film dans les haut-parleurs maintient ep.speaking()
        // indefiniment (tours sans cesse annules) et la conversation ne
        // redormait JAMAIS. Si Waly n'a rien dit depuis le timeout, la
        // conversation ne vit pas : retour a l'ecoute legere (cascade
        // liberee -> budget RAM de veille tenu).
        if !c.player.is_idle() {
            derniere_activite = Instant::now();
        }
        if let Some(tmax) = veille_timeout {
            if derniere_activite.elapsed() > tmax {
                println!(
                    "veille : Waly muet depuis {} s — retour a l'ecoute legere",
                    tmax.as_secs()
                );
                journal("veille: retour ecoute legere");
                return Ok(());
            }
        }
        // Rappels echus : uniquement au repos (rien en cours, lecteur vide)
        // pour ne pas couper une phrase ni la parole de Waly.
        if last_reminder_check.elapsed() > Duration::from_secs(4)
            && !ep.speaking()
            && utterance.is_empty()
            && spec.is_none()
            && c.player.is_idle()
        {
            last_reminder_check = Instant::now();
            match waly_core::store::due_reminders(&c.conn) {
                Ok(due) => {
                    for rem in due {
                        let phrase = match rem.message.as_deref() {
                            Some(m) if !m.is_empty() => {
                                format!("Petit rappel : {}. {m}", rem.title)
                            }
                            _ => format!("Petit rappel : {}.", rem.title),
                        };
                        println!("  [rappel] {}", rem.title);
                        let text = waly_voice::clean_for_tts(&phrase);
                        if let Ok(samples) = c.tts.synth(&text, 1.0) {
                            c.player.push(&samples, c.tts.sample_rate());
                        }
                        // Coherence : Waly sait qu'il l'a dit (suivi possible)
                        // — fenetre LIVE comprise (append-only, trou pre-R4.5
                        // corrige : seule history etait poussee).
                        c.history.push(("assistant".into(), phrase.clone()));
                        c.messages.push(waly_core::llm::Msg::Assistant(phrase.clone()));
                        waly_core::store::append_message_in(&c.conn, session(), "assistant", &phrase).ok();
                    }
                }
                Err(e) => eprintln!("rappels: {e}"),
            }
            // Moments proactifs : memes conditions de repos que les rappels.
            if camera() && dernier_moment_dit.elapsed() > Duration::from_secs(180)
            {
                if let Ok(entries) = waly_core::store::visual_memory_after(
                    &c.conn,
                    waly_core::store::MAIN_SESSION,
                    moment_dit,
                    5,
                ) {
                    if let Some(dernier) = entries.last().map(|(id, _, _, _)| *id) {
                        // Le PLUS RECENT moment seulement (pas de backlog) ;
                        // le curseur saute tout ce qui est plus vieux.
                        if let Some((_, _, _, contenu)) =
                            entries.iter().rev().find(|(_, _, k, _)| k == "moment")
                        {
                            let phrase = format!("Au fait — {contenu}.");
                            println!("  [moment] {contenu}");
                            journal(&format!("moment dit: {contenu}"));
                            let text = waly_voice::clean_for_tts(&phrase);
                            if let Ok(samples) = c.tts.synth(&text, 1.0) {
                                c.player.push(&samples, c.tts.sample_rate());
                            }
                            c.history.push(("assistant".into(), phrase.clone()));
                            // Fenetre LIVE aussi : append-only, le modele doit
                            // savoir qu'il l'a dit (trou des rappels corrige
                            // au meme endroit ci-dessous).
                            c.messages
                                .push(waly_core::llm::Msg::Assistant(phrase.clone()));
                            waly_core::store::append_message_in(&c.conn, session(), "assistant", &phrase)
                                .ok();
                            dernier_moment_dit = Instant::now();
                        }
                        moment_dit = dernier;
                    }
                }
            }
        }
        // Mute micro (R5) : Waly lit le device brut (le mute systeme ne
        // l'arrete pas) — quand le pop-up coupe, on JETTE le son du micro et on
        // remet tout a plat (pas de demi-phrase qui se termine au demutage).
        if c.muted.load(Ordering::Relaxed) {
            lock_audio(&sink).clear();
            lock_audio(&tap).clear();
            utterance.clear();
            pend16k.clear();
            speech_frames = 0;
            if let Some(p) = &c.pouls {
                if c.player.is_idle() {
                    p.poser(0, 0.0);
                }
            }
            continue;
        }
        let chunk: Vec<f32> = std::mem::take(&mut *lock_audio(&sink));
        let chunk16k = resample_linear(&chunk, native_rate, TARGET_RATE);
        // Pouls (ch. 4) : ecoute quand Michee parle (niveau = micro), repos
        // sinon — les etats reflexion/parole sont poses par le tour.
        if let Some(p) = &c.pouls {
            if ep.speaking() {
                p.poser(1, (rms(&chunk16k) * 6.0).min(1.0));
            } else if c.player.is_idle() {
                p.poser(0, 0.0);
            }
        }
        // Garde anti-echo cote tour : la queue d'echo apres la lecture ne
        // doit pas ouvrir un faux tour utilisateur.
        {
            let played: Vec<f32> = std::mem::take(&mut *lock_audio(&tap));
            let mut g = gate.borrow_mut();
            if !played.is_empty() {
                g.push_played(&resample_linear(&played, out_rate, TARGET_RATE));
            }
            g.observe_mic(&chunk16k);
        }
        pend16k.extend(chunk16k);

        while pend16k.len() >= FRAME {
            let frame: Vec<f32> = pend16k.drain(..FRAME).collect();
            // Une erreur ONNX transitoire ne doit pas terminer le processus
            // en pleine conversation (la closure de barge fait pareil).
            let prob = match silero.process(&frame) {
                Ok(p) => p,
                Err(e) => {
                    vad_errs += 1;
                    if vad_errs <= 3 {
                        eprintln!("vad principal: {e} (trame ignoree)");
                    }
                    continue;
                }
            };
            let edge = ep.push(prob);
            if ep.speaking() || edge == EndpointEdge::Ended {
                utterance.extend_from_slice(&frame);
                // Borne de securite : un monologue sans pause de 800 ms ne
                // doit pas faire croitre le buffer sans limite. On garde la
                // fin (Parakeet transcrira les ~60 dernieres secondes).
                let cap = TARGET_RATE as usize * 60;
                if utterance.len() > cap {
                    let cut = utterance.len() - cap;
                    utterance.drain(..cut);
                }
                if prob > 0.5 {
                    speech_frames += 1;
                }
            } else {
                preroll.extend_from_slice(&frame);
                let cap = TARGET_RATE as usize / 2; // 500 ms de pre-roll
                if preroll.len() > cap {
                    let cut = preroll.len() - cap;
                    preroll.drain(..cut);
                }
            }
            match edge {
                EndpointEdge::Started => {
                    // Echo de queue (haut-parleurs sans casque) : pas un tour.
                    if gate.borrow().mic_is_echo() {
                        ep = AdaptiveEndpointer::new(
                            cfg.endpoint_fast_ms,
                            cfg.endpoint_period_ms,
                            cfg.end_of_turn_silence_ms,
                        );
                        silero.reset();
                        utterance.clear();
                        speech_frames = 0;
                        continue;
                    }
                    if !c.player.is_idle() {
                        c.player.clear(); // barge-in pendant la lecture
                        println!("  (barge-in)");
                    }
                    clock = waly_voice::TurnClock::new(Duration::from_millis(
                        cfg.end_of_turn_silence_ms,
                    ));
                    spec = None;
                    let mut u = std::mem::take(&mut preroll);
                    u.extend_from_slice(&frame);
                    utterance = u;
                }
                // `endpoint_fast_ms` de silence : STT speculatif. `?`/`!` ->
                // commit immediat (STT paye PENDANT le silence) ; point final
                // -> echeance raccourcie a `endpoint_period_ms` ; en suspens
                // -> filet long. Le transcript est garde pour reutilisation.
                EndpointEdge::Speculate => {
                    if speech_frames >= MIN_SPEECH_FRAMES {
                        let t0 = Instant::now();
                        match c.stt.transcribe(&utterance, TARGET_RATE as i32) {
                            Ok(text) => {
                                // L'utilisateur a-t-il repris la parole
                                // PENDANT le STT (~0,2 s) ? Alors pas de
                                // commit : on lui rend ses echantillons.
                                let fresh: Vec<f32> =
                                    std::mem::take(&mut *lock_audio(&sink));
                                let resumed =
                                    max_windowed_rms(&fresh, barge_win) > 0.02;
                                pend16k.extend(resample_linear(
                                    &fresh,
                                    native_rate,
                                    TARGET_RATE,
                                ));
                                // Derive hors-langue Parakeet : jamais de
                                // commit anticipe sur un transcript suspect.
                                let halluc = waly_voice::sanitize::looks_offlang_hallucination(
                                    &text,
                                    speech_frames as f32 * 0.032,
                                );
                                let verdict = assess(&text);
                                if resumed || halluc {
                                    if halluc {
                                        // Log AVEC le texte : sans lui, un faux
                                        // positif (vrai francais rejete) est
                                        // indiagnosticable (vecu 2026-07-05).
                                        println!("  (ep: transcript hors-langue suspect, pas de commit: {text})");
                                    } else {
                                        println!("  (ep: reprise pendant le STT)");
                                    }
                                    spec = Some((text, speech_frames));
                                } else if ep.commit_on(verdict) {
                                    clock.end_of_turn_silence =
                                        Duration::from_millis(cfg.endpoint_fast_ms) + t0.elapsed();
                                    clock.mark_speech_end();
                                    clock.mark_stt_final();
                                    utterance.clear();
                                    speech_frames = 0;
                                    spec = None;
                                    run_turn(&mut c, &text, &mut clock, &barge);
                                    silero.reset();
                                    pend16k.clear();
                                    let _ = std::mem::take(&mut *lock_audio(&sink));
                                    let kept = std::mem::take(&mut *lock_audio(&stolen));
                                    if barged.replace(false) {
                                        // La voix qui a interrompu redevient
                                        // l'entree du VAD : tour suivant.
                                        pend16k.extend(resample_linear(
                                            &kept,
                                            native_rate,
                                            TARGET_RATE,
                                        ));
                                    }
                                    if barge_pmax.get() > 0.0 {
                                        println!(
                                            "  (barge: p max {:.2})",
                                            barge_pmax.get()
                                        );
                                        barge_pmax.set(0.0);
                                    }
                                    hot.set(0);
                                    barge_vad.borrow_mut().reset();
                                    barge_pend.borrow_mut().clear();
                                } else {
                                    println!(
                                        "  (ep: {} -> fin a {} ms)",
                                        match verdict {
                                            waly_voice::endpoint::Completeness::LikelyComplete =>
                                                "point final",
                                            _ => "en suspens",
                                        },
                                        match verdict {
                                            waly_voice::endpoint::Completeness::LikelyComplete =>
                                                cfg.endpoint_period_ms,
                                            _ => cfg.end_of_turn_silence_ms,
                                        }
                                    );
                                    spec = Some((text, speech_frames));
                                }
                            }
                            Err(e) => eprintln!("stt speculatif rate: {e}"),
                        }
                    }
                }
                EndpointEdge::Ended => {
                    // Silence reellement attendu (500 ms point final / 800 filet).
                    clock.end_of_turn_silence =
                        Duration::from_millis(ep.last_end_silence_ms());
                    clock.mark_speech_end();
                    let samples = std::mem::take(&mut utterance);
                    let frames = std::mem::take(&mut speech_frames);
                    let speculated = spec.take();
                    if frames < MIN_SPEECH_FRAMES {
                        silero.reset();
                        continue; // souffle/bruit : pas un enonce
                    }
                    let transcript = match speculated {
                        // Rien de parle depuis le STT speculatif : reutilise.
                        Some((text, f)) if f == frames => text,
                        // Un STT final qui rate abandonne CE tour, pas la
                        // session (meme douceur que le STT speculatif).
                        _ => match c.stt.transcribe(&samples, TARGET_RATE as i32) {
                            Ok(text) => text,
                            Err(e) => {
                                eprintln!("stt final rate: {e} (tour abandonne)");
                                silero.reset();
                                continue;
                            }
                        },
                    };
                    clock.mark_stt_final();
                    // Derive hors-langue Parakeet sur enonce court : du
                    // bruit, pas un tour.
                    if waly_voice::sanitize::looks_offlang_hallucination(
                        &transcript,
                        frames as f32 * 0.032,
                    ) {
                        println!("  (transcript hors-langue suspect ignore: {transcript})");
                        silero.reset();
                        continue;
                    }
                    run_turn(&mut c, &transcript, &mut clock, &barge);
                    silero.reset();
                    pend16k.clear();
                    let _ = std::mem::take(&mut *lock_audio(&sink));
                    let kept = std::mem::take(&mut *lock_audio(&stolen));
                    if barged.replace(false) {
                        // La voix qui a interrompu redevient l'entree du VAD.
                        pend16k.extend(resample_linear(&kept, native_rate, TARGET_RATE));
                    }
                    if barge_pmax.get() > 0.0 {
                        println!("  (barge: p max {:.2})", barge_pmax.get());
                        barge_pmax.set(0.0);
                    }
                    hot.set(0);
                    barge_vad.borrow_mut().reset();
                    barge_pend.borrow_mut().clear();
                }
                EndpointEdge::None => {}
            }
        }
    }
    Ok(())
}

/// Pic de RMS par fenetres de `win` echantillons : detecte une prise de
/// parole breve dans un tampon qui contient surtout du silence — la ou une
/// RMS moyenne sur tout le tampon la dilue (retour terrain 2026-07-04 :
/// barge-in pas immediat).
fn max_windowed_rms(samples: &[f32], win: usize) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    samples
        .chunks(win.max(1))
        .map(|w| (w.iter().map(|s| s * s).sum::<f32>() / w.len() as f32).sqrt())
        .fold(0.0, f32::max)
}

/// Reechantillonnage lineaire (suffisant pour de la voix vers 16 kHz).
fn resample_linear(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = (input.len() as f64 / ratio).floor() as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = input[idx];
            let b = *input.get(idx + 1).unwrap_or(&a);
            a + (b - a) * frac
        })
        .collect()
}

