//! Génération du corpus « Waly » (dev-time) avec les moteurs TTS de R-V :
//! Pocket TTS (graines LIBRES → chaque prise diffère) sur 4 timbres
//! (fabien, développeuse, + clones des timbres Piper pierre/jessica via le
//! clonage par WAV de référence) et Piper direct (pierre/jessica, vitesses
//! variées). Écrit des WAV 16 kHz mono 16-bit, silence rogné, organisés
//! par voix — le split train/validation se fait PAR VOIX (généralisation
//! à des timbres jamais vus).

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use waly_voice::pocket::{resample_sinc, PocketNative};
use waly_voice::tts::PiperTts;

use crate::oww::SR;

pub const TEXTES_POS: &[&str] = &["Waly.", "Waly !", "Waly ?"];

/// Porteuses : le mot arrive APRÈS de la parole (le cas réel « dis Waly ») —
/// le mot doit rester en FIN de prise (le clip d'entraînement place la fin
/// de la prise près de la fin de la fenêtre).
pub const TEXTES_POS_CONTEXTE: &[&str] =
    &["Dis Waly.", "Hé Waly.", "Ok Waly.", "Alors Waly.", "Bonjour Waly."];

/// Phrases PIÈGES : les voisins phonétiques EN CONTEXTE (négatifs durs de
/// fenêtres glissantes) — c'est là que le v1 s'effondrait (FAR 30-60 %).
pub const PHRASES_NEG_DURES: &[&str] = &[
    "La vallée est magnifique en été.",
    "Willy arrive demain matin.",
    "Le wallon est un dialecte belge.",
    "Cette valise est trop lourde.",
    "On va au Mali cet hiver.",
    "Le wagon de tête est complet.",
    "Un whisky sans glace, merci.",
    "Allez, on y va tout de suite.",
    "Voilà, c'est exactement ça.",
    "Ils sont allés à Bali en avril.",
    "L'allée du jardin est fleurie.",
    "Il avait un vieil ami à Vichy.",
];

/// Négatifs durs : voisins phonétiques de /wa.li/ + mots courants courts.
pub const MOTS_NEG: &[&str] = &[
    "vallée.", "Wallis.", "wallon.", "Willy.", "voilà.", "olé.", "allez.",
    "valise.", "Bali.", "Mali.", "wagon.", "whisky.", "ouais.", "salut.",
    "alors.", "vélo.", "canapé.", "métallique.",
];

pub const PHRASES_NEG: &[&str] = &[
    "Bonjour, comment ça va aujourd'hui ?",
    "Il fait vraiment beau ce matin.",
    "Tu peux me passer le sel s'il te plaît ?",
    "Je dois finir ce rapport avant ce soir.",
    "On se retrouve demain à la gare.",
    "La réunion commence dans dix minutes.",
    "J'ai oublié mes clés dans la voiture.",
    "Le café est encore chaud, sers-toi.",
    "Quelle heure est-il maintenant ?",
    "Les enfants jouent dans le jardin.",
    "Ce film était vraiment magnifique.",
    "Il faut acheter du pain en rentrant.",
    "Mon ordinateur est encore trop lent.",
    "La musique est un peu trop forte.",
    "Nous allons partir en vacances en août.",
    "Le train a vingt minutes de retard.",
    "Elle travaille à l'hôpital depuis mars.",
    "Range ta chambre avant de sortir.",
    "Le match commence à vingt et une heures.",
    "J'aimerais un thé avec un nuage de lait.",
    "La facture arrive à la fin du mois.",
    "Ce restaurant italien est excellent.",
    "Il pleut des cordes depuis ce matin.",
    "Appelle-moi quand tu arrives.",
];

fn write_wav16k(path: &Path, samples: &[f32]) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SR as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for s in samples {
        w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16)?;
    }
    w.finalize()?;
    Ok(())
}

pub fn read_wav_mono(path: &Path) -> Result<(Vec<f32>, u32)> {
    let mut r = hound::WavReader::open(path).with_context(|| path.display().to_string())?;
    let spec = r.spec();
    let ch = spec.channels as usize;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().map(|s| s.unwrap_or(0) as f32 / max).collect()
        }
        hound::SampleFormat::Float => r.samples::<f32>().map(|s| s.unwrap_or(0.0)).collect(),
    };
    let mono: Vec<f32> = if ch > 1 {
        samples.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect()
    } else {
        samples
    };
    Ok((mono, spec.sample_rate))
}

