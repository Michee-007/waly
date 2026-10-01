//! Profil matériel + recommandation de modèle (2026-09-10, stratégie open
//! source « point 3 » — ne dépendre d'aucune machine).
//!
//! Au premier lancement, Waly regarde la machine (RAM, cartes graphiques,
//! NPU, Smart App Control), la compare aux modèles déjà installés et propose
//! un cerveau (+ un modèle vision si le cerveau ne voit pas), avec les gestes
//! pour les obtenir. Waly NE TÉLÉCHARGE RIEN : il est scellé (huis clos) —
//! c'est l'utilisateur qui lance `ollama pull`.
//!
//! Détection Windows SANS dépendance neuve (windows-sys reste hors de
//! waly-core, cf. waly-seal) : quelques déclarations FFI vers kernel32 et
//! advapi32 — DLL système signées, SAC-safe (piège 3). Registre :
//! - classe Display `{4d36e968-…}\00NN` : `DriverDesc`,
//!   `HardwareInformation.qwMemorySize` (VRAM dédiée) ;
//! - classe ComputeAccelerator `{f01a9d53-…}` : le NPU (« NPU Compute
//!   Accelerator Device » sur Ryzen AI — vérifié sur la machine de référence) ;
//! - `CI\Policy\VerifiedAndReputablePolicyState` = 1 : Smart App Control actif.
//!
//! Règles de recommandation : MESURÉES sur la machine de référence
//! (docs/JOURNAL.md) quand c'est possible, sinon marquées « non mesuré ».

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Gpu {
    pub nom: String,
    pub vram_go: f32,
    pub integre: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Profil {
    pub ram_totale_go: f32,
    pub ram_dispo_go: f32,
    pub gpus: Vec<Gpu>,
    /// Nom du NPU s'il est présent (ex. « NPU Compute Accelerator Device »).
    pub npu: Option<String>,
    /// Smart App Control actif : les exécutables non signés sont bloqués.
    pub sac_actif: bool,
    /// Système (`windows`, `linux`, `macos`) — adapte les gestes proposés.
    pub os: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reco {
    pub moteur: String,
    pub cerveau: String,
    /// `true` : le cerveau voit lui-même (pas de modèle vision à ajouter).
    pub cerveau_voit: bool,
    pub modele_vision: Option<String>,
    pub raisons: Vec<String>,
    /// Gestes concrets : commandes à lancer, ou « déjà installé ».
    pub a_faire: Vec<String>,
    /// Bloc à placer dans waly.toml.
    pub toml: String,
}

// Modèles recommandés (noms Ollama) et tailles de téléchargement approchées.
const QWEN3_TEXTE: &str = "qwen3:4b-instruct-2507-q4_K_M";
const QWEN3_VL_4B: &str = "qwen3-vl:4b-instruct";
const QWEN3_VL_8B: &str = "qwen3-vl:8b-instruct";
const GEMMA3: &str = "gemma3:4b";
const TAILLES_GO: &[(&str, f32)] =
    &[(QWEN3_TEXTE, 2.5), (QWEN3_VL_4B, 3.3), (QWEN3_VL_8B, 6.1), (GEMMA3, 3.3)];

/// Carte graphique intégrée ? VRAM réservée < 2 Go, ou famille connue
/// (Radeon hors « RX », Intel hors Arc A/B). NVIDIA = dédiée.
pub fn est_integre(nom: &str, vram_go: f32) -> bool {
    let n = nom.to_lowercase();
    if n.contains("nvidia") || n.contains("geforce") || n.contains("rtx") {
        return false;
    }
    if n.contains("radeon") && !n.contains(" rx") {
        return true;
    }
    if n.contains("intel") {
        // Arc DÉDIÉE = numéro de série A/B (A380, A770, B580…) ; « Intel(R)
        // Arc(TM) Graphics » sans numéro = puce intégrée des CPU récents.
        let arc_dediee = n.contains("arc")
            && n.split(|c: char| !c.is_ascii_alphanumeric()).any(|t| {
                t.len() == 4
                    && (t.starts_with('a') || t.starts_with('b'))
                    && t[1..].chars().all(|c| c.is_ascii_digit())
            });
        return !arc_dediee;
    }
    vram_go < 2.0
}

fn installe(installes: &[String], modele: &str) -> bool {
    installes.iter().any(|i| i == modele || i.strip_suffix(":latest") == Some(modele))
}

fn taille(modele: &str) -> f32 {
    TAILLES_GO.iter().find(|(m, _)| *m == modele).map(|(_, t)| *t).unwrap_or(0.0)
}

/// Geste d'installation de FastFlowLM selon le système.
fn geste_flm(os: &str) -> String {
    if os == "windows" {
        "Installer FastFlowLM (github.com/FastFlowLM/FastFlowLM), puis lancer \
         engines/start-flm.ps1 -Model qwen3vl-it:4b"
            .into()
    } else {
        "Installer FastFlowLM pour Linux (fastflowlm.com/docs/install_lin), puis le \
         servir sur le port 42626 avec le modèle qwen3vl-it:4b"
            .into()
    }
}

/// Recommandation PURE (testable) pour un profil et les modèles installés.
pub fn recommander(p: &Profil, installes: &[String]) -> Reco {
    let mut raisons = Vec::new();
    // 1. NPU AMD : la voie la plus rapide MESURÉE (cerveau unique qui voit).
    if let Some(npu) = &p.npu {
        if !p.sac_actif {
            raisons.push(format!(
                "{npu} détecté : c'est la voie la plus rapide mesurée (tour de \
                 conversation ~1,4-2 s, tour vision ~5 s) avec un seul cerveau qui \
                 parle, utilise les outils et voit."
            ));
            return Reco {
                moteur: "FastFlowLM (NPU)".into(),
                cerveau: "qwen3vl-it:4b".into(),
                cerveau_voit: true,
                modele_vision: None,
                raisons,
                a_faire: vec![geste_flm(&p.os)],
                toml: "[llm]\nport = 42626\nmodele = \"qwen3vl-it:4b\"\n".into(),
            };
        }
        raisons.push(format!(
            "{npu} détecté, mais Smart App Control est actif et bloque FastFlowLM \
             tant qu'AMD ne le publie pas signé. En attendant : Ollama."
        ));
    }
    let dediee = p
        .gpus
        .iter()
        .filter(|g| !g.integre)
        .max_by(|a, b| a.vram_go.partial_cmp(&b.vram_go).unwrap_or(std::cmp::Ordering::Equal));
    // Windows nomme « AMD Radeon(TM) 840M… », Linux « AMD (amdgpu) ».
    let igpu_amd = p.gpus.iter().any(|g| {
        let n = g.nom.to_lowercase();
        g.integre && (n.contains("radeon") || n.starts_with("amd"))
    });
    let (cerveau, vision): (&str, Option<&str>) = match dediee {
        // 2. Carte dédiée : cerveau unique qui voit (non mesuré ici).
        Some(g) if g.vram_go >= 10.0 => {
            raisons.push(format!(
                "{} ({:.0} Go) : un modèle 8B qui voit tient entièrement dans la carte \
                 graphique (non mesuré sur la machine de référence).",
                g.nom, g.vram_go
            ));
            (QWEN3_VL_8B, None)
        }
        Some(g) if g.vram_go >= 5.0 => {
            raisons.push(format!(
                "{} ({:.0} Go) : un modèle 4B qui voit tient dans la carte graphique \
                 (non mesuré sur la machine de référence).",
                g.nom, g.vram_go
            ));
            (QWEN3_VL_4B, None)
        }
        // 3. Carte intégrée / processeur : la RAM décide.
        _ => {
            if p.ram_totale_go < 8.0 {
                raisons.push(format!(
                    "{:.0} Go de RAM : sous le minimum conseillé (8 Go) — Waly tournera, \
                     lentement, sans vision.",
                    p.ram_totale_go
                ));
                (QWEN3_TEXTE, None)
            } else if p.ram_totale_go < 12.0 {
                raisons.push(format!(
                    "{:.0} Go de RAM : un seul modèle tient ; cerveau texte, vision \
                     absente (Waly le dira honnêtement).",
                    p.ram_totale_go
                ));
                (QWEN3_TEXTE, None)
            } else if igpu_amd {
                raisons.push(
                    "Carte graphique intégrée AMD : qwen3-vl plante sous Ollama/Vulkan \
                     (mesuré, Ollama 0.33.3 et 0.34.0) ; gemma3 voit mais ne gère pas \
                     les outils → cerveau qwen3 (texte, outils) + gemma3 qui lui décrit \
                     les images. Lent en vision (rechargements de modèle)."
                        .into(),
                );
                (QWEN3_TEXTE, Some(GEMMA3))
            } else {
                raisons.push(format!(
                    "{:.0} Go de RAM : un modèle 4B qui voit (non mesuré sur cette \
                     famille de machine — s'il plante, cerveau {QWEN3_TEXTE} + \
                     modele_vision {GEMMA3}).",
                    p.ram_totale_go
                ));
                (QWEN3_VL_4B, None)
            }
        }
    };
    let cerveau_voit = cerveau != QWEN3_TEXTE;
    let mut a_faire = Vec::new();
    for m in std::iter::once(cerveau).chain(vision) {
        if !installe(installes, m) {
            a_faire.push(format!("ollama pull {m}   (~{:.1} Go)", taille(m)));
        }
    }
    if a_faire.is_empty() {
        a_faire.push("Tout est déjà installé ✓".into());
    }
    let mut toml = format!("[llm]\nport = 11434\nmodele = \"{cerveau}\"\n");
    if let Some(v) = vision {
        toml.push_str(&format!("modele_vision = \"{v}\"\n"));
    }
    Reco {
        moteur: "Ollama".into(),
        cerveau: cerveau.into(),
        cerveau_voit,
        modele_vision: vision.map(String::from),
        raisons,
        a_faire,
        toml,
    }
}

/// Profil de CETTE machine.
pub fn detecter() -> Profil {
    #[cfg(windows)]
    {
        win::detecter()
    }
    #[cfg(not(windows))]
    {
        // Linux : RAM par /proc/meminfo, cartes par /sys/class/drm (VRAM lue
        // pour amdgpu ; NVIDIA : non lue → recommandation prudente), NPU par
        // /sys/class/accel (pilote amdxdna des Ryzen AI).
        let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let ko = |cle: &str| -> f32 {
            meminfo
                .lines()
                .find(|l| l.starts_with(cle))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0)
        };
        Profil {
            ram_totale_go: arrondi(ko("MemTotal:") / 1_048_576.0),
            ram_dispo_go: arrondi(ko("MemAvailable:") / 1_048_576.0),
            gpus: gpus_linux(),
            npu: npu_linux(),
            sac_actif: false,
            os: std::env::consts::OS.into(),
        }
    }
}

fn arrondi(go: f32) -> f32 {
    (go * 10.0).round() / 10.0
}

/// Cartes graphiques Linux (`/sys/class/drm/cardN/device`). Le nom réel du
/// modèle exigerait pci.ids : on garde fabricant + pilote, suffisant pour
/// recommander. VRAM lue pour amdgpu (`mem_info_vram_total`).
#[cfg(not(windows))]
fn gpus_linux() -> Vec<Gpu> {
    let Ok(rd) = std::fs::read_dir("/sys/class/drm") else { return Vec::new() };
    let mut cartes: Vec<String> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("card") && !n.contains('-'))
        .collect();
    cartes.sort();
    let mut out = Vec::new();
    for c in cartes {
        let dev = format!("/sys/class/drm/{c}/device");
        let lire = |f: &str| std::fs::read_to_string(format!("{dev}/{f}")).ok().map(|s| s.trim().to_string());
        let Some(vendor) = lire("vendor") else { continue };
        let vram_go = lire("mem_info_vram_total")
            .and_then(|s| s.parse::<f64>().ok())
            .map(|o| arrondi((o / 1_073_741_824.0) as f32))
            .unwrap_or(0.0);
        let pilote = lire("uevent")
            .and_then(|u| u.lines().find_map(|l| l.strip_prefix("DRIVER=").map(String::from)))
            .unwrap_or_else(|| "pilote inconnu".into());
        let (fabricant, dediee) = match vendor.as_str() {
            "0x10de" => ("NVIDIA", true),
            "0x1002" => ("AMD", false),
            "0x8086" => ("Intel", false),
            _ => ("Carte graphique", false),
        };
        out.push(Gpu {
            nom: format!("{fabricant} ({pilote})"),
            vram_go,
            integre: !dediee && vram_go < 2.0,
        });
    }
    out
}

