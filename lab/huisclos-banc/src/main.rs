//! Banc R6a « Huis clos » — les 3 gates du scellé WFP, mesurés sous SAC.
//!
//! GATE 1 : WFP depuis Rust windows-gnu, modèle de droits (standard vs admin).
//! GATE 2 : la paire PERMIT-loopback / BLOCK laisse-t-elle vivre le loopback ?
//! GATE 3 : latence de pose/levée du sceau, surcoût par connexion loopback.
//!
//! Sous-commandes :
//!   banc                        — auto-test complet (scelle SON PROPRE exe)
//!   sonder                      — sondes réseau seules (avant/après un sceau externe)
//!   sceller <exe>... -- <secs>  — scelle des exes arbitraires et tient N s
//!                                 (test GATE 2 sur flm.exe pendant qu'il sert)
//!
//! Sortie ASCII pur (console cp850). Aucun std::process::Command (vécu SAC R2).

#![allow(non_snake_case)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::NetworkManagement::WindowsFilteringPlatform::*;
use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// Sous-couche dédiée au banc (GUID fixe, sans signification extérieure).
const BANC_SUBLAYER: GUID = GUID::from_u128(0x5ea1_c105_2026_0720_a11e_000000000001);
/// RPC_C_AUTHN_WINNT (évite la feature Win32_System_Rpc pour une constante).
const RPC_C_AUTHN_WINNT: u32 = 10;

/// Tee vers stdout ET, si WALY_BANC_LOG est pose, vers ce fichier — le
/// process eleve (UAC) a une console separee, on capte sa sortie par fichier.
macro_rules! tee {
    ($($a:tt)*) => {{
        let line = format!($($a)*);
        println!("{line}");
        // Chemin fixe (le process eleve a une console separee ET n'herite pas
        // toujours l'env via UAC — on ne compte pas dessus).
        {
            use std::io::Write;
            let p = std::env::var("WALY_BANC_LOG")
                .unwrap_or_else(|_| r"C:\waly\lab\huisclos-banc\gate-eleve.log".into());
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
                let _ = writeln!(f, "{line}");
            }
        }
    }};
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn est_eleve() -> bool {
    unsafe {
        let mut tok: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok) == 0 {
            return false;
        }
        let mut elev = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(
            tok,
            TokenElevation,
            &mut elev as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        ok != 0 && elev.TokenIsElevated != 0
    }
}

fn code(e: u32) -> String {
    match e {
        0 => "OK".into(),
        0x80070005 => "0x80070005 (ACCESS_DENIED)".into(),
        0x80320035 => "0x80320035 (FWP_E_NOT_FOUND)".into(),
        other => format!("0x{other:08X}"),
    }
}

// ---------------------------------------------------------------- Le sceau —

struct Sceau {
    engine: HANDLE,
    /// app-ids poses (blobs copies pour comparer aux net events)
    app_ids: Vec<Vec<u8>>,
    /// valeur precedente de CollectNetEvents (a restaurer), si lue
    net_events_avant: Option<u32>,
}

