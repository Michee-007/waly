//! Trouver les agents IA qui TOURNENT sur cette machine, pour proposer de les
//! sceller sans que l'utilisateur ait à chercher un fichier `.exe`.
//!
//! Ce module ne scelle rien : il lit la liste des processus de la session
//! (nom du programme et ligne de commande), reconnaît les agents connus et
//! dit honnêtement sur QUEL programme porterait le scellé. Limite assumée
//! (voir `docs/AUDIT-2026-10-02-promesses-rejouees.md`, problème A) : le scellé
//! vaut par programme. Un agent écrit en Python ou en Node tourne sur un
//! moteur que d'autres logiciels utilisent ; le sceller les coupe aussi. On le
//! dit, avec le nombre de programmes concernés, et l'utilisateur décide.

use serde::Serialize;

/// Un processus vu sur la machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Processus {
    pub pid: u32,
    /// Chemin complet du programme.
    pub exe: String,
    /// Ligne de commande (vide si illisible).
    pub ligne: String,
}

/// Un agent reconnu, regroupé par programme à sceller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentTrouve {
    /// Nom lisible (« Ollama », « Hermes »…).
    pub nom: String,
    /// Le programme sur lequel porterait le scellé.
    pub exe: String,
    /// Ce programme est un moteur partagé (Python, Node…).
    pub partage: bool,
    /// Autres processus en cours qui utilisent le MÊME programme sans être cet
    /// agent : ils seraient coupés aussi.
    pub autres: usize,
    /// Nombre de processus de cet agent.
    pub processus: usize,
    /// Leurs identifiants : figer un agent vise CES processus, pas le
    /// programme. C'est précis même sur un moteur partagé.
    pub pids: Vec<u32>,
    /// Ligne de commande de son premier processus : c'est elle qu'on relance
    /// pour le mettre dans l'enclos.
    pub ligne: String,
}

/// (nom lisible, marqueurs cherchés dans le nom du programme, marqueurs
/// cherchés dans la ligne de commande). Tout en minuscules.
const CATALOGUE: &[(&str, &[&str], &[&str])] = &[
    ("OpenClaw", &["openclaw"], &["openclaw"]),
    ("Hermes", &["hermes"], &["hermes-agent", "hermes_agent", "hermes_cli"]),
    ("NemoClaw", &["nemoclaw"], &["nemoclaw"]),
    ("Ollama", &["ollama.exe"], &[]),
    ("LM Studio", &["lm studio", "lmstudio", "lms.exe"], &[]),
    ("Jan", &["jan.exe"], &[]),
    ("Claude", &["claude"], &["claude-code", "@anthropic-ai"]),
    ("Codex", &["codex"], &["@openai/codex"]),
    ("Gemini CLI", &[], &["@google/gemini-cli", "gemini-cli"]),
    ("Aider", &["aider"], &["aider"]),
    ("Goose", &["goose.exe", "goosed.exe"], &[]),
    ("Open Interpreter", &[], &["open-interpreter", "open_interpreter"]),
];

fn base(exe: &str) -> String {
    exe.rsplit(['\\', '/']).next().unwrap_or(exe).to_ascii_lowercase()
}

/// Les programmes de Waly lui-même ne sont pas des « autres agents ».
fn est_waly(exe: &str) -> bool {
    let b = base(exe);
    b.starts_with("waly") || b == "flm.exe"
}

/// Quel agent connu est ce processus ? Le nom du programme d'abord ; sinon la
/// ligne de commande, mais seulement pour un moteur partagé (sinon n'importe
/// quel éditeur ouvert sur un dossier « hermes » serait pris pour l'agent).
pub fn reconnaitre(exe: &str, ligne: &str) -> Option<&'static str> {
    if est_waly(exe) {
        return None;
    }
    let b = base(exe);
    for (nom, noms, _) in CATALOGUE {
        if noms.iter().any(|m| b.contains(m)) {
            return Some(nom);
        }
    }
    if !waly_seal::ipc::ressemble_interpreteur(exe) {
        return None;
    }
    let l = ligne.to_ascii_lowercase();
    for (nom, _, marques) in CATALOGUE {
        if marques.iter().any(|m| l.contains(m)) {
            return Some(nom);
        }
    }
    None
}