/// NPU Linux : un périphérique d'accélération (`/sys/class/accel/accelN`,
/// pilote amdxdna sur Ryzen AI).
#[cfg(not(windows))]
fn npu_linux() -> Option<String> {
    let e = std::fs::read_dir("/sys/class/accel").ok()?.filter_map(|e| e.ok()).next()?;
    Some(format!("NPU ({})", e.file_name().to_string_lossy()))
}

#[cfg(windows)]
mod win {
    use super::{arrondi, est_integre, Gpu, Profil};

    #[repr(C)]
    #[derive(Default)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }

    type Hkey = isize;
    // HKEY_LOCAL_MACHINE = (HKEY)(LONG)0x80000002, étendu en signe sur 64 bits.
    const HKEY_LOCAL_MACHINE: Hkey = 0x8000_0002_u32 as i32 as isize;
    const KEY_READ: u32 = 0x2_0019;
    const REG_SZ: u32 = 1;
    const REG_EXPAND_SZ: u32 = 2;
    const REG_BINARY: u32 = 3;
    const REG_DWORD: u32 = 4;
    const REG_QWORD: u32 = 11;

    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(buf: *mut MemoryStatusEx) -> i32;
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(cle: Hkey, sous: *const u16, opts: u32, acces: u32, out: *mut Hkey) -> i32;
        fn RegQueryValueExW(
            cle: Hkey,
            nom: *const u16,
            reserve: *mut u32,
            typ: *mut u32,
            data: *mut u8,
            len: *mut u32,
        ) -> i32;
        fn RegCloseKey(cle: Hkey) -> i32;
    }

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Clé ouverte en lecture, fermée au Drop.
    struct Cle(Hkey);

    impl Drop for Cle {
        fn drop(&mut self) {
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn ouvrir(chemin: &str) -> Option<Cle> {
        let mut h: Hkey = 0;
        let r = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, w(chemin).as_ptr(), 0, KEY_READ, &mut h) };
        (r == 0).then_some(Cle(h))
    }

    fn brut(cle: &Cle, nom: &str) -> Option<(u32, Vec<u8>)> {
        let nom = w(nom);
        let (mut typ, mut len) = (0u32, 0u32);
        let r = unsafe {
            RegQueryValueExW(cle.0, nom.as_ptr(), std::ptr::null_mut(), &mut typ, std::ptr::null_mut(), &mut len)
        };
        if r != 0 || len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        let r = unsafe {
            RegQueryValueExW(cle.0, nom.as_ptr(), std::ptr::null_mut(), &mut typ, buf.as_mut_ptr(), &mut len)
        };
        (r == 0).then(|| {
            buf.truncate(len as usize);
            (typ, buf)
        })
    }

    fn chaine(cle: &Cle, nom: &str) -> Option<String> {
        let (typ, b) = brut(cle, nom)?;
        if typ != REG_SZ && typ != REG_EXPAND_SZ {
            return None;
        }
        let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Some(String::from_utf16_lossy(&u).trim_end_matches('\0').trim().to_string())
    }

    fn entier(cle: &Cle, nom: &str) -> Option<u64> {
        let (typ, b) = brut(cle, nom)?;
        match (typ, b.len()) {
            (REG_DWORD | REG_BINARY, 4) => Some(u32::from_le_bytes(b[..4].try_into().ok()?) as u64),
            (REG_QWORD | REG_BINARY, 8) => Some(u64::from_le_bytes(b[..8].try_into().ok()?)),
            _ => None,
        }
    }

    const CLASSE_AFFICHAGE: &str =
        r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";
    const CLASSE_NPU: &str =
        r"SYSTEM\CurrentControlSet\Control\Class\{f01a9d53-3ff6-48d2-9f97-c8a7004be10c}";
    const GO: f32 = 1_073_741_824.0;

    pub fn detecter() -> Profil {
        let mut m = MemoryStatusEx { length: std::mem::size_of::<MemoryStatusEx>() as u32, ..Default::default() };
        let ok = unsafe { GlobalMemoryStatusEx(&mut m) } != 0;
        let (totale, dispo) = if ok { (m.total_phys, m.avail_phys) } else { (0, 0) };
        let mut gpus = Vec::new();
        let mut npu = None;
        for i in 0..16 {
            if let Some(c) = ouvrir(&format!(r"{CLASSE_AFFICHAGE}\{i:04}")) {
                let Some(nom) = chaine(&c, "DriverDesc") else { continue };
                let n = nom.to_lowercase();
                if n.starts_with("microsoft") || n.contains("virtual") || n.contains("remote") {
                    continue;
                }
                let vram = entier(&c, "HardwareInformation.qwMemorySize")
                    .or_else(|| entier(&c, "HardwareInformation.MemorySize"))
                    .unwrap_or(0);
                let vram_go = arrondi(vram as f32 / GO);
                gpus.push(Gpu { integre: est_integre(&nom, vram_go), nom, vram_go });
            }
            if npu.is_none() {
                if let Some(c) = ouvrir(&format!(r"{CLASSE_NPU}\{i:04}")) {
                    if let Some(nom) = chaine(&c, "DriverDesc") {
                        let n = nom.to_lowercase();
                        if n.contains("npu") || n.contains("ipu") || n.contains("neural") {
                            npu = Some(nom);
                        }
                    }
                }
            }
        }
        let sac_actif = ouvrir(r"SYSTEM\CurrentControlSet\Control\CI\Policy")
            .and_then(|c| entier(&c, "VerifiedAndReputablePolicyState"))
            == Some(1);
        Profil {
            ram_totale_go: arrondi(totale as f32 / GO),
            ram_dispo_go: arrondi(dispo as f32 / GO),
            gpus,
            npu,
            sac_actif,
            os: "windows".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(nom: &str, vram: f32) -> Gpu {
        Gpu { nom: nom.into(), vram_go: vram, integre: est_integre(nom, vram) }
    }

    fn profil(ram: f32, gpus: Vec<Gpu>, npu: bool, sac: bool) -> Profil {
        Profil {
            ram_totale_go: ram,
            ram_dispo_go: ram / 2.0,
            gpus,
            npu: npu.then(|| "NPU Compute Accelerator Device".into()),
            sac_actif: sac,
            os: "windows".into(),
        }
    }

    #[test]
    fn linux_igpu_amd_et_geste_flm() {
        let mut p = profil(
            16.0,
            vec![Gpu { nom: "AMD (amdgpu)".into(), vram_go: 0.5, integre: true }],
            false,
            false,
        );
        p.os = "linux".into();
        let r = recommander(&p, &[]);
        // Nommage Linux reconnu comme iGPU AMD → même règle mesurée.
        assert_eq!((r.cerveau.as_str(), r.modele_vision.as_deref()), (QWEN3_TEXTE, Some(GEMMA3)));
        p.npu = Some("NPU (accel0)".into());
        let r = recommander(&p, &[]);
        assert_eq!(r.moteur, "FastFlowLM (NPU)");
        assert!(r.a_faire[0].contains("Linux") && !r.a_faire[0].contains(".ps1"));
    }

    #[test]
    fn familles_integrees_et_dediees() {
        assert!(est_integre("AMD Radeon(TM) 840M Graphics", 0.5));
        assert!(!est_integre("AMD Radeon RX 7800 XT", 16.0));
        assert!(!est_integre("NVIDIA GeForce RTX 4060", 8.0));
        assert!(est_integre("Intel(R) UHD Graphics", 0.1));
        assert!(!est_integre("Intel(R) Arc(TM) A770 Graphics", 16.0));
        assert!(!est_integre("Intel(R) Arc(TM) B580 Graphics", 12.0));
        assert!(est_integre("Intel(R) Arc(TM) Graphics", 0.1));
    }

    #[test]
    fn machine_de_reference_npu_bloque_par_sac() {
        let p = profil(15.3, vec![gpu("AMD Radeon(TM) 840M Graphics", 0.5)], true, true);
        let installes = vec![QWEN3_TEXTE.to_string(), "gemma3:4b".into()];
        let r = recommander(&p, &installes);
        assert_eq!(r.moteur, "Ollama");
        assert_eq!(r.cerveau, QWEN3_TEXTE);
        assert_eq!(r.modele_vision.as_deref(), Some(GEMMA3));
        assert!(!r.cerveau_voit);
        assert!(r.raisons[0].contains("Smart App Control"));
        assert_eq!(r.a_faire, vec!["Tout est déjà installé ✓".to_string()]);
        // Le bloc proposé est lisible par notre parseur waly.toml.
        let t = crate::config::parse(&r.toml);
        assert_eq!(t.get("llm.modele_vision").map(String::as_str), Some(GEMMA3));
        assert_eq!(t.get("llm.port").map(String::as_str), Some("11434"));
    }

    #[test]
    fn npu_sans_sac_donne_fastflowlm() {
        let p = profil(15.3, vec![gpu("AMD Radeon(TM) 840M Graphics", 0.5)], true, false);
        let r = recommander(&p, &[]);
        assert_eq!(r.moteur, "FastFlowLM (NPU)");
        assert!(r.cerveau_voit && r.modele_vision.is_none());
        assert!(r.toml.contains("42626"));
    }

    #[test]
    fn cartes_dediees_cerveau_unique_qui_voit() {
        let r = recommander(&profil(16.0, vec![gpu("NVIDIA GeForce RTX 4060", 8.0)], false, true), &[]);
        assert_eq!(r.cerveau, QWEN3_VL_4B);
        assert!(r.cerveau_voit);
        assert_eq!(r.a_faire, vec![format!("ollama pull {QWEN3_VL_4B}   (~3.3 Go)")]);
        let r = recommander(&profil(32.0, vec![gpu("NVIDIA GeForce RTX 4070", 12.0)], false, false), &[]);
        assert_eq!(r.cerveau, QWEN3_VL_8B);
    }

    #[test]
    fn petites_machines_sans_carte() {
        let r = recommander(&profil(8.0, vec![], false, false), &[]);
        assert_eq!((r.cerveau.as_str(), r.modele_vision.clone()), (QWEN3_TEXTE, None));
        assert!(r.raisons[0].contains("un seul modèle tient"));
        let r = recommander(&profil(6.0, vec![], false, false), &[]);
        assert!(r.raisons[0].contains("sous le minimum"));
        // 16 Go, iGPU Intel : cerveau unique 4B qui voit (marqué non mesuré).
        let r = recommander(&profil(16.0, vec![gpu("Intel(R) Iris(R) Xe Graphics", 0.1)], false, false), &[]);
        assert_eq!(r.cerveau, QWEN3_VL_4B);
        assert!(r.raisons[0].contains("non mesuré"));
    }

    #[test]
    fn modele_installe_avec_suffixe_latest() {
        assert!(installe(&["gemma3:4b".into()], "gemma3:4b"));
        assert!(installe(&["gemma3:4b:latest".into()], "gemma3:4b"));
        assert!(!installe(&["gemma3:12b".into()], "gemma3:4b"));
    }

    #[test]
    fn detecter_ne_panique_pas() {
        // Sur l'hôte de test (Linux/WSL ou Windows) : un profil cohérent.
        let p = detecter();
        assert!(p.ram_totale_go >= 0.0 && p.ram_dispo_go <= p.ram_totale_go + 0.1);
    }
}
