//! Le regard du service : VOIR ce que des programmes touchent (Garde, étape 3).
//!
//! Une session de suivi d'événements du noyau (ETW) en temps réel, sur trois
//! sources : fichiers, lancements de programmes, connexions. Éteinte par
//! défaut et à chaque démarrage du service. Deux portées : les programmes
//! qu'on lui désigne (et ce qu'ils lancent), ou toute la machine.
//!
//! On retient des CHEMINS, des noms de programmes et des adresses — jamais un
//! contenu. Rien ne quitte la machine : l'app lit le carnet par le canal du
//! service. Le tri (bruit, regroupement) est dans `tri.rs`, pur et testé.
//!
//! Faisabilité mesurée au banc (`lab/garde-banc`) : ~865 événements/s pour
//! tout le système, les quatre gestes d'un faux agent retrouvés.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, ERROR_SUCCESS};
use windows_sys::Win32::Storage::FileSystem::QueryDosDeviceW;
use windows_sys::Win32::System::Diagnostics::Etw::{
    CloseTrace, ControlTraceW, EnableTraceEx2, OpenTraceW, ProcessTrace, StartTraceW, TdhGetProperty,
    TdhGetPropertySize, CONTROLTRACE_HANDLE, EVENT_CONTROL_CODE_ENABLE_PROVIDER, EVENT_RECORD,
    EVENT_TRACE_CONTROL_STOP, EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES, EVENT_TRACE_REAL_TIME_MODE,
    PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_REAL_TIME, PROPERTY_DATA_DESCRIPTOR,
    WNODE_FLAG_TRACED_GUID,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::ipc::{Observation, REGARD_AGENTS, REGARD_RIEN, REGARD_TOUT};
use crate::tri;

const SESSION: &str = "WalyGardeRegard";
const FICHIERS: GUID = GUID::from_u128(0xedd08927_9cc4_4e65_b970_c2560fb5c289);
const PROCESSUS: GUID = GUID::from_u128(0x22fb2cd6_0e7b_422b_a0c7_2fad1fd0e716);
const RESEAU: GUID = GUID::from_u128(0x7dd42a49_5329_4832_8dfd_43d979153a88);
/// Fichiers : noms, créations, écritures, suppressions, renommages, nouveaux.
const MOTS_FICHIERS: u64 = 0x1E90;
const MOTS_PROCESSUS: u64 = 0x10;
const MOTS_RESEAU: u64 = 0x30;

struct Etat {
    mode: String,
    /// programmes désignés (chemins en minuscules)
    exes: Vec<String>,
    /// processus suivis en mode « agents » : pid -> programme de l'agent
    suivis: HashMap<u32, String>,
    /// processus déjà examinés et hors sujet (mode « agents »)
    ecartes: HashSet<u32>,
    /// pid -> chemin du programme (mode « tout »)
    images: HashMap<u32, String>,
    /// objet fichier du noyau -> chemin (pour nommer les écritures)
    ouverts: HashMap<u64, String>,
    volumes: Vec<(String, String)>,
    carnet: tri::Carnet,
    session: u64,
    moi: u32,
}

fn etat() -> &'static Mutex<Etat> {
    static E: OnceLock<Mutex<Etat>> = OnceLock::new();
    E.get_or_init(|| {
        Mutex::new(Etat {
            mode: REGARD_RIEN.into(),
            exes: Vec::new(),
            suivis: HashMap::new(),
            ecartes: HashSet::new(),
            images: HashMap::new(),
            ouverts: HashMap::new(),
            volumes: volumes(),
            carnet: tri::Carnet::nouveau(4000),
            session: 0,
            moi: std::process::id(),
        })
    })
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Table (périphérique du noyau, lettre de lecteur).
fn volumes() -> Vec<(String, String)> {
    let mut v = Vec::new();
    for l in b'A'..=b'Z' {
        let lettre = format!("{}:", l as char);
        let mut buf = [0u16; 512];
        let n = unsafe { QueryDosDeviceW(wide(&lettre).as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
        if n > 0 {
            let fin = buf.iter().position(|&c| c == 0).unwrap_or(0);
            let p = String::from_utf16_lossy(&buf[..fin]);
            if !p.is_empty() {
                v.push((p, lettre));
            }
        }
    }
    v
}

fn image(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut n);
        CloseHandle(h);
        (ok != 0 && n > 0).then(|| String::from_utf16_lossy(&buf[..n as usize]))
    }
}

/// Bloc de propriétés d'une session (structure + place pour le nom).
fn proprietes() -> Vec<u8> {
    let nom = wide(SESSION);
    let base = std::mem::size_of::<EVENT_TRACE_PROPERTIES>();
    let total = base + nom.len() * 2 + 2048;
    let mut buf = vec![0u8; total];
    unsafe {
        let p = buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
        (*p).Wnode.BufferSize = total as u32;
        (*p).Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        (*p).Wnode.ClientContext = 1;
        (*p).LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        (*p).BufferSize = 64;
        (*p).MinimumBuffers = 16;
        (*p).MaximumBuffers = 64;
        (*p).FlushTimer = 1;
        (*p).LoggerNameOffset = base as u32;
    }
    buf
}

fn arreter_session() {
    let nom = wide(SESSION);
    let mut p = proprietes();
    unsafe {
        ControlTraceW(CONTROLTRACE_HANDLE { Value: 0 }, nom.as_ptr(), p.as_mut_ptr() as *mut _, EVENT_TRACE_CONTROL_STOP);
    }
}

fn demarrer_session() -> Result<u64, String> {
    let nom = wide(SESSION);
    let mut h = CONTROLTRACE_HANDLE { Value: 0 };
    let mut r = unsafe { StartTraceW(&mut h, nom.as_ptr(), proprietes().as_mut_ptr() as *mut _) };
    if r == ERROR_ALREADY_EXISTS {
        arreter_session();
        r = unsafe { StartTraceW(&mut h, nom.as_ptr(), proprietes().as_mut_ptr() as *mut _) };
    }
    if r != ERROR_SUCCESS {
        return Err(format!("ouverture de la session de suivi refusee (code {r}) : le service doit tourner en SYSTEM ou en administrateur"));
    }
    for (guid, mots) in [(FICHIERS, MOTS_FICHIERS), (PROCESSUS, MOTS_PROCESSUS), (RESEAU, MOTS_RESEAU)] {
        let r = unsafe { EnableTraceEx2(h, &guid, EVENT_CONTROL_CODE_ENABLE_PROVIDER, 4, mots, 0, 0, std::ptr::null()) };
        if r != ERROR_SUCCESS {
            arreter_session();
            return Err(format!("activation d'une source du noyau refusee (code {r})"));
        }
    }
    Ok(h.Value)
}

/// Règle ce que le service observe. « rien » éteint tout.
pub fn regler(mode: &str, exes: &[String]) -> Result<(), String> {
    arreter_session();
    {
        let mut e = etat().lock().map_err(|_| "etat du regard indisponible")?;
        e.mode = REGARD_RIEN.into();
        e.exes.clear();
        e.suivis.clear();
        e.ecartes.clear();
        e.images.clear();
        e.ouverts.clear();
        e.session = 0;
        if mode == REGARD_RIEN {
            return Ok(());
        }
        e.volumes = volumes();
    }
    if mode != REGARD_TOUT && mode != REGARD_AGENTS {
        return Err("mode de surveillance inconnu".into());
    }
    let session = demarrer_session()?;
    {
        let mut e = etat().lock().map_err(|_| "etat du regard indisponible")?;
        e.mode = mode.to_string();
        e.exes = exes.iter().map(|x| x.to_lowercase()).collect();
        e.session = session;
    }
    std::thread::Builder::new()
        .name("waly-regard".into())
        .spawn(lire_en_continu)
        .map_err(|e| format!("fil de lecture : {e}"))?;
    Ok(())
}

/// (mode, programmes désignés, observations neuves, dernier numéro).
pub fn lire(apres: u64) -> (String, Vec<String>, Vec<Observation>, u64) {
    match etat().lock() {
        Ok(e) => (e.mode.clone(), e.exes.clone(), e.carnet.depuis(apres, 500), e.carnet.dernier()),
        Err(_) => (REGARD_RIEN.into(), Vec::new(), Vec::new(), 0),
    }
}

/// Le fil de lecture : bloque dans `ProcessTrace` tant que la session vit.
fn lire_en_continu() {
    let mut nom = wide(SESSION);
    unsafe {
        let mut log: EVENT_TRACE_LOGFILEW = std::mem::zeroed();
        log.LoggerName = nom.as_mut_ptr();
        log.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        log.Anonymous2.EventRecordCallback = Some(rappel);
        let h = OpenTraceW(&mut log);
        if h.Value == u64::MAX || h.Value == 0xFFFF_FFFF {
            return;
        }
        ProcessTrace(&h, 1, std::ptr::null(), std::ptr::null());
        CloseTrace(h);
    }
}

/// Lit une propriété d'un événement par son nom.
unsafe fn propriete(rec: *const EVENT_RECORD, nom: &str) -> Option<Vec<u8>> {
    let n = wide(nom);
    let d = PROPERTY_DATA_DESCRIPTOR { PropertyName: n.as_ptr() as u64, ArrayIndex: u32::MAX, Reserved: 0 };
    let mut taille = 0u32;
    if TdhGetPropertySize(rec, 0, std::ptr::null(), 1, &d, &mut taille) != ERROR_SUCCESS || taille == 0 || taille > 65_536 {
        return None;
    }
    let mut buf = vec![0u8; taille as usize];
    (TdhGetProperty(rec, 0, std::ptr::null(), 1, &d, taille, buf.as_mut_ptr()) == ERROR_SUCCESS).then_some(buf)
}

unsafe fn prop_u32(rec: *const EVENT_RECORD, nom: &str) -> Option<u32> {
    let b = propriete(rec, nom)?;
    (b.len() >= 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

unsafe fn prop_u16(rec: *const EVENT_RECORD, nom: &str) -> Option<u16> {
    let b = propriete(rec, nom)?;
    (b.len() >= 2).then(|| u16::from_le_bytes([b[0], b[1]]))
}

unsafe fn prop_u64(rec: *const EVENT_RECORD, nom: &str) -> Option<u64> {
    let b = propriete(rec, nom)?;
    match b.len() {
        8.. => Some(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])),
        4.. => Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64),
        _ => None,
    }
}

