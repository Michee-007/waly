//! Garde anti-écho v1 (clôture R1.5) — suppression, pas soustraction.
//!
//! Objectif : parler à Waly SANS casque. Le danger : sa propre voix sort des
//! haut-parleurs, revient par le micro, et déclenche le barge-in (il
//! s'interromprait lui-même) ou un faux tour utilisateur après la lecture.
//!
//! Principe : le lecteur fournit ce qui SORT réellement (au moment où ça
//! sort, pas au moment où c'est mis en file). La garde compare l'ENVELOPPE
//! d'énergie du micro (fenêtres de 10 ms) à celle du son joué sur la dernière
//! seconde : un écho a la même enveloppe, retardée (trajet acoustique +
//! tampons) et filtrée — la corrélation de Pearson y est robuste là où la
//! corrélation d'onde brute est détruite par la phase. Une vraie prise de
//! parole par-dessus a une enveloppe indépendante → corrélation basse.
//!
//! v2 (plus tard) : soustraction adaptative NLMS pour transcrire À TRAVERS
//! l'écho ; ici on se contente de ne pas se laisser berner par lui.

use std::collections::VecDeque;

/// Fenêtre d'enveloppe : 10 ms à 16 kHz.
const ENV_WIN: usize = 160;
/// Historique du son joué : 1,6 s (160 fenêtres) — couvre les retards réels.
const PLAYED_ENV: usize = 160;
/// Fenêtre d'analyse micro : 600 ms (60 fenêtres). Assez de structure pour
/// que deux enveloppes indépendantes ne s'alignent pas par hasard (mesuré :
/// à 400 ms, corrélation fortuite jusqu'à 0,67 ; l'écho vrai reste > 0,9).
const MIC_ENV: usize = 60;

pub struct EchoGate {
    played_env: VecDeque<f32>,
    mic_env: VecDeque<f32>,
    played_partial: Vec<f32>,
    mic_partial: Vec<f32>,
}

impl Default for EchoGate {
    fn default() -> Self {
        Self::new()
    }
}

impl EchoGate {
    pub fn new() -> Self {
        Self {
            played_env: VecDeque::with_capacity(PLAYED_ENV + 1),
            mic_env: VecDeque::with_capacity(MIC_ENV + 1),
            played_partial: Vec::with_capacity(ENV_WIN),
            mic_partial: Vec::with_capacity(ENV_WIN),
        }
    }

    fn push_env(queue: &mut VecDeque<f32>, cap: usize, partial: &mut Vec<f32>, samples: &[f32]) {
        for &s in samples {
            partial.push(s);
            if partial.len() == ENV_WIN {
                let rms =
                    (partial.iter().map(|x| x * x).sum::<f32>() / ENV_WIN as f32).sqrt();
                queue.push_back(rms);
                if queue.len() > cap {
                    queue.pop_front();
                }
                partial.clear();
            }
        }
    }

    /// Ce qui sort des haut-parleurs, à 16 kHz mono, AU MOMENT où ça sort.
    pub fn push_played(&mut self, samples: &[f32]) {
        Self::push_env(
            &mut self.played_env,
            PLAYED_ENV,
            &mut self.played_partial,
            samples,
        );
    }

    /// Ce qui entre au micro, à 16 kHz mono.
    pub fn observe_mic(&mut self, samples: &[f32]) {
        Self::push_env(&mut self.mic_env, MIC_ENV, &mut self.mic_partial, samples);
    }

    /// Vrai si le micro des ~600 dernières ms (`MIC_ENV` fenêtres de 10 ms)
    /// ressemble à un écho du son joué (voir [`Self::score`]).
    pub fn mic_is_echo(&self) -> bool {
        let (pearson, leak) = self.score();
        pearson > 0.70 && leak < 0.30
    }

