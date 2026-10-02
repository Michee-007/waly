//! SCM (install/désinstall/démarrage) + boucle de service + serveur de pipe.
//!
//! Le service tourne en **LocalSystem** : le privilège WFP reste hors du compte
//! utilisateur. Le pipe est ACLé (SYSTEM+Admins plein, utilisateurs
//! authentifiés lecture/écriture) et n'accepte que le protocole étroit
//! `ipc::Requete` — jamais de filtre brut.

#![allow(non_snake_case)]

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows_sys::Win32::Security::{
    GetTokenInformation, RevertToSelf, TokenElevation, SECURITY_ATTRIBUTES, TOKEN_ELEVATION,
    TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::{
    FlushFileBuffers, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::*;
use windows_sys::Win32::System::Services::*;
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentThread, OpenThreadToken, SetEvent, WaitForMultipleObjects,
    WaitForSingleObject, INFINITE,
};
use windows_sys::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};

use crate::ipc::{Reponse, Requete, Succes, PIPE_NAME, SERVICE_NAME};
use crate::wfp::Sealer;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// --- État global (le service_main C ABI n'a pas de contexte) ---------------

static STATUS_HANDLE: AtomicIsize = AtomicIsize::new(0);
static STOP_EVENT: AtomicIsize = AtomicIsize::new(0);
static SEALER: OnceLock<Mutex<Sealer>> = OnceLock::new();

fn sealer() -> &'static Mutex<Sealer> {
    SEALER.get_or_init(|| {
        Mutex::new(Sealer::new().unwrap_or_else(|e| {
            // Sans moteur WFP, le service n'a pas de raison d'être.
            panic!("Sealer::new: {e}");
        }))
    })
}

// --- Install / désinstall (élevé une fois) ---------------------------------

/// Dossier machine du service, **non modifiable par un utilisateur standard**
/// (Program Files). CRITIQUE pour la sécurité : un service SYSTEM ne doit
/// JAMAIS tourner depuis un chemin où un compte utilisateur a le droit
/// d'écriture (%LOCALAPPDATA%), sinon un malware user-level remplace l'exe que
/// SYSTEM relancera au boot = escalade de privilège.
fn dossier_machine() -> String {
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
    format!(r"{pf}\Waly")
}

/// Chemin cible du service dans le dossier machine.
pub fn chemin_machine() -> String {
    format!(r"{}\waly-seal-svc.exe", dossier_machine())
}

/// Installe le service DEPUIS le dossier machine : copie l'exe courant vers
/// Program Files (admin-only), enregistre le service avec CE chemin, démarre.
/// Appelé élevé une fois par l'installeur (`waly-seal-svc setup`). Idempotent.
pub fn setup(exe_courant: &str) -> Result<(), String> {
    let dossier = dossier_machine();
    let cible = chemin_machine();
    std::fs::create_dir_all(&dossier).map_err(|e| format!("create_dir {dossier}: {e}"))?;
    // Si le service tourne déjà depuis la cible, on ne peut pas écraser l'exe
    // en cours : arrêter+supprimer d'abord (idempotence des mises à jour).
    if std::path::Path::new(&cible).exists() {
        let _ = uninstall();
        std::thread::sleep(std::time::Duration::from_millis(600));
    }
    if !paths_egaux(exe_courant, &cible) {
        std::fs::copy(exe_courant, &cible).map_err(|e| format!("copy vers {cible}: {e}"))?;
    }
    install(&cible)?;
    start()
}

