//! Le moteur WFP du sceau — porté du banc `lab/huisclos-banc` (3 gates verts),
//! généralisé « par session ».
//!
//! Modèle de durée de vie choisi pour le SECRET :
//! - session WFP **non-dynamique** → les filtres survivent au crash du service
//!   (BFE les garde) : **fail-closed**, rien ne fuit pendant une panne ;
//! - filtres/sous-couche **non-persistants** → disparaissent au reboot :
//!   l'état de scellé ne survit pas à un redémarrage (cohérent, le scellé est
//!   par session Waly, qui n'existe plus après reboot).
//! - au démarrage du service : on **purge** notre sous-couche (aucune session
//!   Waly connue en travers d'un redémarrage de service) → repart propre ;
//!   le desktop ré-assied les sceaux actifs qu'il connaît.
//!
//! Filtres possédés par SYSTEM (le service tourne en SYSTEM) → un attaquant
//! user-level ne peut ni les retirer ni les percer.

#![allow(non_snake_case)]

use std::collections::HashMap;

use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::NetworkManagement::WindowsFilteringPlatform::*;

/// Sous-couche dédiée « Waly Huis clos », poids fort (GUID fixe produit).
const WALY_SUBLAYER: GUID = GUID::from_u128(0x5ea1_c105_2026_0720_a11e_c105e0000001);
const RPC_C_AUTHN_WINNT: u32 = 10;

/// Id réservé du PÉRIMÈTRE permanent (scellé par défaut, ADR 2026-07-21) : ne
/// peut PAS être descellé via le pipe (sinon un malware user-level couperait
/// le sceau). Source unique : `crate::ipc::PERIMETRE`.
pub use crate::ipc::PERIMETRE;

/// Le périmètre de base connu du service (chemins fixes). L'app installée
/// (chemin par-utilisateur, inconnu de SYSTEM au boot) REJOINT ensuite via
/// `rejoindre`. Les chemins absents sont ignorés (seal saute les fichiers
/// introuvables) — un exe sera bloqué dès qu'il existera et tournera.
pub fn perimetre_base() -> Vec<String> {
    vec![
        r"C:\waly\bin\waly.exe".into(),
        r"C:\waly\bin\waly-voice.exe".into(),
        r"C:\waly\bin\waly-desktop.exe".into(),
        r"C:\waly\engines\flm\fastflowlm-windows-7eb32868007ae16f0281c875d91c4c16a2c429d3\flm.exe".into(),
    ]
}

/// Admissibilité au périmètre Waly (add-only, user-level). Source unique :
/// `crate::ipc::exe_admissible`. Sceller un exe NON admissible = agent tiers,
/// action admin (ADR 2026-09-16, garde dans `service::traiter`).
pub use crate::ipc::exe_admissible;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub use crate::ipc::Sortie;

/// Le scelleur : tient le moteur WFP ouvert pour la vie du service et la carte
/// session -> (ids de filtres, app-ids) pour lever et journaliser.
pub struct Sealer {
    engine: HANDLE,
    /// session -> ids de filtres runtime posés (à supprimer à la levée)
    filtres: HashMap<i64, Vec<u64>>,
    /// session -> app-ids scellés (pour filtrer les net events du journal)
    app_ids: HashMap<i64, Vec<Vec<u8>>>,
    /// vrai si le privilège WFP manque (service pas en SYSTEM / non élevé) :
    /// le pipe sert quand même (ping/état), mais sceller renvoie une erreur
    /// claire au lieu de paniquer. En prod le service tourne en SYSTEM.
    prive: bool,
    /// chemins d'exe du périmètre permanent (pour l'union à `rejoindre`).
    perimetre_exes: Vec<String>,
}

// HANDLE (*mut c_void) n'est pas Send par défaut ; le moteur n'est touché que
// par le thread de service (accès sérialisé sous Mutex côté appelant).
unsafe impl Send for Sealer {}