impl Sceau {
    /// Ouvre une session WFP DYNAMIQUE et pose PERMIT-loopback + BLOCK pour
    /// chaque exe. Rapporte chaque etape avec code d'erreur exact (GATE 1).
    fn poser(exes: &[String]) -> Result<(Sceau, Duration), String> {
        unsafe {
            let t0 = Instant::now();
            let mut session: FWPM_SESSION0 = std::mem::zeroed();
            session.flags = FWPM_SESSION_FLAG_DYNAMIC;
            let mut nom = wide("Waly banc Huis clos");
            session.displayData.name = nom.as_mut_ptr();

            let mut engine: HANDLE = std::ptr::null_mut();
            let r = FwpmEngineOpen0(
                std::ptr::null(),
                RPC_C_AUTHN_WINNT,
                std::ptr::null_mut(),
                &session,
                &mut engine,
            );
            tee!("  FwpmEngineOpen0 (session dynamique) : {}", code(r));
            if r != 0 {
                return Err(format!("engine open: {}", code(r)));
            }
            let mut sceau = Sceau { engine, app_ids: Vec::new(), net_events_avant: None };

            // Journal : activer la collecte des net events (drops), en
            // memorisant la valeur d'avant pour la restaurer.
            let mut avant: *mut FWP_VALUE0 = std::ptr::null_mut();
            let r = FwpmEngineGetOption0(engine, FWPM_ENGINE_COLLECT_NET_EVENTS, &mut avant);
            if r == 0 && !avant.is_null() {
                let v = if (*avant).r#type == FWP_UINT32 { (*avant).Anonymous.uint32 } else { 0 };
                tee!("  CollectNetEvents (avant) : {v}");
                sceau.net_events_avant = Some(v);
                FwpmFreeMemory0(&mut avant as *mut _ as *mut *mut core::ffi::c_void);
            } else {
                tee!("  FwpmEngineGetOption0(CollectNetEvents) : {}", code(r));
            }
            let mut un: FWP_VALUE0 = std::mem::zeroed();
            un.r#type = FWP_UINT32;
            un.Anonymous.uint32 = 1;
            let r = FwpmEngineSetOption0(engine, FWPM_ENGINE_COLLECT_NET_EVENTS, &un);
            tee!("  FwpmEngineSetOption0(CollectNetEvents=1) : {}", code(r));

            // Sous-couche dediee, poids fort.
            let mut sl: FWPM_SUBLAYER0 = std::mem::zeroed();
            sl.subLayerKey = BANC_SUBLAYER;
            let mut sl_nom = wide("Waly Huis clos (banc)");
            sl.displayData.name = sl_nom.as_mut_ptr();
            sl.weight = 0xFFFF;
            let r = FwpmSubLayerAdd0(engine, &sl, std::ptr::null_mut());
            tee!("  FwpmSubLayerAdd0 : {}", code(r));
            if r != 0 {
                return Err(format!("sublayer add: {}", code(r)));
            }

            for exe in exes {
                let path = wide(exe);
                let mut blob: *mut FWP_BYTE_BLOB = std::ptr::null_mut();
                let r = FwpmGetAppIdFromFileName0(path.as_ptr(), &mut blob);
                if r != 0 {
                    tee!("  app-id {exe} : {}", code(r));
                    return Err(format!("app-id {exe}: {}", code(r)));
                }
                let id = std::slice::from_raw_parts((*blob).data, (*blob).size as usize).to_vec();

                for (layer, v6) in [(FWPM_LAYER_ALE_AUTH_CONNECT_V4, false), (FWPM_LAYER_ALE_AUTH_CONNECT_V6, true)] {
                    // 1) PERMIT loopback, poids 15.
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
                    let mut f_nom = wide("Waly sceau: permit loopback");
                    f.displayData.name = f_nom.as_mut_ptr();
                    f.layerKey = layer;
                    f.subLayerKey = BANC_SUBLAYER;
                    f.weight.r#type = FWP_UINT8;
                    f.weight.Anonymous.uint8 = 15;
                    f.numFilterConditions = 2;
                    f.filterCondition = conds.as_mut_ptr();
                    f.action.r#type = FWP_ACTION_PERMIT;
                    let mut id_f = 0u64;
                    let r = FwpmFilterAdd0(engine, &f, std::ptr::null_mut(), &mut id_f);
                    if r != 0 {
                        tee!("  FwpmFilterAdd0 permit-loopback ({}) : {}", if v6 { "v6" } else { "v4" }, code(r));
                        return Err(format!("filter add permit: {}", code(r)));
                    }

                    // 2) BLOCK tout le reste pour cet exe, poids 0.
                    let mut conds_b = [cond_app];
                    let mut b: FWPM_FILTER0 = std::mem::zeroed();
                    let mut b_nom = wide("Waly sceau: block sortie");
                    b.displayData.name = b_nom.as_mut_ptr();
                    b.layerKey = layer;
                    b.subLayerKey = BANC_SUBLAYER;
                    b.weight.r#type = FWP_UINT8;
                    b.weight.Anonymous.uint8 = 0;
                    b.numFilterConditions = 1;
                    b.filterCondition = conds_b.as_mut_ptr();
                    b.action.r#type = FWP_ACTION_BLOCK;
                    let mut id_b = 0u64;
                    let r = FwpmFilterAdd0(engine, &b, std::ptr::null_mut(), &mut id_b);
                    if r != 0 {
                        tee!("  FwpmFilterAdd0 block ({}) : {}", if v6 { "v6" } else { "v4" }, code(r));
                        return Err(format!("filter add block: {}", code(r)));
                    }
                }
                let mut pblob = blob;
                FwpmFreeMemory0(&mut pblob as *mut _ as *mut *mut core::ffi::c_void);
                sceau.app_ids.push(id);
                tee!("  scelle : {exe} (4 filtres)");
            }
            Ok((sceau, t0.elapsed()))
        }
    }