unsafe fn prop_texte(rec: *const EVENT_RECORD, nom: &str) -> Option<String> {
    let b = propriete(rec, nom)?;
    let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    (!u.is_empty()).then(|| String::from_utf16_lossy(&u))
}

fn maintenant() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Etat {
    /// Ce processus nous intéresse-t-il ? Rend le programme auquel attribuer
    /// le geste (en mode « agents » : celui de l'agent, même pour un enfant).
    fn programme(&mut self, pid: u32) -> Option<String> {
        if pid == self.moi || pid == 0 {
            return None;
        }
        if self.mode == REGARD_TOUT {
            if let Some(x) = self.images.get(&pid) {
                return Some(x.clone());
            }
            let x = image(pid).unwrap_or_else(|| if pid == 4 { "System".into() } else { format!("processus {pid}") });
            if self.images.len() > 8000 {
                self.images.clear();
            }
            self.images.insert(pid, x.clone());
            return Some(x);
        }
        if let Some(x) = self.suivis.get(&pid) {
            return Some(x.clone());
        }
        if self.ecartes.contains(&pid) {
            return None;
        }
        match image(pid) {
            Some(x) if self.exes.contains(&x.to_lowercase()) => {
                self.suivis.insert(pid, x.clone());
                Some(x)
            }
            _ => {
                if self.ecartes.len() > 20_000 {
                    self.ecartes.clear();
                }
                self.ecartes.insert(pid);
                None
            }
        }
    }

    fn fichier(&mut self, exe: &str, pid: u32, genre: &str, brut: &str) {
        let chemin = tri::chemin_dos(brut, &self.volumes);
        if tri::est_bruit(&chemin) || tri::chez_lui(&chemin, exe) {
            return;
        }
        // Mesure au banc : des dossiers arrivent encore sans que le noyau le
        // dise. Pour un nom sans extension, on regarde sur le disque.
        if genre == "ouvert" && !tri::a_une_extension(&chemin) && std::path::Path::new(&chemin).is_dir() {
            return;
        }
        self.carnet.noter(maintenant(), exe, pid, genre, &chemin);
    }
}