fn paths_egaux(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Enregistre le service (SYSTEM, démarrage à la demande). Élévation requise.
pub fn install(exe: &str) -> Result<(), String> {
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_ALL_ACCESS);
        if scm.is_null() {
            return Err(format!("OpenSCManagerW: {}", derniere()));
        }
        let nom = wide(SERVICE_NAME);
        let bin = wide(&format!("\"{exe}\" run"));
        let h = CreateServiceW(
            scm,
            nom.as_ptr(),
            nom.as_ptr(),
            SERVICE_ALL_ACCESS,
            SERVICE_WIN32_OWN_PROCESS,
            // AUTO_START : scellé par défaut = le périmètre doit se refermer
            // au boot, avant tout (ADR 2026-07-21), pas seulement quand l'app
            // le demande.
            SERVICE_AUTO_START,
            SERVICE_ERROR_NORMAL,
            bin.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(), // LocalSystem
            std::ptr::null(),
        );
        // Récupération auto (fail-closed complet) : le sceau NON-dynamique fait
        // que les filtres PERSISTENT si le service crashe (rien ne fuit) ; ici
        // on demande en plus au SCM de RELANCER le service en cas de crash, qui
        // se ré-scelle (idempotent) → réseau bloqué pendant la panne PUIS
        // journal/rejoin de retour en quelques secondes.
        let res = if h.is_null() {
            let e = derniere();
            if e == ERROR_SERVICE_EXISTS {
                // Déjà installé : ouvrir pour (re)poser la récupération.
                let hx = OpenServiceW(scm, nom.as_ptr(), SERVICE_ALL_ACCESS);
                if !hx.is_null() {
                    set_recovery(hx);
                    CloseServiceHandle(hx);
                }
                Ok(())
            } else {
                Err(format!("CreateServiceW: {e}"))
            }
        } else {
            set_recovery(h);
            CloseServiceHandle(h);
            Ok(())
        };
        CloseServiceHandle(scm);
        res
    }
}

/// Configure le SCM pour RELANCER le service en cas de crash (délai court, puis
/// stable). Best-effort : un échec ne compromet pas le fail-closed (les filtres
/// persistent de toute façon).
unsafe fn set_recovery(h: SC_HANDLE) {
    // 3 actions : redémarrer après 5 s, 5 s, puis 5 s ; compteur remis à zéro
    // après un jour sans panne.
    let mut actions = [
        SC_ACTION { Type: SC_ACTION_RESTART, Delay: 5_000 },
        SC_ACTION { Type: SC_ACTION_RESTART, Delay: 5_000 },
        SC_ACTION { Type: SC_ACTION_RESTART, Delay: 5_000 },
    ];
    let mut fa: SERVICE_FAILURE_ACTIONSW = std::mem::zeroed();
    fa.dwResetPeriod = 86_400; // secondes
    fa.cActions = actions.len() as u32;
    fa.lpsaActions = actions.as_mut_ptr();
    ChangeServiceConfig2W(
        h,
        SERVICE_CONFIG_FAILURE_ACTIONS,
        &mut fa as *mut _ as *mut core::ffi::c_void,
    );
}

/// Démarre le service (élévation requise ; sinon le fait le SCM à la demande).
pub fn start() -> Result<(), String> {
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_ALL_ACCESS);
        if scm.is_null() {
            return Err(format!("OpenSCManagerW: {}", derniere()));
        }
        let nom = wide(SERVICE_NAME);
        let h = OpenServiceW(scm, nom.as_ptr(), SERVICE_ALL_ACCESS);
        let res = if h.is_null() {
            Err(format!("OpenServiceW: {}", derniere()))
        } else {
            let ok = StartServiceW(h, 0, std::ptr::null());
            let r = if ok == 0 {
                let e = derniere();
                if e == ERROR_SERVICE_ALREADY_RUNNING {
                    Ok(())
                } else {
                    Err(format!("StartServiceW: {e}"))
                }
            } else {
                Ok(())
            };
            CloseServiceHandle(h);
            r
        };
        CloseServiceHandle(scm);
        res
    }
}

/// Arrête et supprime le service. Élévation requise.
pub fn uninstall() -> Result<(), String> {
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_ALL_ACCESS);
        if scm.is_null() {
            return Err(format!("OpenSCManagerW: {}", derniere()));
        }
        let nom = wide(SERVICE_NAME);
        let h = OpenServiceW(scm, nom.as_ptr(), SERVICE_ALL_ACCESS);
        let res = if h.is_null() {
            let e = derniere();
            if e == ERROR_SERVICE_DOES_NOT_EXIST {
                Ok(())
            } else {
                Err(format!("OpenServiceW: {e}"))
            }
        } else {
            let mut st: SERVICE_STATUS = std::mem::zeroed();
            ControlService(h, SERVICE_CONTROL_STOP, &mut st);
            let ok = DeleteService(h);
            CloseServiceHandle(h);
            if ok == 0 {
                Err(format!("DeleteService: {}", derniere()))
            } else {
                Ok(())
            }
        };
        CloseServiceHandle(scm);
        // Nettoyage best-effort de la copie machine (après l'arrêt du service,
        // qui libère l'exe). Ignore l'échec (exe encore verrouillé, absent).
        let _ = std::fs::remove_file(chemin_machine());
        res
    }
}