    /// Enumere les net events CLASSIFY_DROP dont l'app-id est un des notres.
    fn tentatives(&self) -> Vec<String> {
        let mut sorties = Vec::new();
        unsafe {
            let mut h: HANDLE = std::ptr::null_mut();
            let r = FwpmNetEventCreateEnumHandle0(self.engine, std::ptr::null(), &mut h);
            if r != 0 {
                sorties.push(format!("enum handle : {}", code(r)));
                return sorties;
            }
            loop {
                let mut entries: *mut *mut FWPM_NET_EVENT0 = std::ptr::null_mut();
                let mut n = 0u32;
                let r = FwpmNetEventEnum0(self.engine, h, 64, &mut entries, &mut n);
                if r != 0 || n == 0 {
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
                    if !self.app_ids.iter().any(|a| a == app) {
                        continue;
                    }
                    // FILETIME -> secondes unix.
                    let ft = ((hd.timeStamp.dwHighDateTime as u64) << 32) | hd.timeStamp.dwLowDateTime as u64;
                    let unix = ft.saturating_sub(116_444_736_000_000_000) / 10_000_000;
                    let ip = hd.Anonymous2.remoteAddrV4.to_be_bytes();
                    sorties.push(format!(
                        "DROP t={} proto={} vers {}.{}.{}.{}:{}",
                        unix, hd.ipProtocol, ip[0], ip[1], ip[2], ip[3], hd.remotePort
                    ));
                }
                FwpmFreeMemory0(&mut entries as *mut _ as *mut *mut core::ffi::c_void);
                if n < 64 {
                    break;
                }
            }
            FwpmNetEventDestroyEnumHandle0(self.engine, h);
        }
        sorties
    }

    /// Leve le sceau (fermeture de la session dynamique) et mesure.
    fn lever(self) -> Duration {
        unsafe {
            // Restaurer CollectNetEvents si on l'avait modifie.
            if let Some(v) = self.net_events_avant {
                if v != 1 {
                    let mut val: FWP_VALUE0 = std::mem::zeroed();
                    val.r#type = FWP_UINT32;
                    val.Anonymous.uint32 = v;
                    FwpmEngineSetOption0(self.engine, FWPM_ENGINE_COLLECT_NET_EVENTS, &val);
                }
            }
            let t0 = Instant::now();
            FwpmEngineClose0(self.engine);
            t0.elapsed()
        }
    }
}

// -------------------------------------------------------------- Les sondes —

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Connexion TCP avec timeout ; rapporte duree + resultat (code OS si echec).
fn sonde_tcp(addr: &str, timeout_ms: u64) -> String {
    let sa: std::net::SocketAddr = addr.parse().unwrap();
    let t0 = Instant::now();
    match TcpStream::connect_timeout(&sa, Duration::from_millis(timeout_ms)) {
        Ok(_) => format!("{addr} : CONNECTE en {:.1} ms", ms(t0.elapsed())),
        Err(e) => format!(
            "{addr} : ECHEC en {:.1} ms (os error {})",
            ms(t0.elapsed()),
            e.raw_os_error().unwrap_or(0)
        ),
    }
}

fn sonde_udp(addr: &str) -> String {
    let s = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => return format!("udp bind : {e}"),
    };
    let t0 = Instant::now();
    match s.send_to(&[0u8; 12], addr) {
        Ok(_) => format!("udp {addr} : ENVOYE en {:.1} ms", ms(t0.elapsed())),
        Err(e) => format!(
            "udp {addr} : ECHEC en {:.1} ms (os error {})",
            ms(t0.elapsed()),
            e.raw_os_error().unwrap_or(0)
        ),
    }
}

