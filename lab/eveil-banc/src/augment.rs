//! Assemblage et augmentation des clips d'entraînement (dev-time pur).
//!
//! Chaque clip d'entraînement fait exactement [`crate::oww::CLIP_SAMPLES`]
//! (2,0 s) : le mot y est placé pour FINIR près de la fin (jitter 0-300 ms),
//! comme au runtime où le classifieur glisse tous les 80 ms. Augmentations :
//! vitesse/hauteur (rééchantillonnage sinc), gain, bruit blanc/brun (SNR
//! 5-30 dB). RNG déterministe (xorshift64*) : corpus reproductible.

use crate::oww::{CLIP_SAMPLES, SR};

pub struct XorShift(u64);

impl XorShift {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    /// Uniforme [0,1).
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    /// Uniforme [lo,hi).
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }
    /// Gaussienne approchée (somme de 12 uniformes, centrée réduite).
    pub fn gauss(&mut self) -> f32 {
        (0..12).map(|_| self.f32()).sum::<f32>() - 6.0
    }
}

fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

/// Bruit blanc ou brun (marche intégrée, aigus atténués) à RMS donné.
pub fn noise(rng: &mut XorShift, len: usize, target_rms: f32) -> Vec<f32> {
    let brown = rng.f32() < 0.5;
    let mut out = Vec::with_capacity(len);
    let mut acc = 0.0f32;
    for _ in 0..len {
        let w = rng.gauss();
        if brown {
            // Intégrateur qui fuit : spectre en 1/f², plus proche d'un fond
            // de pièce (ventilation, rumeur) qu'un blanc pur.
            acc = 0.98 * acc + 0.2 * w;
            out.push(acc);
        } else {
            out.push(w);
        }
    }
    let r = rms(&out).max(1e-9);
    let g = target_rms / r;
    out.iter_mut().for_each(|v| *v *= g);
    out
}

/// Assemble UN clip d'entraînement de 2,0 s : mot rééchantillonné
/// (vitesse 0,85-1,2), normalisé, gain 0,25-1,4, placé pour finir à
/// 0-300 ms de la fin, bruit de fond SNR 5-30 dB (ou silence 1 fois sur 5).
pub fn assemble_word(word: &[f32], rng: &mut XorShift) -> Vec<f32> {
    // Vitesse/hauteur : jouer à 16 kHz un signal rééchantillonné vers
    // 16k/speed déforme durée ET hauteur — l'augmentation classique.
    let speed = rng.range(0.85, 1.2);
    let to = (SR as f32 / speed) as u32;
    let word = waly_voice::pocket::resample_sinc(word, SR as u32, to);

    // Normalise au pic puis gain aléatoire (locuteur proche/loin).
    let peak = word.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9);
    let gain = 0.9 / peak * rng.range(0.25, 1.4);

    let mut clip = vec![0.0f32; CLIP_SAMPLES];
    let jitter = (rng.f32() * 0.3 * SR as f32) as usize; // fin à 0-300 ms du bord
    let w_len = word.len().min(CLIP_SAMPLES - 1);
    let end = CLIP_SAMPLES - jitter.min(CLIP_SAMPLES - w_len);
    let start = end - w_len;
    // Prise plus longue que le clip : garder la QUEUE (c'est là qu'est le
    // mot dans les porteuses « dis Waly ») — jamais couper la fin.
    for (i, s) in word[word.len() - w_len..].iter().enumerate() {
        clip[start + i] = s * gain;
    }

    if rng.f32() < 0.8 {
        let word_rms = rms(&clip[start..end]).max(1e-9);
        let snr_db = rng.range(5.0, 30.0);
        let n_rms = word_rms / 10f32.powf(snr_db / 20.0);
        let n = noise(rng, CLIP_SAMPLES, n_rms);
        for (c, v) in clip.iter_mut().zip(n) {
            *c += v;
        }
    }
    // Écrêtage doux : rester dans [-1,1] pour l'échelle int16 du mel.
    clip.iter_mut().for_each(|v| *v = v.clamp(-1.0, 1.0));
    clip
}