// --- Boucle de service (SCM) -----------------------------------------------

/// Scelle le périmètre par défaut au démarrage (best-effort : en mode dégradé
/// non-privilégié, `seal` renvoie une erreur, ignorée — l'état est lisible via
/// `Etat.privilegie`).
fn sceller_perimetre_demarrage() {
    if let Ok(mut g) = sealer().lock() {
        let _ = g.sceller_perimetre();
    }
}

fn set_status(state: u32, accepted: u32) {
    let h = STATUS_HANDLE.load(Ordering::SeqCst);
    if h == 0 {
        return;
    }
    unsafe {
        let mut st: SERVICE_STATUS = std::mem::zeroed();
        st.dwServiceType = SERVICE_WIN32_OWN_PROCESS;
        st.dwCurrentState = state;
        st.dwControlsAccepted = accepted;
        SetServiceStatus(h as SERVICE_STATUS_HANDLE, &mut st);
    }
}

unsafe extern "system" fn handler(control: u32) {
    match control {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
            set_status(SERVICE_STOP_PENDING, 0);
            let e = STOP_EVENT.load(Ordering::SeqCst);
            if e != 0 {
                SetEvent(e as HANDLE);
            }
        }
        SERVICE_CONTROL_INTERROGATE => {
            set_status(SERVICE_RUNNING, SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN);
        }
        _ => {}
    }
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut windows_sys::core::PWSTR) {
    let nom = wide(SERVICE_NAME);
    let h = RegisterServiceCtrlHandlerW(nom.as_ptr(), Some(handler));
    if h.is_null() {
        return;
    }
    STATUS_HANDLE.store(h as isize, Ordering::SeqCst);
    set_status(SERVICE_START_PENDING, 0);

    // Évènement d'arrêt (manuel).
    let stop = CreateEventW(std::ptr::null(), 1, 0, std::ptr::null());
    STOP_EVENT.store(stop as isize, Ordering::SeqCst);

    // Init du moteur PUIS scellé du périmètre par défaut (ADR 2026-07-21 :
    // scellé permanent, dès le démarrage — pas de bouton).
    sceller_perimetre_demarrage();
    set_status(SERVICE_RUNNING, SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN);

    boucle_pipe(stop);

    set_status(SERVICE_STOPPED, 0);
}

/// Lance le dispatcher SCM (appelé par `waly-seal-svc run`).
pub fn run_service() -> Result<(), String> {
    unsafe {
        let nom = wide(SERVICE_NAME);
        let table = [
            SERVICE_TABLE_ENTRYW {
                lpServiceName: nom.as_ptr() as *mut u16,
                lpServiceProc: Some(service_main),
            },
            SERVICE_TABLE_ENTRYW {
                lpServiceName: std::ptr::null_mut(),
                lpServiceProc: None,
            },
        ];
        if StartServiceCtrlDispatcherW(table.as_ptr()) == 0 {
            return Err(format!("StartServiceCtrlDispatcherW: {}", derniere()));
        }
        Ok(())
    }
}

/// Mode console (debug hors SCM) : sert le pipe jusqu'à Ctrl-C.
pub fn run_console() -> Result<(), String> {
    unsafe {
        let stop = CreateEventW(std::ptr::null(), 1, 0, std::ptr::null());
        STOP_EVENT.store(stop as isize, Ordering::SeqCst);
        sceller_perimetre_demarrage();
        eprintln!("waly-seal (console) : pipe {PIPE_NAME} — Ctrl-C pour quitter");
        boucle_pipe(stop);
    }
    Ok(())
}

// --- Serveur de pipe --------------------------------------------------------