impl Sealer {
    /// Ouvre le moteur (session non-dynamique), active la collecte de net
    /// events (journal), garantit la sous-couche et purge tout résidu.
    pub fn new() -> Result<Sealer, String> {
        unsafe {
            let mut session: FWPM_SESSION0 = std::mem::zeroed();
            // NON-dynamique : pas de FWPM_SESSION_FLAG_DYNAMIC (fail-closed).
            let mut nom = wide("Waly Huis clos");
            session.displayData.name = nom.as_mut_ptr();

            let mut engine: HANDLE = std::ptr::null_mut();
            let r = FwpmEngineOpen0(
                std::ptr::null(),
                RPC_C_AUTHN_WINNT,
                std::ptr::null_mut(),
                &session,
                &mut engine,
            );
            if r != 0 {
                return Err(format!("FwpmEngineOpen0: {}", code(r)));
            }

            // Journal : activer la collecte (souvent déjà active système ; on
            // ignore l'échec, cf. banc 0x8032000B non bloquant).
            let mut un: FWP_VALUE0 = std::mem::zeroed();
            un.r#type = FWP_UINT32;
            un.Anonymous.uint32 = 1;
            FwpmEngineSetOption0(engine, FWPM_ENGINE_COLLECT_NET_EVENTS, &un);

            let mut s = Sealer {
                engine,
                filtres: HashMap::new(),
                app_ids: HashMap::new(),
                prive: false,
                perimetre_exes: Vec::new(),
            };
            // Privilège manquant (standard/non-SYSTEM) : mode dégradé plutôt que
            // panique — le pipe reste servi, sceller renvoie une erreur claire.
            match s.ensure_sublayer() {
                Ok(()) => s.purge_orphelins(),
                Err(e) if e.contains("ACCESS_DENIED") => {
                    s.prive = true;
                }
                Err(e) => return Err(e),
            }
            Ok(s)
        }
    }

    /// Le service a-t-il le privilège de sceller ?
    pub fn privilegie(&self) -> bool {
        !self.prive
    }

    unsafe fn ensure_sublayer(&mut self) -> Result<(), String> {
        let mut sl: FWPM_SUBLAYER0 = std::mem::zeroed();
        sl.subLayerKey = WALY_SUBLAYER;
        let mut nom = wide("Waly Huis clos");
        sl.displayData.name = nom.as_mut_ptr();
        sl.weight = 0xFFFF;
        let r = FwpmSubLayerAdd0(self.engine, &sl, std::ptr::null_mut());
        // 0x80320009 = FWP_E_ALREADY_EXISTS : bénin (reboot -> recréée ; ou
        // déjà là).
        if r != 0 && r != 0x8032_0009 {
            return Err(format!("FwpmSubLayerAdd0: {}", code(r)));
        }
        Ok(())
    }

    /// Supprime tous les filtres résidents de notre sous-couche (réconciliation
    /// au démarrage : aucune session connue). Best-effort.
    unsafe fn purge_orphelins(&mut self) {
        for layer in [FWPM_LAYER_ALE_AUTH_CONNECT_V4, FWPM_LAYER_ALE_AUTH_CONNECT_V6] {
            let ids = self.enum_filtres_sous_couche(layer);
            for id in ids {
                FwpmFilterDeleteById0(self.engine, id);
            }
        }
    }

    /// Ids runtime des filtres de NOTRE sous-couche à une couche donnée.
    unsafe fn enum_filtres_sous_couche(&self, layer: GUID) -> Vec<u64> {
        let mut out = Vec::new();
        let mut tmpl: FWPM_FILTER_ENUM_TEMPLATE0 = std::mem::zeroed();
        tmpl.layerKey = layer;
        tmpl.enumType = FWP_FILTER_ENUM_FULLY_CONTAINED;
        tmpl.actionMask = 0xFFFF_FFFF;
        let mut h: HANDLE = std::ptr::null_mut();
        if FwpmFilterCreateEnumHandle0(self.engine, &tmpl, &mut h) != 0 {
            return out;
        }
        loop {
            let mut entries: *mut *mut FWPM_FILTER0 = std::ptr::null_mut();
            let mut n = 0u32;
            if FwpmFilterEnum0(self.engine, h, 256, &mut entries, &mut n) != 0 || n == 0 {
                break;
            }
            for i in 0..n as usize {
                let f = &**entries.add(i);
                if guid_eq(&f.subLayerKey, &WALY_SUBLAYER) {
                    out.push(f.filterId);
                }
            }
            FwpmFreeMemory0(&mut entries as *mut _ as *mut *mut core::ffi::c_void);
            if n < 256 {
                break;
            }
        }
        FwpmFilterDestroyEnumHandle0(self.engine, h);
        out
    }