/// Serveur loopback ephemere (echo 1 octet) pour la mesure du surcout.
fn serveur_echo() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let mut b = [0u8; 1];
            let _ = s.read(&mut b);
            let _ = s.write_all(&b);
        }
    });
    port
}

/// Mediane de N connexions+aller-retour loopback (GATE 3 : surcout du sceau).
fn mediane_loopback(port: u16, n: usize) -> f64 {
    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let mut t = Vec::with_capacity(n);
    for _ in 0..n {
        let t0 = Instant::now();
        if let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
            let _ = s.write_all(&[7]);
            let mut b = [0u8; 1];
            let _ = s.read(&mut b);
            t.push(ms(t0.elapsed()));
        }
    }
    if t.is_empty() {
        return f64::NAN;
    }
    t.sort_by(f64::total_cmp);
    t[t.len() / 2]
}

fn sondes(port_echo: u16) {
    tee!("  loopback 127.0.0.1:{port_echo} -> {}", sonde_tcp(&format!("127.0.0.1:{port_echo}"), 1000));
    tee!("  {}", sonde_tcp("1.1.1.1:443", 3000));
    tee!("  {}", sonde_tcp("9.9.9.9:443", 3000));
    tee!("  {}", sonde_udp("8.8.8.8:53"));
}

// -------------------------------------------------------------------- main —

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let exe = std::env::current_exe().unwrap().to_string_lossy().into_owned();
    tee!("=== huisclos-banc demarre — eleve : {} ===", est_eleve());

    match args.first().map(String::as_str) {
        Some("sonder") => {
            let port = serveur_echo();
            sondes(port);
        }
        Some("sceller") => {
            // sceller <exe>... <secs>  (dernier argument = duree de tenue)
            let secs: u64 = args.last().and_then(|s| s.parse().ok()).unwrap_or(30);
            let exes: Vec<String> = args[1..args.len().saturating_sub(1)].to_vec();
            if exes.is_empty() {
                eprintln!("usage: huisclos-banc sceller <exe>... <secs>");
                std::process::exit(2);
            }
            match Sceau::poser(&exes) {
                Ok((sceau, d)) => {
                    tee!("sceau pose en {:.1} ms — tenue {secs} s", ms(d));
                    std::thread::sleep(Duration::from_secs(secs));
                    for t in sceau.tentatives() {
                        tee!("  {t}");
                    }
                    let d = sceau.lever();
                    tee!("sceau leve en {:.1} ms", ms(d));
                }
                Err(e) => {
                    tee!("ECHEC pose : {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            // banc complet : baseline -> sceau sur soi-meme -> sondes -> journal -> levee.
            let port = serveur_echo();
            tee!("[1] Sondes AVANT sceau (baseline)");
            sondes(port);
            let base = mediane_loopback(port, 200);
            tee!("  mediane loopback x200 : {base:.2} ms");

            tee!("[2] Pose du sceau sur cet exe : {exe}");
            match Sceau::poser(&[exe]) {
                Ok((sceau, d)) => {
                    tee!("  POSE en {:.1} ms", ms(d));
                    tee!("[3] Sondes SOUS sceau (attendu : loopback OK, sorties ECHEC os 10013)");
                    sondes(port);
                    let sous = mediane_loopback(port, 200);
                    tee!("  mediane loopback x200 : {sous:.2} ms (baseline {base:.2})");
                    std::thread::sleep(Duration::from_millis(300));
                    tee!("[4] Journal (net events CLASSIFY_DROP, nos app-ids)");
                    let ts = sceau.tentatives();
                    if ts.is_empty() {
                        tee!("  (aucun evenement lu)");
                    }
                    for t in &ts {
                        tee!("  {t}");
                    }
                    let d = sceau.lever();
                    tee!("[5] Sceau LEVE en {:.1} ms — sondes apres levee (attendu : retour normal)", ms(d));
                    sondes(port);
                }
                Err(e) => {
                    tee!("  ECHEC pose : {e}");
                    tee!("  (GATE 1 : voie a trancher — ACL one-time / service / UAC au scellement)");
                    std::process::exit(1);
                }
            }
        }
    }
}
