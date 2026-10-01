//! Boucle de perception → ÉVÉNEMENTS (R4 chantier 3).
//!
//! L'étage rapide du mode appel : à cadence fixe (défaut 8 Hz), une frame →
//! YuNet (présence, attention) → machine à états
//! à hystérésis → événements typés. Les consommateurs (éclipse du desktop,
//! contexte du prompt) ne voient JAMAIS de pixels : uniquement ces
//! événements sémantiques (règle vie privée du plan R4).
//!
//! La machine ([`Machine`]) est pure et testée sur l'hôte ; seule la boucle
//! ([`demarrer`]) touche la caméra (Windows).

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// Quelqu'un entre dans le champ (après hystérésis).
    Arrivee { visages: usize },
    /// Plus personne dans le champ (après hystérésis).
    Depart,
    /// L'orientation de tête a changé (vers l'écran ou ailleurs).
    Attention { vers_ecran: bool },
    /// La SCÈNE a changé et s'est stabilisée (R4.5 ch. 3) : déclencheur
    /// des moments VLM proactifs — aucun pixel ne sort, juste le signal.
    Scene,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Cadence de la boucle (Hz).
    pub cadence_hz: f32,
    /// Frames consécutives avec visage avant `Arrivee` (réactif).
    pub frames_arrivee: usize,
    /// Frames consécutives sans visage avant `Depart` (tolérant : une
    /// détection ratée ne doit pas faire « partir » l'utilisateur).
    pub frames_depart: usize,
    /// Frames consécutives dans l'état opposé avant bascule d'attention.
    pub frames_attention: usize,
}

impl Default for Config {
    fn default() -> Self {
        // À 8 Hz : arrivée ≈ 250 ms, départ ≈ 1 s, attention ≈ 375 ms —
        // tous sous le budget < 500 ms perçu
        // (le départ est volontairement plus lent, c'est un choix produit).
        Self {
            cadence_hz: 8.0,
            frames_arrivee: 2,
            frames_depart: 8,
            frames_attention: 3,
        }
    }
}

/// Ce que la boucle observe sur UNE frame (déjà réduit — aucun pixel).
#[derive(Debug, Clone, Default)]
pub struct Observation {
    pub visages: usize,
    /// Attention du plus grand visage, si un visage est là.
    pub vers_ecran: Option<bool>,
}

/// Machine à états pure : hystérésis présence/attention.
pub struct Machine {
    config: Config,
    present: bool,
    serie_avec: usize,
    serie_sans: usize,
    attention: Option<bool>,
    serie_attention: usize,
}

impl Machine {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            present: false,
            serie_avec: 0,
            serie_sans: 0,
            attention: None,
            serie_attention: 0,
        }
    }

    pub fn present(&self) -> bool {
        self.present
    }

    /// Pousse une observation, rend les événements déclenchés.
    pub fn pousser(&mut self, obs: &Observation) -> Vec<Event> {
        let mut events = Vec::new();
        // --- Présence (hystérésis asymétrique) ---
        if obs.visages > 0 {
            self.serie_avec += 1;
            self.serie_sans = 0;
            if !self.present && self.serie_avec >= self.config.frames_arrivee {
                self.present = true;
                events.push(Event::Arrivee { visages: obs.visages });
            }
        } else {
            self.serie_sans += 1;
            self.serie_avec = 0;
            if self.present && self.serie_sans >= self.config.frames_depart {
                self.present = false;
                // L'attention repart de zéro au retour.
                self.attention = None;
                self.serie_attention = 0;
                events.push(Event::Depart);
            }
        }
        if !self.present {
            return events;
        }
        // --- Attention (bascule après N frames opposées) ---
        if let Some(vers) = obs.vers_ecran {
            match self.attention {
                None => {
                    // Premier état : émis immédiatement (l'arrivée vient
                    // d'être signalée, l'éclipse veut savoir tout de suite).
                    self.attention = Some(vers);
                    events.push(Event::Attention { vers_ecran: vers });
                }
                Some(actuel) if vers != actuel => {
                    self.serie_attention += 1;
                    if self.serie_attention >= self.config.frames_attention {
                        self.attention = Some(vers);
                        self.serie_attention = 0;
                        events.push(Event::Attention { vers_ecran: vers });
                    }
                }
                Some(_) => self.serie_attention = 0,
            }
        }
        events
    }
}