/// Rogne le silence de tête/queue (seuil relatif au pic, marge 50 ms).
fn trim(samples: &[f32]) -> Vec<f32> {
    let peak = samples.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak < 1e-4 {
        return Vec::new();
    }
    let th = peak * 0.02;
    let first = samples.iter().position(|s| s.abs() > th).unwrap_or(0);
    let last = samples.len() - samples.iter().rev().position(|s| s.abs() > th).unwrap_or(0);
    let margin = SR / 20;
    let a = first.saturating_sub(margin);
    let b = (last + margin).min(samples.len());
    samples[a..b].to_vec()
}

/// Prise plausible pour un MOT isolé : ni muette, ni fleuve (prise Pocket
/// qui divague), entre 0,15 s et 2,5 s après rognage.
fn prise_valide(samples: &[f32]) -> bool {
    let dur = samples.len() as f32 / SR as f32;
    (0.15..=2.5).contains(&dur)
}

struct Sortie<'a> {
    racine: &'a Path,
}

impl Sortie<'_> {
    fn ecrit(&self, sous: &str, voix: &str, nom: &str, idx: usize, audio: &[f32]) -> Result<()> {
        let dir = self.racine.join(sous).join(voix);
        std::fs::create_dir_all(&dir)?;
        write_wav16k(&dir.join(format!("{nom}_{idx:03}.wav")), audio)
    }
}