/// Regroupe les processus reconnus par (agent, programme) et compte, pour
/// chaque programme, ceux qui l'utilisent sans être cet agent. PUR.
pub fn regrouper(procs: &[Processus]) -> Vec<AgentTrouve> {
    let mut out: Vec<AgentTrouve> = Vec::new();
    for p in procs {
        let Some(nom) = reconnaitre(&p.exe, &p.ligne) else { continue };
        match out.iter_mut().find(|a| a.nom == nom && a.exe.eq_ignore_ascii_case(&p.exe)) {
            Some(a) => {
                a.processus += 1;
                a.pids.push(p.pid);
            }
            None => out.push(AgentTrouve {
                nom: nom.to_string(),
                exe: p.exe.clone(),
                partage: waly_seal::ipc::ressemble_interpreteur(&p.exe),
                autres: 0,
                processus: 1,
                pids: vec![p.pid],
                ligne: p.ligne.clone(),
            }),
        }
    }
    for a in &mut out {
        a.autres = procs
            .iter()
            .filter(|p| p.exe.eq_ignore_ascii_case(&a.exe) && reconnaitre(&p.exe, &p.ligne) != Some(a.nom.as_str()))
            .count();
    }
    out.sort_by(|x, y| x.nom.cmp(&y.nom).then(x.exe.to_lowercase().cmp(&y.exe.to_lowercase())));
    out
}

/// Les agents connus qui tournent en ce moment.
pub fn trouver() -> Vec<AgentTrouve> {
    regrouper(&lister())
}

/// Fige des processus : ils restent en mémoire mais n'exécutent plus rien
/// (ni fichier, ni réseau, ni calcul) jusqu'à [`relancer`]. Aucune élévation
/// requise pour les processus de l'utilisateur. Renvoie combien ont été figés.
/// Ne fige jamais le processus courant.
pub fn figer(pids: &[u32]) -> usize {
    pids.iter().filter(|&&p| p != std::process::id() && suspendre(p, true)).count()
}

/// Relance des processus figés par [`figer`].
pub fn relancer(pids: &[u32]) -> usize {
    pids.iter().filter(|&&p| suspendre(p, false)).count()
}

/// Ferme des processus de l'utilisateur (pour relancer l'agent dans
/// l'enclos). Rend combien ont été fermés. Jamais le processus courant.
pub fn fermer(pids: &[u32]) -> usize {
    pids.iter().filter(|&&p| p != std::process::id() && terminer(p)).count()
}

#[cfg(not(windows))]
fn terminer(_pid: u32) -> bool {
    false
}

#[cfg(windows)]
fn terminer(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if h.is_null() {
            return false;
        }
        let ok = TerminateProcess(h, 1) != 0;
        CloseHandle(h);
        ok
    }
}

#[cfg(not(windows))]
fn suspendre(_pid: u32, _figer: bool) -> bool {
    false
}

#[cfg(windows)]
fn suspendre(pid: u32, figer: bool) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SUSPEND_RESUME};
    #[link(name = "ntdll")]
    extern "system" {
        fn NtSuspendProcess(h: HANDLE) -> i32;
        fn NtResumeProcess(h: HANDLE) -> i32;
    }
    unsafe {
        let h = OpenProcess(PROCESS_SUSPEND_RESUME, 0, pid);
        if h.is_null() {
            return false;
        }
        let r = if figer { NtSuspendProcess(h) } else { NtResumeProcess(h) };
        CloseHandle(h);
        r >= 0
    }
}

/// Les processus lisibles de la machine (ceux de l'utilisateur, en pratique).
#[cfg(not(windows))]
pub fn lister() -> Vec<Processus> {
    Vec::new()
}