// ---------------------------------------------------------------------------
// Boucle temps réel (caméra → Machine) — Windows seulement.
/// dHash 64 bits d'une frame RGB : grille 9×8 de luma moyenne par bloc,
/// bit = « plus clair que le voisin de droite ». Deux vues de la même scène
/// ont une distance de Hamming faible ; coût ~µs (72 moyennes
/// sous-échantillonnées) — le déclencheur quasi gratuit de la cascade
/// (littérature : le resize est 95 % du coût d'un hash, ici il est
/// implicite dans l'échantillonnage).
pub fn dhash(rgb: &[u8], w: usize, h: usize) -> u64 {
    if w == 0 || h == 0 || rgb.len() < w * h * 3 {
        return 0;
    }
    let mut cells = [[0f64; 9]; 8];
    for (gy, ligne) in cells.iter_mut().enumerate() {
        for (gx, cell) in ligne.iter_mut().enumerate() {
            let x0 = gx * w / 9;
            let x1 = ((gx + 1) * w / 9).max(x0 + 1);
            let y0 = gy * h / 8;
            let y1 = ((gy + 1) * h / 8).max(y0 + 1);
            let sx = ((x1 - x0) / 4).max(1);
            let sy = ((y1 - y0) / 4).max(1);
            let (mut somme, mut n) = (0f64, 0f64);
            let mut y = y0;
            while y < y1 {
                let mut x = x0;
                while x < x1 {
                    let i = (y * w + x) * 3;
                    somme += 0.299 * rgb[i] as f64
                        + 0.587 * rgb[i + 1] as f64
                        + 0.114 * rgb[i + 2] as f64;
                    n += 1.0;
                    x += sx;
                }
                y += sy;
            }
            *cell = somme / n.max(1.0);
        }
    }
    let mut hash = 0u64;
    for gy in 0..8 {
        for gx in 0..8 {
            hash <<= 1;
            if cells[gy][gx] > cells[gy][gx + 1] {
                hash |= 1;
            }
        }
    }
    hash
}

/// Détecteur de changement de SCÈNE (pur, testé) : événement quand le hash
/// est LOIN de la référence (> `seuil_change`) ET STABLE d'une frame à
/// l'autre (≤ `seuil_stable`) pendant `frames_stables` frames — la
/// stabilité écarte le simple mouvement (marcher devant la caméra n'est
/// pas un changement de scène ; poser un objet, changer de pièce, oui).
pub struct SceneDetector {
    reference: Option<u64>,
    precedent: Option<u64>,
    serie: usize,
    pub seuil_change: u32,
    pub seuil_stable: u32,
    pub frames_stables: usize,
}

impl Default for SceneDetector {
    fn default() -> Self {
        // À 8 Hz : 8 frames stables ≈ 1 s de nouvelle scène posée.
        Self {
            reference: None,
            precedent: None,
            serie: 0,
            seuil_change: 16,
            seuil_stable: 6,
            frames_stables: 8,
        }
    }
}

impl SceneDetector {
    /// Pousse le hash d'une frame ; `true` = la scène vient de changer
    /// (la nouvelle devient la référence).
    pub fn pousser(&mut self, hash: u64) -> bool {
        let stable = self
            .precedent
            .map(|p| (p ^ hash).count_ones() <= self.seuil_stable)
            .unwrap_or(false);
        self.precedent = Some(hash);
        let Some(reference) = self.reference else {
            self.reference = Some(hash);
            return false;
        };
        if (reference ^ hash).count_ones() > self.seuil_change && stable {
            self.serie += 1;
        } else {
            self.serie = 0;
        }
        if self.serie >= self.frames_stables {
            self.reference = Some(hash);
            self.serie = 0;
            return true;
        }
        false
    }
}

// ---------------------------------------------------------------------------

#[cfg(windows)]
pub use boucle::{demarrer, Cliche, Percepteur, Stats};

#[cfg(not(windows))]
pub use stub::{demarrer, Cliche, Percepteur, Stats};

/// Hors Windows : même surface publique, erreur honnête à l'appel — sert
/// uniquement au `cargo check/test --workspace` sur l'hôte WSL (le desktop
/// et la caméra ne tournent que côté Windows).
#[cfg(not(windows))]
mod stub {
    use std::sync::mpsc::Receiver;

    use super::{Config, Event};

    #[derive(Debug, Default, Clone)]
    pub struct Stats {
        pub cycles: usize,
        pub grab_ms_med: f64,
        pub detect_ms_med: f64,
        pub cadence_effective_hz: f64,
    }

    pub struct Percepteur;

    #[derive(Clone)]
    pub struct Cliche;