fn slug(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// Deuxième vague (itération v2) : porteuses positives + phrases pièges,
/// ADD-ONLY à côté du corpus v1 (`pos-ctx/`, `neg-pieges/`).
pub fn gen_extra(out: &Path, prises: usize) -> Result<()> {
    std::env::set_var("WALY_POCKET_SEED", "0");
    waly_voice::vad::init_onnxruntime().map_err(|e| anyhow!("{e}"))?;
    let sortie = Sortie { racine: out };

    let dll = std::env::var("WALY_SHERPA_DLL")
        .unwrap_or_else(|_| r"C:\waly\engines\sherpa\lib\sherpa-onnx-c-api.dll".into());
    let piper_dir = r"C:\waly\engines\models\vits-piper-fr_FR-upmc-medium";
    let mut piper = PiperTts::new(&dll, piper_dir, "fr_FR-upmc-medium", 2)
        .map_err(|e| anyhow!("Piper: {e}"))?;
    let piper_sr = piper.sample_rate();
    let mut refs_clone: Vec<(String, Vec<f32>, u32)> = Vec::new();
    for (sid, nom) in [(1, "pierre"), (0, "jessica")] {
        piper.set_speaker(sid);
        let r = piper
            .synth("Bonjour, je suis très content de discuter avec toi aujourd'hui, et nous allons parler de beaucoup de choses intéressantes.", 1.0)
            .map_err(|e| anyhow!("Piper ref {nom}: {e}"))?;
        refs_clone.push((format!("clone-{nom}"), r, piper_sr));
        for (ti, text) in TEXTES_POS_CONTEXTE.iter().enumerate() {
            for (vi, speed) in [0.85f32, 1.0, 1.2].iter().enumerate() {
                let a = piper.synth(text, *speed).map_err(|e| anyhow!("Piper: {e}"))?;
                let a = trim(&resample_sinc(&a, piper_sr, SR as u32));
                if prise_valide(&a) {
                    sortie.ecrit("pos-ctx", &format!("piper-{nom}"), "ctx", ti * 10 + vi, &a)?;
                }
            }
        }
        for (pi, phrase) in PHRASES_NEG_DURES.iter().enumerate() {
            let a = piper.synth(phrase, 1.0).map_err(|e| anyhow!("Piper: {e}"))?;
            let a = resample_sinc(&a, piper_sr, SR as u32);
            sortie.ecrit("neg-pieges", &format!("piper-{nom}"), "piege", pi, &a)?;
        }
        println!("Piper {nom} (extra) : fait");
    }

    let model_dir = std::env::var("WALY_POCKET_DIR")
        .unwrap_or_else(|_| r"C:\waly\engines\models\pocket-tts-fr-24l".into());
    let mut pocket = PocketNative::new(&model_dir, 4).map_err(|e| anyhow!("Pocket: {e}"))?;
    let pocket_sr = pocket.sample_rate();
    let mut voix: Vec<(String, Vec<f32>, u32)> = vec![
        ("fabien".into(), read_wav_mono(Path::new(r"C:\waly\engines\voices-fr\fabien.wav"))?.0, {
            read_wav_mono(Path::new(r"C:\waly\engines\voices-fr\fabien.wav"))?.1
        }),
        (
            "developpeuse".into(),
            read_wav_mono(Path::new(r"C:\waly\engines\voices-fr\developpeuse.wav"))?.0,
            read_wav_mono(Path::new(r"C:\waly\engines\voices-fr\developpeuse.wav"))?.1,
        ),
    ];
    voix.extend(refs_clone);

    for (nom, ref_audio, ref_sr) in &voix {
        pocket
            .set_voice(ref_audio, *ref_sr)
            .map_err(|e| anyhow!("Pocket voix {nom}: {e}"))?;
        let mut ok = 0usize;
        for (ti, text) in TEXTES_POS_CONTEXTE.iter().enumerate() {
            for p in 0..prises {
                let a = pocket.synth(text, 1.0).map_err(|e| anyhow!("Pocket: {e}"))?;
                let a = trim(&resample_sinc(&a, pocket_sr, SR as u32));
                if prise_valide(&a) {
                    sortie.ecrit("pos-ctx", nom, "ctx", ti * 1000 + p, &a)?;
                    ok += 1;
                }
            }
        }
        for (pi, phrase) in PHRASES_NEG_DURES.iter().enumerate() {
            for p in 0..2 {
                let a = pocket.synth(phrase, 1.0).map_err(|e| anyhow!("Pocket: {e}"))?;
                let a = resample_sinc(&a, pocket_sr, SR as u32);
                sortie.ecrit("neg-pieges", nom, "piege", pi * 10 + p, &a)?;
            }
        }
        println!("Pocket {nom} (extra) : {ok} porteuses valides");
    }
    Ok(())
}

/// Troisième vague : PSEUDO-TIMBRES par décalage de hauteur des références
/// de clonage (référence rééchantillonnée ×0,88 / ×1,14 avant `set_voice`
/// → l'encodeur mimi voit un AUTRE locuteur). La parade au sur-apprentissage
/// des timbres constaté au v2 (loss train 0,0009, val plate) : plus de
/// locuteurs, pas plus de prises.
pub fn gen_pitch(out: &Path, prises: usize) -> Result<()> {
    std::env::set_var("WALY_POCKET_SEED", "0");
    waly_voice::vad::init_onnxruntime().map_err(|e| anyhow!("{e}"))?;
    let sortie = Sortie { racine: out };

    let dll = std::env::var("WALY_SHERPA_DLL")
        .unwrap_or_else(|_| r"C:\waly\engines\sherpa\lib\sherpa-onnx-c-api.dll".into());
    let piper_dir = r"C:\waly\engines\models\vits-piper-fr_FR-upmc-medium";
    let mut piper = PiperTts::new(&dll, piper_dir, "fr_FR-upmc-medium", 2)
        .map_err(|e| anyhow!("Piper: {e}"))?;
    let piper_sr = piper.sample_rate();
    let mut bases: Vec<(String, Vec<f32>, u32)> = Vec::new();
    for (sid, nom) in [(1, "pierre"), (0, "jessica")] {
        piper.set_speaker(sid);
        let r = piper
            .synth("Bonjour, je suis très content de discuter avec toi aujourd'hui, et nous allons parler de beaucoup de choses intéressantes.", 1.0)
            .map_err(|e| anyhow!("Piper ref {nom}: {e}"))?;
        bases.push((format!("clone-{nom}"), r, piper_sr));
    }
    for nom in ["fabien", "developpeuse"] {
        let p = format!(r"C:\waly\engines\voices-fr\{nom}.wav");
        let (a, sr) = read_wav_mono(Path::new(&p))?;
        bases.push((nom.to_string(), a, sr));
    }

    let model_dir = std::env::var("WALY_POCKET_DIR")
        .unwrap_or_else(|_| r"C:\waly\engines\models\pocket-tts-fr-24l".into());
    let mut pocket = PocketNative::new(&model_dir, 4).map_err(|e| anyhow!("Pocket: {e}"))?;
    let pocket_sr = pocket.sample_rate();

    for (base, ref_audio, ref_sr) in &bases {
        for (tag, shift) in [("p088", 0.88f32), ("p114", 1.14)] {
            let nom = format!("{base}-{tag}");
            // Décalage : la référence lue plus grave/aiguë devient un autre
            // locuteur aux oreilles de l'encodeur de voix.
            let shifted =
                resample_sinc(ref_audio, *ref_sr, (*ref_sr as f32 * shift) as u32);
            pocket
                .set_voice(&shifted, *ref_sr)
                .map_err(|e| anyhow!("Pocket voix {nom}: {e}"))?;
            let mut ok = 0usize;
            for (ti, text) in TEXTES_POS.iter().enumerate() {
                for p in 0..prises {
                    let a = pocket.synth(text, 1.0).map_err(|e| anyhow!("Pocket: {e}"))?;
                    let a = trim(&resample_sinc(&a, pocket_sr, SR as u32));
                    if prise_valide(&a) {
                        sortie.ecrit("pos", &nom, "waly", ti * 1000 + p, &a)?;
                        ok += 1;
                    }
                }
            }
            for (ti, text) in TEXTES_POS_CONTEXTE.iter().enumerate() {
                for p in 0..prises / 2 {
                    let a = pocket.synth(text, 1.0).map_err(|e| anyhow!("Pocket: {e}"))?;
                    let a = trim(&resample_sinc(&a, pocket_sr, SR as u32));
                    if prise_valide(&a) {
                        sortie.ecrit("pos-ctx", &nom, "ctx", ti * 1000 + p, &a)?;
                        ok += 1;
                    }
                }
            }
            for mot in MOTS_NEG {
                let a = pocket.synth(mot, 1.0).map_err(|e| anyhow!("Pocket: {e}"))?;
                let a = trim(&resample_sinc(&a, pocket_sr, SR as u32));
                if prise_valide(&a) {
                    sortie.ecrit("neg-mots", &nom, &slug(mot), 0, &a)?;
                }
            }
            for (pi, phrase) in PHRASES_NEG_DURES.iter().enumerate() {
                let a = pocket.synth(phrase, 1.0).map_err(|e| anyhow!("Pocket: {e}"))?;
                let a = resample_sinc(&a, pocket_sr, SR as u32);
                sortie.ecrit("neg-pieges", &nom, "piege", pi, &a)?;
            }
            for (pi, phrase) in PHRASES_NEG.iter().enumerate().take(12) {
                let a = pocket.synth(phrase, 1.0).map_err(|e| anyhow!("Pocket: {e}"))?;
                let a = resample_sinc(&a, pocket_sr, SR as u32);
                sortie.ecrit("neg-phrases", &nom, "phr", pi, &a)?;
            }
            println!("Pocket {nom} : {ok} prises positives valides");
        }
    }
    Ok(())
}

pub fn gen(out: &Path, prises_pos: usize, prises_neg: usize) -> Result<()> {
    // Tirage LIBRE : l'identité gravée (graine 42) est pour le produit ;
    // ici on VEUT la variabilité entre prises.
    std::env::set_var("WALY_POCKET_SEED", "0");
    waly_voice::vad::init_onnxruntime().map_err(|e| anyhow!("{e}"))?;
    let sortie = Sortie { racine: out };

    // ── Piper (pierre sid 1 / jessica sid 0), vitesses variées ──────────────
    let dll = std::env::var("WALY_SHERPA_DLL")
        .unwrap_or_else(|_| r"C:\waly\engines\sherpa\lib\sherpa-onnx-c-api.dll".into());
    let piper_dir = r"C:\waly\engines\models\vits-piper-fr_FR-upmc-medium";
    let mut piper = PiperTts::new(&dll, piper_dir, "fr_FR-upmc-medium", 2)
        .map_err(|e| anyhow!("Piper: {e}"))?;
    let piper_sr = piper.sample_rate();

    let mut refs_clone: Vec<(String, Vec<f32>, u32)> = Vec::new();
    for (sid, nom) in [(1, "pierre"), (0, "jessica")] {
        piper.set_speaker(sid);
        // Référence de clonage Pocket : une phrase longue et neutre.
        let r = piper
            .synth("Bonjour, je suis très content de discuter avec toi aujourd'hui, et nous allons parler de beaucoup de choses intéressantes.", 1.0)
            .map_err(|e| anyhow!("Piper ref {nom}: {e}"))?;
        refs_clone.push((format!("clone-{nom}"), r, piper_sr));

        for (ti, text) in TEXTES_POS.iter().enumerate() {
            for (vi, speed) in [0.8f32, 0.95, 1.1, 1.25].iter().enumerate() {
                let a = piper.synth(text, *speed).map_err(|e| anyhow!("Piper: {e}"))?;
                let a = trim(&resample_sinc(&a, piper_sr, SR as u32));
                if prise_valide(&a) {
                    sortie.ecrit("pos", &format!("piper-{nom}"), "waly", ti * 10 + vi, &a)?;
                }
            }
        }
        for mot in MOTS_NEG {
            for (vi, speed) in [0.85f32, 1.15].iter().enumerate() {
                let a = piper.synth(mot, *speed).map_err(|e| anyhow!("Piper: {e}"))?;
                let a = trim(&resample_sinc(&a, piper_sr, SR as u32));
                if prise_valide(&a) {
                    sortie.ecrit("neg-mots", &format!("piper-{nom}"), &slug(mot), vi, &a)?;
                }
            }
        }
        for (pi, phrase) in PHRASES_NEG.iter().enumerate() {
            let a = piper.synth(phrase, 1.0).map_err(|e| anyhow!("Piper: {e}"))?;
            let a = resample_sinc(&a, piper_sr, SR as u32);
            sortie.ecrit("neg-phrases", &format!("piper-{nom}"), "phr", pi, &a)?;
        }
        println!("Piper {nom} : fait");
    }

    // ── Pocket : 4 timbres (2 refs officielles + 2 clones Piper) ────────────
    let model_dir = std::env::var("WALY_POCKET_DIR")
        .unwrap_or_else(|_| r"C:\waly\engines\models\pocket-tts-fr-24l".into());
    let mut pocket = PocketNative::new(&model_dir, 4).map_err(|e| anyhow!("Pocket: {e}"))?;
    let pocket_sr = pocket.sample_rate();

    let mut voix: Vec<(String, Vec<f32>, u32)> = vec![
        ("fabien".into(), read_wav_mono(Path::new(r"C:\waly\engines\voices-fr\fabien.wav"))?.0, {
            read_wav_mono(Path::new(r"C:\waly\engines\voices-fr\fabien.wav"))?.1
        }),
        (
            "developpeuse".into(),
            read_wav_mono(Path::new(r"C:\waly\engines\voices-fr\developpeuse.wav"))?.0,
            read_wav_mono(Path::new(r"C:\waly\engines\voices-fr\developpeuse.wav"))?.1,
        ),
    ];
    voix.extend(refs_clone);

    for (nom, ref_audio, ref_sr) in &voix {
        pocket
            .set_voice(ref_audio, *ref_sr)
            .map_err(|e| anyhow!("Pocket voix {nom}: {e}"))?;
        let mut ok = 0usize;
        let mut essais = 0usize;
        'pos: for (ti, text) in TEXTES_POS.iter().enumerate() {
            for p in 0..prises_pos {
                essais += 1;
                let a = pocket.synth(text, 1.0).map_err(|e| anyhow!("Pocket: {e}"))?;
                let a = trim(&resample_sinc(&a, pocket_sr, SR as u32));
                if prise_valide(&a) {
                    sortie.ecrit("pos", nom, "waly", ti * 1000 + p, &a)?;
                    ok += 1;
                }
                // Garde-fou : si le timbre cloné diverge (prises fleuves),
                // ne pas boucler pour rien.
                if essais > prises_pos * TEXTES_POS.len() * 2 {
                    break 'pos;
                }
            }
        }
        for mot in MOTS_NEG {
            for p in 0..prises_neg {
                let a = pocket.synth(mot, 1.0).map_err(|e| anyhow!("Pocket: {e}"))?;
                let a = trim(&resample_sinc(&a, pocket_sr, SR as u32));
                if prise_valide(&a) {
                    sortie.ecrit("neg-mots", nom, &slug(mot), p, &a)?;
                }
            }
        }
        for (pi, phrase) in PHRASES_NEG.iter().enumerate() {
            let a = pocket.synth(phrase, 1.0).map_err(|e| anyhow!("Pocket: {e}"))?;
            let a = resample_sinc(&a, pocket_sr, SR as u32);
            sortie.ecrit("neg-phrases", nom, "phr", pi, &a)?;
        }
        println!("Pocket {nom} : {ok} prises positives valides");
    }
    Ok(())
}