    /// Scelle une session : pour chaque exe, PERMIT-loopback (poids 15) + BLOCK
    /// (poids 0) aux couches V4/V6. Idempotent par session (re-sceller lève
    /// d'abord). Retourne le nombre de filtres posés.
    pub fn seal(&mut self, session: i64, exes: &[String]) -> Result<usize, String> {
        if self.prive {
            return Err("privilège insuffisant : le service doit tourner en SYSTEM".into());
        }
        self.retirer(session); // idempotence (interne : ne bute pas sur la garde périmètre)
        let mut ids = Vec::new();
        let mut app_ids = Vec::new();
        unsafe {
            // Transaction : pose atomique (tout ou rien).
            if FwpmTransactionBegin0(self.engine, 0) != 0 {
                return Err("FwpmTransactionBegin0".into());
            }
            let res = (|| -> Result<(), String> {
                for exe in exes {
                    // Chemin REEL (noms longs, liens suivis) : un chemin court
                    // `NOM~1` donnerait un app-id qui ne correspond pas au
                    // processus -> filtre pose mais inoperant (vecu 2026-10-01).
                    let exe = &chemin_reel(exe);
                    let path = wide(exe);
                    let mut blob: *mut FWP_BYTE_BLOB = std::ptr::null_mut();
                    let r = FwpmGetAppIdFromFileName0(path.as_ptr(), &mut blob);
                    if r != 0 {
                        // exe absent (ex. ollama non installé) : on saute sans
                        // faire échouer tout le sceau.
                        // fichier absent : codes Win32 bruts 2/3 ou HRESULT.
                        if matches!(r, 2 | 3 | 0x8032_0035 | 0x8007_0002 | 0x8007_0003) {
                            continue;
                        }
                        return Err(format!("app-id {exe}: {}", code(r)));
                    }
                    let id = std::slice::from_raw_parts((*blob).data, (*blob).size as usize).to_vec();
                    for layer in [FWPM_LAYER_ALE_AUTH_CONNECT_V4, FWPM_LAYER_ALE_AUTH_CONNECT_V6] {
                        let (p, b) = add_pair(self.engine, layer, blob)?;
                        ids.push(p);
                        ids.push(b);
                    }
                    let mut pblob = blob;
                    FwpmFreeMemory0(&mut pblob as *mut _ as *mut *mut core::ffi::c_void);
                    app_ids.push(id);
                }
                Ok(())
            })();
            match res {
                Ok(()) => {
                    if FwpmTransactionCommit0(self.engine) != 0 {
                        FwpmTransactionAbort0(self.engine);
                        return Err("FwpmTransactionCommit0".into());
                    }
                }
                Err(e) => {
                    FwpmTransactionAbort0(self.engine);
                    return Err(e);
                }
            }
        }
        let n = ids.len();
        self.filtres.insert(session, ids);
        self.app_ids.insert(session, app_ids);
        Ok(n)
    }

    /// Lève le sceau d'une session (supprime ses filtres). **Refuse l'id
    /// PÉRIMÈTRE** (scellé permanent, ADR 2026-07-21) : le périmètre par
    /// défaut ne se lève pas par le pipe. Best-effort.
    pub fn unseal(&mut self, session: i64) {
        if session == PERIMETRE {
            return;
        }
        self.retirer(session);
    }

    /// Suppression interne des filtres d'une session (sans la garde périmètre),
    /// pour l'idempotence de `seal` et la ré-application du périmètre.
    fn retirer(&mut self, session: i64) {
        if let Some(ids) = self.filtres.remove(&session) {
            unsafe {
                for id in ids {
                    FwpmFilterDeleteById0(self.engine, id);
                }
            }
        }
        self.app_ids.remove(&session);
    }