/// Descripteur de sécurité du pipe : SYSTEM + Admins plein contrôle ;
/// utilisateurs authentifiés lecture/écriture (pour sceller leur session).
fn security_attributes() -> Option<(SECURITY_ATTRIBUTES, *mut core::ffi::c_void)> {
    let sddl = wide("D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;AU)");
    unsafe {
        let mut psd: *mut core::ffi::c_void = std::ptr::null_mut();
        let ok = ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1, // SDDL_REVISION_1
            &mut psd,
            std::ptr::null_mut(),
        );
        if ok == 0 {
            return None;
        }
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: psd,
            bInheritHandle: 0,
        };
        Some((sa, psd))
    }
}

fn boucle_pipe(stop: HANDLE) {
    let name = wide(PIPE_NAME);
    let sa = security_attributes();
    loop {
        // Arrêt demandé ?
        if unsafe { WaitForSingleObject(stop, 0) } == WAIT_OBJECT_0 {
            break;
        }
        let pipe = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                sa.as_ref().map(|(s, _)| s as *const _).unwrap_or(std::ptr::null()),
            )
        };
        if pipe == INVALID_HANDLE_VALUE {
            break;
        }
        if !accept(pipe, stop) {
            unsafe { CloseHandle(pipe) };
            break; // stop demandé pendant l'attente
        }
        servir(pipe, stop);
        unsafe {
            DisconnectNamedPipe(pipe);
            CloseHandle(pipe);
        }
    }
    if let Some((_, psd)) = sa {
        unsafe { LocalFree(psd) };
    }
}

/// Attend une connexion (overlapped) OU l'arrêt. `true` si connecté.
fn accept(pipe: HANDLE, stop: HANDLE) -> bool {
    unsafe {
        let ev = CreateEventW(std::ptr::null(), 1, 0, std::ptr::null());
        let mut ov: OVERLAPPED = std::mem::zeroed();
        ov.hEvent = ev;
        let r = ConnectNamedPipe(pipe, &mut ov);
        let connecte = if r != 0 {
            true
        } else {
            match derniere() {
                ERROR_PIPE_CONNECTED => true,
                ERROR_IO_PENDING => {
                    let handles = [ev, stop];
                    let w = WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE);
                    if w == WAIT_OBJECT_0 {
                        let mut n = 0u32;
                        GetOverlappedResult(pipe, &mut ov, &mut n, 0) != 0
                    } else {
                        false // stop
                    }
                }
                _ => false,
            }
        };
        CloseHandle(ev);
        connecte
    }
}

/// Lit une requête (ligne JSON), la traite, écrit la réponse. Capture d'abord
/// l'élévation du client (chantier C : sceller un agent TIERS exige un client
/// élevé — ADR 2026-09-16).
fn servir(pipe: HANDLE, _stop: HANDLE) {
    // Lire la requête AVANT d'impersonner : sur un pipe en mode octet,
    // `ImpersonateNamedPipeClient` échoue tant que le client n'a rien écrit
    // (on lirait « non élevé » à tort → faux refus). L'impersonation reflète le
    // token du client de la connexion, pas d'un message précis.
    let Some(ligne) = lire_ligne(pipe, 64 * 1024, 3000) else {
        return;
    };
    let eleve = unsafe { client_est_eleve(pipe) };
    let rep = traiter(&ligne, eleve);
    let mut s = serde_json::to_vec(&rep).unwrap_or_default();
    s.push(b'\n');
    ecrire(pipe, &s);
}

/// Le client du pipe est-il élevé (token élevé) ? On impersonne le client,
/// ouvre son token de thread et lit `TokenElevation`, puis on revient à SYSTEM.
/// Toute erreur = « pas élevé » (fail-safe : on refuse plutôt qu'on autorise).
unsafe fn client_est_eleve(pipe: HANDLE) -> bool {
    if ImpersonateNamedPipeClient(pipe) == 0 {
        return false;
    }
    let mut token: HANDLE = std::ptr::null_mut();
    // OpenAsSelf=TRUE : l'ouverture se fait dans le contexte du process (SYSTEM),
    // pas dans celui du client impersonné (qui peut être restreint).
    let ouvert = OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token);
    let eleve = if ouvert != 0 {
        let mut elev: TOKEN_ELEVATION = std::mem::zeroed();
        let mut ret = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elev as *mut _ as *mut core::ffi::c_void,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret,
        );
        CloseHandle(token);
        ok != 0 && elev.TokenIsElevated != 0
    } else {
        false
    };
    RevertToSelf();
    eleve
}