#[cfg(windows)]
pub fn lister() -> Vec<Processus> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut suite = Process32FirstW(snap, &mut e) != 0;
        while suite {
            let pid = e.th32ProcessID;
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if !h.is_null() {
                let mut buf = [0u16; 1024];
                let mut n = buf.len() as u32;
                if QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut n) != 0 && n > 0 {
                    out.push(Processus {
                        pid,
                        exe: String::from_utf16_lossy(&buf[..n as usize]),
                        ligne: ligne_de_commande(h),
                    });
                }
                CloseHandle(h);
            }
            suite = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    out
}

/// Ligne de commande d'un processus (classe 60 de `NtQueryInformationProcess`,
/// lisible avec le droit « information limitée »). Vide si refusée.
#[cfg(windows)]
unsafe fn ligne_de_commande(h: windows_sys::Win32::Foundation::HANDLE) -> String {
    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryInformationProcess(
            h: windows_sys::Win32::Foundation::HANDLE,
            classe: u32,
            info: *mut core::ffi::c_void,
            taille: u32,
            rendu: *mut u32,
        ) -> i32;
    }
    #[repr(C)]
    struct UnicodeString {
        longueur: u16,
        maximum: u16,
        tampon: *const u16,
    }
    const LIGNE_DE_COMMANDE: u32 = 60;
    // u64 : l'en-tete UNICODE_STRING (qui contient un pointeur) doit etre aligne.
    let mut buf = vec![0u64; 4096];
    let mut rendu = 0u32;
    let r = NtQueryInformationProcess(
        h,
        LIGNE_DE_COMMANDE,
        buf.as_mut_ptr() as *mut _,
        (buf.len() * 8) as u32,
        &mut rendu,
    );
    if r < 0 {
        return String::new();
    }
    let us = &*(buf.as_ptr() as *const UnicodeString);
    if us.tampon.is_null() || us.longueur == 0 {
        return String::new();
    }
    let debut = buf.as_ptr() as usize;
    let fin = debut + buf.len() * 8;
    let p = us.tampon as usize;
    if p < debut || p + us.longueur as usize > fin {
        return String::new();
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(us.tampon, (us.longueur / 2) as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, exe: &str, ligne: &str) -> Processus {
        Processus { pid, exe: exe.into(), ligne: ligne.into() }
    }

    #[test]
    fn reconnait_par_le_nom_du_programme() {
        assert_eq!(reconnaitre(r"C:\Users\x\AppData\Local\Programs\Ollama\ollama.exe", "ollama serve"), Some("Ollama"));
        assert_eq!(reconnaitre(r"C:\Hermes\hermes.exe", ""), Some("Hermes"));
        assert_eq!(reconnaitre(r"C:\apps\OpenClaw\OpenClaw.exe", ""), Some("OpenClaw"));
        assert_eq!(reconnaitre(r"C:\x\claude.exe", ""), Some("Claude"));
        // « ollama app.exe » est la barre d'icônes, pas le serveur qui sort.
        assert_eq!(reconnaitre(r"C:\x\ollama app.exe", ""), None);
    }

    #[test]
    fn reconnait_un_agent_sur_un_moteur_partage_par_sa_ligne_de_commande() {
        let node = r"C:\Program Files\nodejs\node.exe";
        assert_eq!(reconnaitre(node, r"node C:\Users\x\AppData\Roaming\npm\node_modules\openclaw\dist\index.js"), Some("OpenClaw"));
        assert_eq!(reconnaitre(r"C:\Python314\python.exe", "python -m hermes_cli chat"), Some("Hermes"));
        // Un moteur partagé qui fait autre chose n'est pas un agent.
        assert_eq!(reconnaitre(node, "node server.js"), None);
    }

    #[test]
    fn un_editeur_ouvert_sur_un_dossier_d_agent_n_est_pas_l_agent() {
        // La ligne de commande ne compte que pour un moteur partagé.
        assert_eq!(reconnaitre(r"C:\x\Code.exe", r"Code.exe C:\projets\openclaw"), None);
        assert_eq!(reconnaitre(r"C:\Windows\explorer.exe", r"explorer C:\hermes-agent"), None);
    }

    #[test]
    fn waly_n_est_pas_un_autre_agent() {
        assert_eq!(reconnaitre(r"C:\waly\bin\waly-voice.exe", ""), None);
        assert_eq!(reconnaitre(r"C:\x\Waly.exe", "claude"), None);
        assert_eq!(reconnaitre(r"C:\waly\engines\flm\flm.exe", ""), None);
    }

    #[test]
    fn regroupe_et_compte_les_programmes_qui_seraient_coupes_aussi() {
        let node = r"C:\Program Files\nodejs\node.exe";
        let procs = [
            p(1, node, r"node C:\npm\node_modules\openclaw\dist\index.js"),
            p(2, node, r"node C:\npm\node_modules\openclaw\dist\worker.js"),
            p(3, node, "node server.js"),
            p(4, r"C:\NODEJS-autre\node.exe", "node autre.js"),
            p(5, r"C:\Ollama\ollama.exe", "ollama serve"),
            p(6, r"C:\x\notepad.exe", ""),
        ];
        let a = regrouper(&procs);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].nom, "Ollama");
        assert!(!a[0].partage);
        assert_eq!((a[0].autres, a[0].processus), (0, 1));
        assert_eq!(a[1].nom, "OpenClaw");
        assert!(a[1].partage, "node.exe est un moteur partagé");
        assert_eq!(a[1].processus, 2);
        assert_eq!(a[1].pids, vec![1, 2], "figer vise les processus de l'agent, pas le serveur Node voisin");
        assert!(a[1].ligne.ends_with("index.js"), "la ligne relancée dans l'enclos est celle du premier processus");
        assert_eq!(a[0].pids, vec![5]);
        // Le serveur Node sans rapport tourne sur le même programme : coupé aussi.
        assert_eq!(a[1].autres, 1);
    }

    /// Sur le vrai systeme : la liste contient ce processus-ci, avec son chemin
    /// et sa ligne de commande (lecture par `NtQueryInformationProcess`).
    #[cfg(windows)]
    #[test]
    fn lister_voit_ce_processus_et_sa_ligne_de_commande() {
        let moi = std::process::id();
        let procs = lister();
        assert!(procs.len() > 5, "{} processus seulement", procs.len());
        let p = procs.iter().find(|p| p.pid == moi).expect("ce processus doit etre liste");
        let exe = std::env::current_exe().unwrap();
        assert_eq!(base(&p.exe), base(&exe.to_string_lossy()));
        assert!(!p.ligne.is_empty(), "ligne de commande illisible");
        // Et la decouverte complete ne plante pas.
        let _ = trouver();
    }

    /// Sur le vrai système : un processus figé ne fait plus rien, relancé il
    /// reprend. `ping -n 3` dure ~2 s ; figé 4 s, il doit toujours être là.
    #[cfg(windows)]
    #[test]
    fn figer_arrete_vraiment_un_processus_et_relancer_le_reprend() {
        use std::os::windows::process::CommandExt;
        let mut enfant = std::process::Command::new("ping")
            .args(["-n", "3", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .creation_flags(0x0800_0000) // sans fenêtre
            .spawn()
            .expect("ping doit se lancer");
        assert_eq!(figer(&[enfant.id()]), 1);
        std::thread::sleep(std::time::Duration::from_secs(4));
        assert!(enfant.try_wait().unwrap().is_none(), "figé, le processus ne doit pas avoir fini");
        assert_eq!(relancer(&[enfant.id()]), 1);
        let debut = std::time::Instant::now();
        let fin = enfant.wait().unwrap();
        assert!(fin.success());
        assert!(debut.elapsed() < std::time::Duration::from_secs(8));
        // On ne se fige jamais soi-même.
        assert_eq!(figer(&[std::process::id()]), 0);
    }

    #[test]
    fn rien_a_trouver_rend_une_liste_vide() {
        assert!(regrouper(&[]).is_empty());
        assert!(regrouper(&[p(1, r"C:\x\notepad.exe", "")]).is_empty());
    }
}