    /// Scelle le PÉRIMÈTRE de base au démarrage du service (ADR 2026-07-21 :
    /// scellé par défaut). Idempotent.
    pub fn sceller_perimetre(&mut self) -> Result<usize, String> {
        self.perimetre_exes = perimetre_base();
        let base = self.perimetre_exes.clone();
        self.seal(PERIMETRE, &base)
    }

    /// L'app REJOINT le périmètre en donnant son propre chemin d'exe (add-only,
    /// validé `exe_admissible`) : re-scelle le périmètre avec l'union. Ignore
    /// silencieusement un exe déjà présent ou non admissible.
    pub fn rejoindre(&mut self, exe: &str) -> Result<usize, String> {
        if !exe_admissible(exe) {
            return Err("exe non admissible au périmètre".into());
        }
        // Reconstituer la liste courante à partir des app-ids serait fragile ;
        // on garde la liste des chemins du périmètre à part.
        if self.perimetre_exes.iter().any(|e| e.eq_ignore_ascii_case(exe)) {
            return Ok(self.filtres.get(&PERIMETRE).map(|v| v.len()).unwrap_or(0));
        }
        self.perimetre_exes.push(exe.to_string());
        let liste = self.perimetre_exes.clone();
        self.seal(PERIMETRE, &liste)
    }

    /// Une session est-elle scellée ?
    pub fn est_scelle(&self, session: i64) -> bool {
        self.filtres.contains_key(&session)
    }

    /// Sessions actuellement scellées.
    pub fn sessions(&self) -> Vec<i64> {
        self.filtres.keys().copied().collect()
    }

    /// Draine les net events CLASSIFY_DROP correspondant aux app-ids d'une
    /// session scellée (le journal d'audit de cette session).
    pub fn drops(&self, session: i64) -> Vec<Sortie> {
        let Some(app_ids) = self.app_ids.get(&session) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        unsafe {
            let mut h: HANDLE = std::ptr::null_mut();
            if FwpmNetEventCreateEnumHandle0(self.engine, std::ptr::null(), &mut h) != 0 {
                return out;
            }
            loop {
                let mut entries: *mut *mut FWPM_NET_EVENT0 = std::ptr::null_mut();
                let mut n = 0u32;
                if FwpmNetEventEnum0(self.engine, h, 128, &mut entries, &mut n) != 0 || n == 0 {
                    break;
                }
                for i in 0..n as usize {
                    let ev = &**entries.add(i);
                    if ev.r#type != FWPM_NET_EVENT_TYPE_CLASSIFY_DROP {
                        continue;
                    }
                    let hd = &ev.header;
                    if hd.appId.data.is_null() {
                        continue;
                    }
                    let app = std::slice::from_raw_parts(hd.appId.data, hd.appId.size as usize);
                    if !app_ids.iter().any(|a| a == app) {
                        continue;
                    }
                    let ft = ((hd.timeStamp.dwHighDateTime as u64) << 32) | hd.timeStamp.dwLowDateTime as u64;
                    let unix = ft.saturating_sub(116_444_736_000_000_000) / 10_000_000;
                    let ip = hd.Anonymous2.remoteAddrV4.to_be_bytes();
                    out.push(Sortie {
                        t: unix,
                        exe: exe_de_appid(app),
                        proto: hd.ipProtocol,
                        adresse: format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]),
                        port: hd.remotePort,
                    });
                }
                FwpmFreeMemory0(&mut entries as *mut _ as *mut *mut core::ffi::c_void);
                if n < 128 {
                    break;
                }
            }
            FwpmNetEventDestroyEnumHandle0(self.engine, h);
        }
        out
    }
}

impl Drop for Sealer {
    fn drop(&mut self) {
        // Fail-closed : on NE supprime PAS les filtres à la fermeture du moteur
        // (session non-dynamique) — si le service tombe, le sceau tient. La
        // réconciliation se fait au prochain démarrage (purge_orphelins).
        unsafe {
            FwpmEngineClose0(self.engine);
        }
    }
}