    impl Cliche {
        pub fn jpeg(&self, _max_w: u32) -> Result<(Vec<u8>, u32, u32), String> {
            Err("caméra : Windows seulement".into())
        }
    }

    impl Percepteur {
        pub fn arreter(self) -> Stats {
            Stats::default()
        }
        pub fn cliche(&self) -> Cliche {
            Cliche
        }
        pub fn jpeg(&self, _max_w: u32) -> Result<(Vec<u8>, u32, u32), String> {
            Err("caméra : Windows seulement".into())
        }
    }

    pub fn demarrer(
        _cam_index: u32,
        _config: Config,
        _modele_visage: &str,
    ) -> Result<(Receiver<Event>, Percepteur), String> {
        Err("perception caméra : Windows seulement".into())
    }
}

#[cfg(windows)]
mod boucle {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{channel, Receiver};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use super::{Config, Event, Machine, Observation};
    use crate::camera::Cam;
    use crate::face::FaceDetector;

    /// Coûts réels de la boucle, relevés à l'arrêt.
    #[derive(Debug, Default, Clone)]
    pub struct Stats {
        pub cycles: usize,
        pub grab_ms_med: f64,
        pub detect_ms_med: f64,
        pub cadence_effective_hz: f64,
    }

    pub struct Percepteur {
        stop: Arc<AtomicBool>,
        handle: std::thread::JoinHandle<Stats>,
        cliche: Arc<Mutex<Option<crate::camera::Frame>>>,
    }

    /// Poignée CLONABLE et Send sur le cliché de la boucle : l'aperçu UI
    /// (auto-vue de l'écran d'appel) la lit sans passer par le thread qui
    /// possède le `Percepteur` — sinon la vignette gèlerait pendant les
    /// tours LLM (worker sériel).
    #[derive(Clone)]
    pub struct Cliche(Arc<Mutex<Option<crate::camera::Frame>>>);