fn traiter(ligne: &[u8], eleve: bool) -> Reponse {
    let req: Requete = match serde_json::from_slice(ligne) {
        Ok(r) => r,
        Err(e) => return Reponse::err(format!("requete illisible: {e}")),
    };
    // La politique d'abord (pure, testée : `ipc::autoriser`) — le moteur
    // n'est jamais touché par une requête qu'elle refuse.
    if let Err(raison) = crate::ipc::autoriser(&req, eleve) {
        return Reponse::err(raison);
    }
    let m = sealer();
    let mut g = match m.lock() {
        Ok(g) => g,
        Err(_) => return Reponse::err("moteur indisponible"),
    };
    match req {
        Requete::Ping => Reponse::Ok(Succes::Fait { fait: true }),
        Requete::Etat => Reponse::Ok(Succes::Etat {
            sessions: g.sessions(),
            version: crate::VERSION.to_string(),
            privilegie: g.privilegie(),
        }),
        Requete::Sceller { session, exes } => {
            let vus = exes.len();
            match g.seal(session, &exes) {
                Ok(n) => Reponse::Ok(Succes::Scelle { filtres: n, exes_vus: vus }),
                Err(e) => Reponse::err(e),
            }
        }
        Requete::Rejoindre { exe } => match g.rejoindre(&exe) {
            Ok(n) => Reponse::Ok(Succes::Scelle { filtres: n, exes_vus: 1 }),
            Err(e) => Reponse::err(e),
        },
        Requete::Desceller { session } => {
            g.unseal(session);
            Reponse::Ok(Succes::Fait { fait: true })
        }
        Requete::Journal { session } => Reponse::Ok(Succes::Journal { drops: g.drops(session) }),
    }
}

/// Lit jusqu'à '\n' (ou `max` octets, ou `timeout_ms`), en overlapped.
fn lire_ligne(pipe: HANDLE, max: usize, timeout_ms: u32) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    unsafe {
        let ev = CreateEventW(std::ptr::null(), 1, 0, std::ptr::null());
        let res = loop {
            let mut ov: OVERLAPPED = std::mem::zeroed();
            ov.hEvent = ev;
            let r = ReadFile(pipe, tmp.as_mut_ptr(), tmp.len() as u32, std::ptr::null_mut(), &mut ov);
            let mut n = 0u32;
            if r == 0 {
                if derniere() != ERROR_IO_PENDING {
                    break None;
                }
                if WaitForSingleObject(ev, timeout_ms) != WAIT_OBJECT_0 {
                    windows_sys::Win32::System::IO::CancelIo(pipe);
                    break None;
                }
                if GetOverlappedResult(pipe, &mut ov, &mut n, 0) == 0 {
                    break None;
                }
            } else {
                GetOverlappedResult(pipe, &mut ov, &mut n, 0);
            }
            if n == 0 {
                break None;
            }
            for &b in &tmp[..n as usize] {
                if b == b'\n' {
                    break;
                }
                buf.push(b);
            }
            if tmp[..n as usize].contains(&b'\n') || buf.len() >= max {
                break Some(std::mem::take(&mut buf));
            }
        };
        CloseHandle(ev);
        res
    }
}

fn ecrire(pipe: HANDLE, data: &[u8]) {
    unsafe {
        let ev = CreateEventW(std::ptr::null(), 1, 0, std::ptr::null());
        let mut ov: OVERLAPPED = std::mem::zeroed();
        ov.hEvent = ev;
        let mut n = 0u32;
        let r = WriteFile(pipe, data.as_ptr(), data.len() as u32, std::ptr::null_mut(), &mut ov);
        if r == 0 && derniere() == ERROR_IO_PENDING {
            WaitForSingleObject(ev, 3000);
            GetOverlappedResult(pipe, &mut ov, &mut n, 0);
        }
        FlushFileBuffers(pipe);
        CloseHandle(ev);
    }
}

fn derniere() -> u32 {
    unsafe { GetLastError() }
}