/// Pose la paire PERMIT-loopback / BLOCK pour un app-id à une couche. Renvoie
/// (id_permit, id_block). `blob` doit rester valide pendant l'appel.
unsafe fn add_pair(engine: HANDLE, layer: GUID, blob: *mut FWP_BYTE_BLOB) -> Result<(u64, u64), String> {
    let mut cond_app: FWPM_FILTER_CONDITION0 = std::mem::zeroed();
    cond_app.fieldKey = FWPM_CONDITION_ALE_APP_ID;
    cond_app.matchType = FWP_MATCH_EQUAL;
    cond_app.conditionValue.r#type = FWP_BYTE_BLOB_TYPE;
    cond_app.conditionValue.Anonymous.byteBlob = blob;

    let mut cond_loop: FWPM_FILTER_CONDITION0 = std::mem::zeroed();
    cond_loop.fieldKey = FWPM_CONDITION_FLAGS;
    cond_loop.matchType = FWP_MATCH_FLAGS_ALL_SET;
    cond_loop.conditionValue.r#type = FWP_UINT32;
    cond_loop.conditionValue.Anonymous.uint32 = FWP_CONDITION_FLAG_IS_LOOPBACK;

    let mut conds = [cond_app, cond_loop];
    let mut f: FWPM_FILTER0 = std::mem::zeroed();
    let mut fnom = wide("Waly sceau: permit loopback");
    f.displayData.name = fnom.as_mut_ptr();
    f.layerKey = layer;
    f.subLayerKey = WALY_SUBLAYER;
    f.weight.r#type = FWP_UINT8;
    f.weight.Anonymous.uint8 = 15;
    f.numFilterConditions = 2;
    f.filterCondition = conds.as_mut_ptr();
    f.action.r#type = FWP_ACTION_PERMIT;
    let mut id_p = 0u64;
    let r = FwpmFilterAdd0(engine, &f, std::ptr::null_mut(), &mut id_p);
    if r != 0 {
        return Err(format!("FwpmFilterAdd0 permit: {}", code(r)));
    }

    let mut conds_b = [cond_app];
    let mut b: FWPM_FILTER0 = std::mem::zeroed();
    let mut bnom = wide("Waly sceau: block sortie");
    b.displayData.name = bnom.as_mut_ptr();
    b.layerKey = layer;
    b.subLayerKey = WALY_SUBLAYER;
    b.weight.r#type = FWP_UINT8;
    b.weight.Anonymous.uint8 = 0;
    b.numFilterConditions = 1;
    b.filterCondition = conds_b.as_mut_ptr();
    b.action.r#type = FWP_ACTION_BLOCK;
    let mut id_b = 0u64;
    let r = FwpmFilterAdd0(engine, &b, std::ptr::null_mut(), &mut id_b);
    if r != 0 {
        return Err(format!("FwpmFilterAdd0 block: {}", code(r)));
    }
    Ok((id_p, id_b))
}

/// L'app-id WFP est le chemin NT du fichier en UTF-16LE (avec NUL final).
/// On le décode en chemin lisible pour le journal.
fn exe_de_appid(app: &[u8]) -> String {
    if app.len() < 2 {
        return String::new();
    }
    let u16s: Vec<u16> = app
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();
    String::from_utf16_lossy(&u16s)
}

fn guid_eq(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

fn code(e: u32) -> String {
    match e {
        0 => "OK".into(),
        // WFP renvoie les codes Win32 bruts (pas des HRESULT) : ACCESS_DENIED=5.
        5 | 0x8007_0005 => format!("0x{e:08X} (ACCESS_DENIED)"),
        0x8032_0009 => "0x80320009 (FWP_E_ALREADY_EXISTS)".into(),
        0x8032_0035 => "0x80320035 (FWP_E_NOT_FOUND)".into(),
        other => format!("0x{other:08X}"),
    }
}

/// Chemin resolu d'un executable, sans le prefixe de chemin etendu.
fn chemin_reel(exe: &str) -> String {
    const ETENDU: &str = r"\\?\";
    match std::fs::canonicalize(exe) {
        Ok(p) => {
            let s = p.to_string_lossy().into_owned();
            match s.strip_prefix(ETENDU) {
                Some(reste) if !reste.starts_with("UNC") => reste.to_string(),
                _ => s,
            }
        }
        Err(_) => exe.to_string(),
    }
}