    /// (meilleure corrélation de Pearson, fuite d'énergie hors-joué) au
    /// meilleur alignement — voir `mic_is_echo` pour les seuils.
    pub fn score(&self) -> (f32, f32) {
        if self.mic_env.len() < MIC_ENV || self.played_env.len() < MIC_ENV {
            return (0.0, 1.0);
        }
        let mic: Vec<f32> = self.mic_env.iter().copied().collect();
        let played: Vec<f32> = self.played_env.iter().copied().collect();
        // Rien ne joue de significatif -> pas d'écho possible.
        let played_peak = played.iter().fold(0.0f32, |m, &e| m.max(e));
        if played_peak < 0.01 {
            return (0.0, 1.0);
        }
        // Le micro « maintenant » correspond au joué « il y a delta » : on
        // aligne la fenêtre micro sur toutes les positions passées du joué.
        // DEUX critères au meilleur alignement (la corrélation seule se fait
        // piéger par des enveloppes quasi-périodiques alignées par hasard) :
        // 1. corrélation de Pearson élevée ;
        // 2. « fuite » faible : un écho n'a AUCUNE énergie là où le
        //    haut-parleur était muet — une vraie voix par-dessus, si.
        let mut best = (0.0f32, 1.0f32); // (pearson, fuite)
        for start in 0..=(played.len() - mic.len()) {
            let seg = &played[start..start + mic.len()];
            let c = pearson(&mic, seg);
            if c > best.0 {
                best = (c, leak_ratio(&mic, seg));
            }
        }
        best
    }
}

/// Part de l'énergie micro tombant dans les silences du signal joué
/// (fenêtres où le joué est sous 10 % de son pic).
fn leak_ratio(mic: &[f32], played_seg: &[f32]) -> f32 {
    let peak = played_seg.iter().fold(0.0f32, |m, &e| m.max(e));
    if peak < 1e-6 {
        return 1.0;
    }
    let gate = peak * 0.1;
    let total: f32 = mic.iter().map(|e| e * e).sum();
    if total < 1e-9 {
        return 0.0;
    }
    let in_silence: f32 = mic
        .iter()
        .zip(played_seg)
        .filter(|(_, &p)| p < gate)
        .map(|(m, _)| m * m)
        .sum();
    in_silence / total
}

/// Corrélation de Pearson de deux séries de même longueur.
fn pearson(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len() as f32;
    let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
    let mut cov = 0.0;
    let mut va = 0.0;
    let mut vb = 0.0;
    for (x, y) in a.iter().zip(b) {
        let (dx, dy) = (x - ma, y - mb);
        cov += dx * dy;
        va += dx * dx;
        vb += dy * dy;
    }
    if va < 1e-9 || vb < 1e-9 {
        return 0.0;
    }
    cov / (va.sqrt() * vb.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Signal de « parole » synthétique : syllabes de DURÉES et amplitudes
    /// variables (une grille fixe et des amplitudes binaires créent des
    /// coïncidences d'alignement que la vraie parole n'a pas — vécu en
    /// écrivant ces tests).
    fn burst_signal(seed: u64, secs: f32) -> Vec<f32> {
        let n = (16_000.0 * secs) as usize;
        let mut state = seed | 1;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u64 << 24) as f32 - 0.5
        };
        let mut h = seed;
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            h = (h ^ 0x9E37_79B9_7F4A_7C15).wrapping_mul(0xD1B5_4A32_D192_ED03);
            let dur = 1200 + ((h >> 48) as usize % 2400); // 75-225 ms
            let on = (h >> 40) % 5 < 3;
            let amp = if on { 0.1 + ((h >> 24) & 0xFFFF) as f32 / 65535.0 * 0.4 } else { 0.01 };
            for _ in 0..dur {
                if out.len() >= n {
                    break;
                }
                out.push(next() * amp);
            }
        }
        out
    }

    #[test]
    fn echo_retarde_et_attenue_detecte() {
        let played = burst_signal(42, 1.6);
        let mut gate = EchoGate::new();
        gate.push_played(&played);
        // écho : même signal, retardé de 200 ms, atténué 4x, le micro voit
        // les ~700 dernières ms de la fenêtre décalée
        let delay = 3200;
        let echo: Vec<f32> = played[played.len() - delay - 11200..played.len() - delay]
            .iter()
            .map(|s| s * 0.25)
            .collect();
        gate.observe_mic(&echo);
        assert!(gate.mic_is_echo(), "l'echo retarde doit etre reconnu");
    }

    #[test]
    fn vraie_voix_independante_passe() {
        let played = burst_signal(42, 1.6);
        let voice = burst_signal(777, 0.7); // enveloppe indépendante
        let mut gate = EchoGate::new();
        gate.push_played(&played);
        gate.observe_mic(&voice);
        let (p, l) = gate.score();
        assert!(
            !gate.mic_is_echo(),
            "une voix independante ne doit pas etre bloquee (pearson {p:.2}, fuite {l:.2})"
        );
    }

    #[test]
    fn silence_joue_jamais_echo() {
        let mut gate = EchoGate::new();
        gate.push_played(&vec![0.0; 16_000]);
        gate.observe_mic(&burst_signal(9, 0.3));
        assert!(!gate.mic_is_echo());
    }
}