unsafe extern "system" fn rappel(rec: *mut EVENT_RECORD) {
    if rec.is_null() {
        return;
    }
    let h = &(*rec).EventHeader;
    let (id, pid, source) = (h.EventDescriptor.Id, h.ProcessId, h.ProviderId.data1);
    let Ok(mut e) = etat().lock() else { return };
    if e.mode == REGARD_RIEN {
        return;
    }
    if source == FICHIERS.data1 {
        // Le processus « System » (4) vide les caches a la place des
        // programmes : lui attribuer leurs ecritures serait faux.
        if pid == 4 {
            return;
        }
        let Some(exe) = e.programme(pid) else { return };
        match id {
            // Ouverture : on retient le nom pour les écritures qui suivront.
            12 => {
                if let Some(nom) = prop_texte(rec, "FileName") {
                    if let Some(objet) = prop_u64(rec, "FileObject") {
                        if e.ouverts.len() > 30_000 {
                            e.ouverts.clear();
                        }
                        e.ouverts.insert(objet, nom.clone());
                    }
                    if tri::ouverture_de_fichier(&nom, prop_u32(rec, "CreateOptions").unwrap_or(0)) {
                        e.fichier(&exe, pid, "ouvert", &nom);
                    }
                }
            }
            30 => {
                if let Some(nom) = prop_texte(rec, "FileName") {
                    e.fichier(&exe, pid, "cree", &nom);
                }
            }
            16 => {
                if let Some(nom) = prop_u64(rec, "FileObject").and_then(|o| e.ouverts.get(&o).cloned()) {
                    e.fichier(&exe, pid, "ecrit", &nom);
                }
            }
            26 | 27 => {
                if let Some(nom) = prop_texte(rec, "FilePath").or_else(|| prop_texte(rec, "FileName")) {
                    e.fichier(&exe, pid, if id == 26 { "supprime" } else { "renomme" }, &nom);
                }
            }
            _ => {}
        }
    } else if source == PROCESSUS.data1 {
        match id {
            // Lancement : l'en-tête porte le parent.
            1 => {
                let neuf = prop_u32(rec, "ProcessID").unwrap_or(0);
                let parent = prop_u32(rec, "ParentProcessID").unwrap_or(pid);
                let Some(exe) = e.programme(parent) else { return };
                if e.mode == REGARD_AGENTS && neuf != 0 {
                    e.ecartes.remove(&neuf);
                    e.suivis.insert(neuf, exe.clone()); // l'enfant est suivi au nom de l'agent
                }
                if let Some(img) = prop_texte(rec, "ImageName") {
                    let chemin = tri::chemin_dos(&img, &e.volumes);
                    if e.mode == REGARD_TOUT && neuf != 0 {
                        e.images.insert(neuf, chemin.clone());
                    }
                    if tri::lancement_sans_interet(&chemin) {
                        return;
                    }
                    e.carnet.noter(maintenant(), &exe, parent, "lance", &chemin);
                }
            }
            2 => {
                if let Some(mort) = prop_u32(rec, "ProcessID") {
                    e.suivis.remove(&mort);
                    e.ecartes.remove(&mort);
                    e.images.remove(&mort);
                }
            }
            _ => {}
        }
    } else if source == RESEAU.data1 && (id == 12 || id == 28) {
        // Connexion sortante (12 : IPv4, 28 : IPv6). Le PID est dans les données.
        let qui = prop_u32(rec, "PID").unwrap_or(pid);
        let Some(exe) = e.programme(qui) else { return };
        let port = prop_u16(rec, "dport").map(tri::port).unwrap_or(0);
        let adresse = if id == 12 {
            prop_u32(rec, "daddr").map(tri::adresse_v4)
        } else {
            propriete(rec, "daddr").filter(|b| b.len() >= 16).map(|b| {
                let mut o = [0u8; 16];
                o.copy_from_slice(&b[..16]);
                std::net::Ipv6Addr::from(o).to_string()
            })
        };
        if let Some(a) = adresse {
            if !tri::est_local(&a) {
                e.carnet.noter(maintenant(), &exe, qui, "connecte", &format!("{a}:{port}"));
            }
        }
    }
}