    impl Cliche {
        /// Dernière frame, réduite (largeur max `max_w`, échelle uniforme) et
        /// encodée JPEG q80 — 640 px = format du moment VLM (~300 tok de
        /// vision, GATE A), 320 px = aperçu UI. Rend (octets, largeur,
        /// hauteur). La frame ne quitte JAMAIS la RAM ici.
        pub fn jpeg(&self, max_w: u32) -> Result<(Vec<u8>, u32, u32), String> {
            let guard = self.0.lock().map_err(|_| "verrou cliché empoisonné")?;
            let frame = guard.as_ref().ok_or("aucune frame disponible (boucle jeune)")?;
            let img: image::RgbImage =
                image::ImageBuffer::from_raw(frame.width, frame.height, frame.rgb.clone())
                    .ok_or("frame RGB incohérente")?;
            let (w, h) = if frame.width > max_w {
                let h = frame.height * max_w / frame.width;
                (max_w, h.max(1))
            } else {
                (frame.width, frame.height)
            };
            let reduite = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
            let mut jpeg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 80)
                .encode(&reduite, w, h, image::ColorType::Rgb8)
                .map_err(|e| format!("encodage JPEG: {e}"))?;
            Ok((jpeg, w, h))
        }
    }

    impl Percepteur {
        pub fn arreter(self) -> Stats {
            self.stop.store(true, Ordering::Relaxed);
            self.handle.join().unwrap_or_default()
        }

        /// Poignée partagée sur le cliché (voir [`Cliche`]).
        pub fn cliche(&self) -> Cliche {
            Cliche(self.cliche.clone())
        }

        pub fn jpeg(&self, max_w: u32) -> Result<(Vec<u8>, u32, u32), String> {
            self.cliche().jpeg(max_w)
        }
    }

    /// Démarre la perception sur la caméra `cam_index`.
    pub fn demarrer(
        cam_index: u32,
        config: Config,
        modele_visage: &str,
    ) -> Result<(Receiver<Event>, Percepteur), String> {
        // La caméra MSMF n'est pas `Send` (COM) : tout naît DANS le thread,
        // et un handshake d'init rend l'échec synchrone pour l'appelant
        // (caméra occupée, modèle manquant…).
        let modele_visage = modele_visage.to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let cliche = Arc::new(Mutex::new(None));
        let cliche_thread = cliche.clone();
        let (tx, rx) = channel();
        let (init_tx, init_rx) = channel::<Result<(), String>>();
        let periode = Duration::from_secs_f32(1.0 / config.cadence_hz.max(0.5));
        let handle = std::thread::spawn(move || {
            let mut cam = match Cam::open(cam_index) {
                Ok(c) => c,
                Err(e) => {
                    let _ = init_tx.send(Err(e));
                    return Stats::default();
                }
            };
            let mut visage = match FaceDetector::load(&modele_visage) {
                Ok(v) => v,
                Err(e) => {
                    let _ = init_tx.send(Err(e));
                    return Stats::default();
                }
            };
            let _ = init_tx.send(Ok(()));
            let mut machine = Machine::new(config);
            let mut scene = super::SceneDetector::default();
            let (mut g_ms, mut d_ms) = (Vec::new(), Vec::new());
            let debut = Instant::now();
            while !stop_thread.load(Ordering::Relaxed) {
                let cycle = Instant::now();
                let mut obs = Observation::default();
                let t = Instant::now();
                match cam.grab() {
                    Ok(frame) => {
                        g_ms.push(t.elapsed().as_secs_f64() * 1000.0);
                        let t = Instant::now();
                        match visage.detect(&frame.rgb, frame.width, frame.height) {
                            Ok(faces) => {
                                d_ms.push(t.elapsed().as_secs_f64() * 1000.0);
                                obs.visages = faces.len();
                                // Le plus grand visage porte l'attention.
                                if let Some(f) = faces.iter().max_by(|a, b| {
                                    (a.bbox[2] * a.bbox[3]).total_cmp(&(b.bbox[2] * b.bbox[3]))
                                }) {
                                    obs.vers_ecran = Some(f.vers_ecran());
                                }
                            }
                            Err(e) => tracing::warn!("détection: {e}"),
                        }
                        // Changement de scène (ch. 3) : dHash ~µs par frame.
                        let hash = super::dhash(
                            &frame.rgb,
                            frame.width as usize,
                            frame.height as usize,
                        );
                        if scene.pousser(hash) && tx.send(Event::Scene).is_err() {
                            stop_thread.store(true, Ordering::Relaxed);
                        }
                        // La frame (détection finie) devient le cliché courant,
                        // servi au moment VLM (outil regarder / raccourci).
                        if let Ok(mut c) = cliche_thread.lock() {
                            *c = Some(frame);
                        }
                    }
                    Err(e) => tracing::warn!("capture: {e}"),
                }
                for ev in machine.pousser(&obs) {
                    if tx.send(ev).is_err() {
                        // Plus personne n'écoute : on s'arrête proprement.
                        stop_thread.store(true, Ordering::Relaxed);
                    }
                }
                if let Some(reste) = periode.checked_sub(cycle.elapsed()) {
                    std::thread::sleep(reste);
                }
            }
            let med = |v: &mut Vec<f64>| {
                if v.is_empty() {
                    return 0.0;
                }
                v.sort_by(f64::total_cmp);
                v[v.len() / 2]
            };
            Stats {
                cycles: g_ms.len(),
                grab_ms_med: med(&mut g_ms),
                detect_ms_med: med(&mut d_ms),
                cadence_effective_hz: g_ms.len() as f64
                    / debut.elapsed().as_secs_f64().max(0.001),
            }
        });
        match init_rx.recv() {
            Ok(Ok(())) => Ok((rx, Percepteur { stop, handle, cliche })),
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => Err("thread de perception mort à l'initialisation".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_test() -> Config {
        Config {
            cadence_hz: 8.0,
            frames_arrivee: 2,
            frames_depart: 4,
            frames_attention: 3,
        }
    }

    fn avec_visage(vers: bool) -> Observation {
        Observation { visages: 1, vers_ecran: Some(vers) }
    }

    fn sans_visage() -> Observation {
        Observation::default()
    }

    #[test]
    fn scene_changee_stable_seulement() {
        let mut d = SceneDetector { frames_stables: 3, ..Default::default() };
        let ref_hash = 0u64;
        let loin = u64::MAX; // distance 64 > seuil_change
        // 1re frame : pose la référence, jamais d'événement.
        assert!(!d.pousser(ref_hash));
        // Même scène répétée : rien.
        for _ in 0..10 {
            assert!(!d.pousser(ref_hash));
        }
        // Mouvement : loin de la référence mais INSTABLE (alternance) → rien.
        let loin2 = loin ^ 0x00FF_FF00_0000_0000; // à 16 bits de `loin`
        for _ in 0..6 {
            assert!(!d.pousser(loin));
            assert!(!d.pousser(loin2));
        }
        // Nouvelle scène posée : loin ET stable 3 frames → UN événement.
        assert!(!d.pousser(loin)); // stable=false (venait de loin2)
        assert!(!d.pousser(loin)); // serie=1
        assert!(!d.pousser(loin)); // serie=2
        assert!(d.pousser(loin)); // serie=3 → scène changée
        // Puis silence : la nouvelle scène est la référence.
        for _ in 0..10 {
            assert!(!d.pousser(loin));
        }
    }

    #[test]
    fn dhash_discrimine_et_tolere() {
        // Image A : moitié gauche sombre, droite claire ; B = inverse ;
        // C = A avec un léger bruit. dist(A,B) grand, dist(A,C) petit.
        let (w, h) = (90, 80);
        let mk = |f: &dyn Fn(usize, usize) -> u8| {
            let mut rgb = vec![0u8; w * h * 3];
            for y in 0..h {
                for x in 0..w {
                    let v = f(x, y);
                    let i = (y * w + x) * 3;
                    rgb[i] = v;
                    rgb[i + 1] = v;
                    rgb[i + 2] = v;
                }
            }
            rgb
        };
        // A : rampe horizontale (chaque paire de blocs adjacents a un ordre
        // FRANC — les aplats purs sont degeneres pour un dHash, leurs
        // quasi-egalites basculent au moindre bruit).
        let a = dhash(&mk(&|x, _| (x * 220 / w) as u8), w, h);
        // B : rayures verticales — une VRAIE autre scene.
        let b = dhash(&mk(&|x, _| if (x * 9 / w) % 2 == 0 { 220 } else { 20 }), w, h);
        // C : la rampe de A + petit bruit (amplitude << delta entre blocs).
        let c = dhash(&mk(&|x, y| (x * 220 / w) as u8 + ((x + y) % 7) as u8), w, h);
        assert!((a ^ b).count_ones() > 16, "A vs B = {}", (a ^ b).count_ones());
        assert!((a ^ c).count_ones() <= 6, "A vs C = {}", (a ^ c).count_ones());
    }

    #[test]
    fn arrivee_apres_hysteresis_pas_avant() {
        let mut m = Machine::new(config_test());
        assert!(m.pousser(&avec_visage(true)).is_empty());
        let evs = m.pousser(&avec_visage(true));
        assert!(evs.contains(&Event::Arrivee { visages: 1 }));
        // L'attention initiale part avec l'arrivée.
        assert!(evs.contains(&Event::Attention { vers_ecran: true }));
    }

    #[test]
    fn une_frame_ratee_ne_fait_pas_partir() {
        let mut m = Machine::new(config_test());
        m.pousser(&avec_visage(true));
        m.pousser(&avec_visage(true));
        assert!(m.present());
        // 3 trous < frames_depart=4 : toujours présent.
        for _ in 0..3 {
            assert!(m.pousser(&sans_visage()).is_empty());
        }
        assert!(m.present());
        let evs = m.pousser(&sans_visage());
        assert_eq!(evs, vec![Event::Depart]);
        assert!(!m.present());
    }

    #[test]
    fn attention_bascule_apres_n_frames_opposees() {
        let mut m = Machine::new(config_test());
        m.pousser(&avec_visage(true));
        m.pousser(&avec_visage(true)); // Arrivee + Attention(true)
        // 2 frames « ailleurs » : pas encore.
        assert!(m.pousser(&avec_visage(false)).is_empty());
        assert!(m.pousser(&avec_visage(false)).is_empty());
        // 3e : bascule.
        assert_eq!(
            m.pousser(&avec_visage(false)),
            vec![Event::Attention { vers_ecran: false }]
        );
        // Un aller-retour bref ne re-bascule pas.
        assert!(m.pousser(&avec_visage(true)).is_empty());
        assert!(m.pousser(&avec_visage(false)).is_empty());
        assert!(m.pousser(&avec_visage(true)).is_empty());
    }

    #[test]
    fn presence_se_reemet_au_retour() {
        let mut m = Machine::new(config_test());
        m.pousser(&avec_visage(true));
        m.pousser(&avec_visage(true)); // arrivée
        // Départ → tout se ré-émettra au retour.
        for _ in 0..4 {
            m.pousser(&sans_visage());
        }
        assert!(!m.present());
        m.pousser(&avec_visage(true));
        let evs = m.pousser(&avec_visage(true));
        assert!(evs.contains(&Event::Arrivee { visages: 1 }));
        assert!(evs.contains(&Event::Attention { vers_ecran: true }));
    }

    #[test]
    fn absent_aucun_evenement_attention() {
        let mut m = Machine::new(config_test());
        assert!(m.pousser(&sans_visage()).is_empty());
        assert!(m.pousser(&sans_visage()).is_empty());
    }
}
