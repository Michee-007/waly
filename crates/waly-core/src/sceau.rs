//! Client du sceau « Huis clos » (R6a) — côté user-level.
//!
//! Parle au service `waly-seal-svc` (SYSTEM) par le named-pipe, en protocole
//! **étroit** (`waly_seal::ipc`) : sceller/desceller/journal/état MA session,
//! jamais un filtre brut. Le service détient tout le WFP ; ici, zéro
//! dépendance Windows lourde — le pipe s'ouvre comme un fichier.
//!
//! Écrit aussi le **journal d'audit local** (`audit_sceau` en SQLite) : pose,
//! levée, et chaque tentative de sortie bloquée. C'est la preuve consultable
//! (one-pager conformité, chantier 4).
//!
//! Si le service est absent : `disponible()` renvoie `false`. Le fallback
//! UAC-par-sceau (relance élevée ponctuelle) est câblé au desktop (chantier 1),
//! pas ici — ici on constate seulement l'indisponibilité.

use rusqlite::Connection;
use waly_seal::ipc::{Reponse, Requete, Sortie, Succes, PIPE_NAME};

/// Id du périmètre permanent (scellé par défaut, ADR 2026-07-21). Le journal
/// et l'auto-test portent sur lui.
pub const PERIMETRE: i64 = 0;

/// Le périmètre par défaut d'une session Waly (chemins d'exe à sceller). Le
/// service saute silencieusement ceux qui n'existent pas (ex. ollama absent).
/// WebView2 est ABSENT exprès : exe partagé, neutralisé au spawn (chantier 1).
pub fn perimetre_defaut() -> Vec<String> {
    let mut v = Vec::new();
    // waly.exe (desktop) : l'exe courant si c'est lui, sinon chemins connus.
    if let Ok(exe) = std::env::current_exe() {
        v.push(exe.to_string_lossy().into_owned());
    }
    for p in [
        r"C:\waly\bin\waly.exe",
        r"C:\waly\bin\waly-voice.exe",
        r"C:\waly\engines\flm\fastflowlm-windows-7eb32868007ae16f0281c875d91c4c16a2c429d3\flm.exe",
    ] {
        if !v.iter().any(|x| x.eq_ignore_ascii_case(p)) {
            v.push(p.to_string());
        }
    }
    v
}

/// Le service répond-il ? (Ping.)
pub fn disponible() -> bool {
    matches!(appel(&Requete::Ping), Ok(Reponse::Ok(_)))
}

/// L'app REJOINT le périmètre permanent en déclarant son propre chemin d'exe
/// (ADR 2026-07-21 : scellé par défaut ; l'app se déclare, elle ne « scelle »
/// pas). Best-effort — à appeler au démarrage. `db` : ligne d'audit `pose`.
pub fn rejoindre(exe: &str, db: Option<&Connection>) -> Result<usize, String> {
    let exe = &chemin_reel(exe);
    let r = appel(&Requete::Rejoindre { exe: exe.to_string() })?;
    match r {
        Reponse::Ok(Succes::Scelle { filtres, .. }) => {
            if let Some(c) = db {
                let court = exe_court(exe);
                let _ = journaliser_evenement(c, PERIMETRE, "pose", &format!("{court} a rejoint le périmètre ({filtres} filtres)"));
            }
            Ok(filtres)
        }
        Reponse::Ok(_) => Err("réponse inattendue".into()),
        Reponse::Err { message } => Err(message),
    }
}

/// Chemin RÉEL d'un exécutable : celui que le noyau associe au processus.
/// Vécu 2026-10-01 : lancée par un chemin court Windows (`NOM~1`) ou à
/// travers un lien, l'app déclarait ce chemin-là ; les filtres visaient alors
/// un autre identifiant d'application et RIEN n'était bloqué, alors que le
/// service répondait « scellé ». On déclare toujours le chemin résolu (noms
/// longs, liens suivis), sans le préfixe de chemin étendu de Windows.
pub fn chemin_reel(exe: &str) -> String {
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

/// Scelle `session` sur `exes`. Renvoie le nombre de filtres posés.
/// Écrit une ligne d'audit `pose` (best-effort) si `db` fourni.
pub fn poser(session: i64, exes: &[String], db: Option<&Connection>) -> Result<usize, String> {
    let r = appel(&Requete::Sceller {
        session,
        exes: exes.to_vec(),
    })?;
    match r {
        Reponse::Ok(Succes::Scelle { filtres, exes_vus }) => {
            if let Some(c) = db {
                let _ = journaliser_evenement(c, session, "pose", &format!("{exes_vus} exes, {filtres} filtres"));
            }
            Ok(filtres)
        }
        Reponse::Ok(_) => Err("réponse inattendue au scellement".into()),
        Reponse::Err { message } => Err(message),
    }
}

/// Lève le sceau de `session`. Écrit une ligne d'audit `levee` (best-effort).
pub fn lever(session: i64, db: Option<&Connection>) -> Result<(), String> {
    let r = appel(&Requete::Desceller { session })?;
    match r {
        Reponse::Ok(_) => {
            if let Some(c) = db {
                let _ = journaliser_evenement(c, session, "levee", "");
            }
            Ok(())
        }
        Reponse::Err { message } => Err(message),
    }
}

/// Draine le journal des sorties bloquées de `session` depuis le service, et
/// les inscrit dans `audit_sceau` (best-effort). Renvoie les tentatives.
pub fn drainer_journal(session: i64, db: Option<&Connection>) -> Result<Vec<Sortie>, String> {
    let r = appel(&Requete::Journal { session })?;
    match r {
        Reponse::Ok(Succes::Journal { drops }) => {
            if let Some(c) = db {
                for d in &drops {
                    let _ = journaliser_sortie(c, session, d);
                }
            }
            Ok(drops)
        }
        Reponse::Ok(_) => Err("réponse inattendue au journal".into()),
        Reponse::Err { message } => Err(message),
    }
}

/// Sessions actuellement scellées (selon le service).
pub fn sessions_scellees() -> Result<Vec<i64>, String> {
    match appel(&Requete::Etat)? {
        Reponse::Ok(Succes::Etat { sessions, .. }) => Ok(sessions),
        Reponse::Ok(_) => Err("réponse inattendue à l'état".into()),
        Reponse::Err { message } => Err(message),
    }
}

/// État du service : (sessions scellées, version, privilégié SYSTEM). Le
/// desktop l'utilise pour distinguer « service absent » de « service présent
/// mais sans privilège » (chantier 1).
pub fn etat() -> Result<(Vec<i64>, String, bool), String> {
    match appel(&Requete::Etat)? {
        Reponse::Ok(Succes::Etat { sessions, version, privilegie }) => Ok((sessions, version, privilegie)),
        Reponse::Ok(_) => Err("réponse inattendue à l'état".into()),
        Reponse::Err { message } => Err(message),
    }
}

/// Le huis clos est-il RÉELLEMENT tenu ? (service présent, privilégié, et le
/// périmètre effectivement scellé.) Sert la conscience du sceau au prompt :
/// on n'affirme le huis clos que s'il est vrai (honnêteté — sinon le prompt
/// mentirait quand le service manque). À interroger AUX REBUILDS seulement
/// (rare) — jamais par tour (discipline append-only du cache FLM).
pub fn actif() -> bool {
    matches!(etat(), Ok((sessions, _, privilegie)) if privilegie && sessions.contains(&PERIMETRE))
}

/// Auto-test du sceau : tente une VRAIE sortie réseau depuis CE processus
/// (`waly.exe`, qui fait partie du périmètre scellé). Sous sceau, WFP la
/// bloque (`WSAEACCES` = os 10013) ET la journalise ; hors sceau, elle
/// réussit. C'est l'auto-vérification pour le professionnel : « prouve-moi
/// que rien ne sort ». Ne persiste rien lui-même — le journal WFP est le
/// registre ; l'appelant draine ensuite pour voir la tentative.
pub fn tester_sortie() -> (bool, String) {
    use std::net::TcpStream;
    use std::time::Duration;
    // Cible publique neutre (résolveur DNS de Cloudflare) ; connexion TCP
    // seule, aucune donnée envoyée.
    let cible = "1.1.1.1:443";
    let addr: std::net::SocketAddr = match cible.parse() {
        Ok(a) => a,
        Err(_) => return (false, "cible invalide".into()),
    };
    match TcpStream::connect_timeout(&addr, Duration::from_millis(2500)) {
        Ok(_) => (false, format!("sortie vers {cible} : RÉUSSIE — le sceau n'est pas actif")),
        Err(e) => {
            // 10013 = WSAEACCES : bloqué par le pare-feu (notre sceau WFP).
            let bloque = e.raw_os_error() == Some(10013);
            if bloque {
                (true, format!("sortie vers {cible} : BLOQUÉE par le huis clos"))
            } else {
                (false, format!("sortie vers {cible} : échec ({e}) — pas un blocage de sceau"))
            }
        }
    }
}

// --- Agents TIERS scellés par l'utilisateur (chantier C) --------------------
//
// L'UI « Sceller un autre agent » scelle un exe qui n'est PAS de Waly. Le
// service exige une ELEVATION pour ça (ADR 2026-09-16) → l'app (user) ne peut
// pas sceller directement : elle lance le CLI du service en élevé (UAC).
// Le service ne rend que des id de session ; on garde le {exe, session} ici.

pub use waly_seal::ipc::session_pour_exe;

/// Un agent tiers connu de l'app (avec son statut vivant côté service).
#[derive(Debug, serde::Serialize)]
pub struct AgentScelle {
    pub exe: String,
    pub nom: String,
    pub session: i64,
    pub at: String,
    /// le service le tient-il effectivement scellé en ce moment ?
    pub actif: bool,
}

/// Enregistre (ou rafraîchit) un agent tiers dans le registre app-side.
pub fn enregistrer_agent(c: &Connection, exe: &str) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO agents_scelles (exe, session) VALUES (?1, ?2)
         ON CONFLICT(exe) DO UPDATE SET session=excluded.session",
        rusqlite::params![exe, session_pour_exe(exe)],
    )?;
    Ok(())
}

/// Retire un agent du registre (après une levée définitive).
pub fn oublier_agent(c: &Connection, exe: &str) -> rusqlite::Result<()> {
    c.execute("DELETE FROM agents_scelles WHERE exe = ?1", rusqlite::params![exe])?;
    Ok(())
}

/// Liste les agents tiers du registre, statut vivant recoupé avec le service.
pub fn agents(c: &Connection) -> rusqlite::Result<Vec<AgentScelle>> {
    let scellees = sessions_scellees().unwrap_or_default();
    let mut st = c.prepare("SELECT exe, session, at FROM agents_scelles ORDER BY at DESC")?;
    let rows = st.query_map([], |r| {
        let exe: String = r.get(0)?;
        let session: i64 = r.get(1)?;
        Ok(AgentScelle {
            nom: exe_court(&exe),
            actif: scellees.contains(&session),
            exe,
            session,
            at: r.get(2)?,
        })
    })?;
    rows.collect()
}

// --- Élévation : sceller/lever un agent tiers via le CLI du service (Windows)

/// Chemin de l'exe du service scelleur (qui est AUSSI le CLI `seal`/`unseal`).
/// Machine d'abord (`%ProgramFiles%\Waly`), repli dev.
#[cfg(windows)]
pub fn chemin_service() -> Option<String> {
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
    for c in [
        format!(r"{pf}\Waly\waly-seal-svc.exe"),
        r"C:\waly\lab\huisclos-banc\waly-seal-svc-c-release.exe".to_string(),
        r"C:\waly\bin\waly-seal-svc.exe".to_string(),
    ] {
        if std::path::Path::new(&c).exists() {
            return Some(c);
        }
    }
    None
}

/// Scelle un agent TIERS en lançant le CLI du service **élevé** (UAC). Bloque
/// jusqu'à la fin (ou le refus d'UAC). `Ok(true)` = scellé (exit 0), `Ok(false)`
/// = l'exe n'a pas encore de sortie à bloquer / 0 filtre, `Err` = refus/échec.
#[cfg(windows)]
pub fn sceller_agent(exe: &str) -> Result<bool, String> {
    let code = lancer_eleve("seal", exe)?;
    match code {
        0 => Ok(true),
        _ => Err(format!("le scellé a échoué (code {code}) — UAC refusé ?")),
    }
}

/// Lève le sceau d'un agent tiers (CLI élevé, UAC).
#[cfg(windows)]
pub fn desceller_agent(exe: &str) -> Result<(), String> {
    match lancer_eleve("unseal", exe)? {
        0 => Ok(()),
        c => Err(format!("la levée a échoué (code {c}) — UAC refusé ?")),
    }
}

/// Lance `<service> <verbe> <exe>` en élévation (ShellExecuteExW « runas »),
/// attend, renvoie le code de sortie. UAC refusé → `ERROR_CANCELLED` (1223).
#[cfg(windows)]
fn lancer_eleve(verbe: &str, exe: &str) -> Result<i32, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_CANCELLED};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};

    let svc = chemin_service().ok_or("service scelleur introuvable")?;
    let w = |s: &str| -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    };
    let verb_w = w("runas");
    let file_w = w(&svc);
    // Paramètres : `seal "<exe>"` (guillemets pour les chemins avec espaces).
    let params_w = w(&format!("{verbe} \"{exe}\""));

    unsafe {
        let mut sei: SHELLEXECUTEINFOW = std::mem::zeroed();
        sei.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        sei.fMask = SEE_MASK_NOCLOSEPROCESS;
        sei.lpVerb = verb_w.as_ptr();
        sei.lpFile = file_w.as_ptr();
        sei.lpParameters = params_w.as_ptr();
        sei.nShow = 0; // SW_HIDE
        if ShellExecuteExW(&mut sei) == 0 {
            let e = GetLastError();
            if e == ERROR_CANCELLED {
                return Err("élévation refusée (UAC)".into());
            }
            return Err(format!("ShellExecuteExW: erreur {e}"));
        }
        if sei.hProcess.is_null() {
            return Ok(0); // pas de handle (rare) : on suppose lancé
        }
        WaitForSingleObject(sei.hProcess, INFINITE);
        let mut code: u32 = 0;
        GetExitCodeProcess(sei.hProcess, &mut code);
        CloseHandle(sei.hProcess);
        Ok(code as i32)
    }
}

/// Sélecteur de fichier natif filtré .exe (comdlg32, DLL système → SAC-safe).
/// `None` si l'utilisateur annule. Windows uniquement.
#[cfg(windows)]
pub fn choisir_exe() -> Option<String> {
    choisir_fichier(
        "Choisir l'exécutable de l'agent à sceller",
        "Programmes (*.exe)\0*.exe\0Tous les fichiers\0*.*\0\0",
    )
}

/// Sélecteur de fichier natif générique (comdlg32, DLL système → SAC-safe).
/// `filtre` au format OPENFILENAME (`"Nom\0*.a;*.b\0...\0\0"`). `None` si
/// l'utilisateur annule. Sert au sceau (C) et aux fichiers joints (lot 1).
#[cfg(windows)]
pub fn choisir_fichier(titre: &str, filtre: &str) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OPENFILENAMEW,
    };
    let filtre: Vec<u16> = filtre.encode_utf16().collect();
    let titre: Vec<u16> = std::ffi::OsStr::new(titre)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut buf = vec![0u16; 1024];
    unsafe {
        let mut ofn: OPENFILENAMEW = std::mem::zeroed();
        ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.lpstrFilter = filtre.as_ptr();
        ofn.lpstrFile = buf.as_mut_ptr();
        ofn.nMaxFile = buf.len() as u32;
        ofn.lpstrTitle = titre.as_ptr();
        // Propriétaire = la fenêtre active (Waly, qui vient de recevoir le
        // clic) : sans lui le dialogue pouvait s'ouvrir DERRIÈRE l'app.
        ofn.hwndOwner = windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow();
        ofn.Flags = OFN_FILEMUSTEXIST | OFN_HIDEREADONLY;
        if GetOpenFileNameW(&mut ofn) == 0 {
            return None;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]))
    }
}

#[cfg(not(windows))]
pub fn chemin_service() -> Option<String> {
    None
}
#[cfg(not(windows))]
pub fn sceller_agent(_exe: &str) -> Result<bool, String> {
    Err("Windows uniquement".into())
}
#[cfg(not(windows))]
pub fn desceller_agent(_exe: &str) -> Result<(), String> {
    Err("Windows uniquement".into())
}
#[cfg(not(windows))]
pub fn choisir_exe() -> Option<String> {
    None
}
#[cfg(not(windows))]
pub fn choisir_fichier(_titre: &str, _filtre: &str) -> Option<String> {
    None
}

// --- Journal d'audit local (SQLite) ----------------------------------------

/// Une ligne d'audit lisible (pour la vue de l'app, chantier 3).
#[derive(Debug, serde::Serialize)]
pub struct LigneAudit {
    pub at: String,
    pub session: i64,
    pub genre: String,
    pub detail: String,
}

fn journaliser_evenement(c: &Connection, session: i64, genre: &str, detail: &str) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO audit_sceau (session_id, genre, detail) VALUES (?1, ?2, ?3)",
        rusqlite::params![session, genre, detail],
    )?;
    Ok(())
}

/// Inscrit au journal du sceau un fait que l'utilisateur doit pouvoir
/// retrouver (ex. une sortie OUVERTE à sa demande : téléchargement d'un
/// modèle par le moteur, modèle extérieur activé).
pub fn noter(c: &Connection, genre: &str, detail: &str) -> rusqlite::Result<()> {
    journaliser_evenement(c, PERIMETRE, genre, detail)
}

fn journaliser_sortie(c: &Connection, session: i64, d: &Sortie) -> rusqlite::Result<()> {
    // Déduplique sur (session, exe, adresse, port, t) : le service renvoie le
    // ring buffer complet à chaque drain.
    let detail = format!("{} -> {}:{} (proto {})", exe_court(&d.exe), d.adresse, d.port, d.proto);
    c.execute(
        "INSERT INTO audit_sceau (session_id, genre, detail, cle)
         VALUES (?1, 'bloque', ?2, ?3)
         ON CONFLICT(cle) DO NOTHING",
        rusqlite::params![
            session,
            detail,
            format!("{session}|{}|{}|{}|{}", d.exe, d.adresse, d.port, d.t)
        ],
    )?;
    Ok(())
}

/// Les lignes d'audit d'une session, plus récentes d'abord (vue de l'app).
pub fn lignes_audit(c: &Connection, session: i64, limite: i64) -> rusqlite::Result<Vec<LigneAudit>> {
    let mut st = c.prepare(
        "SELECT at, session_id, genre, detail FROM audit_sceau
         WHERE session_id = ?1 ORDER BY id DESC LIMIT ?2",
    )?;
    let rows = st.query_map(rusqlite::params![session, limite], |r| {
        Ok(LigneAudit {
            at: r.get(0)?,
            session: r.get(1)?,
            genre: r.get(2)?,
            detail: r.get(3)?,
        })
    })?;
    rows.collect()
}

fn exe_court(chemin: &str) -> String {
    chemin
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(chemin)
        .to_string()
}

// --- Transport named-pipe (Windows) ----------------------------------------

#[cfg(windows)]
fn appel(req: &Requete) -> Result<Reponse, String> {
    use std::io::{Read, Write};
    let mut corps = serde_json::to_vec(req).map_err(|e| e.to_string())?;
    corps.push(b'\n');

    // Le pipe peut être occupé (une instance à la fois sert) : retenter court.
    let mut f = None;
    for essai in 0..20 {
        match std::fs::OpenOptions::new().read(true).write(true).open(PIPE_NAME) {
            Ok(h) => {
                f = Some(h);
                break;
            }
            Err(e) => {
                // 2 = introuvable (service absent) : inutile de réessayer.
                if e.raw_os_error() == Some(2) {
                    return Err("service scelleur absent".into());
                }
                if essai == 19 {
                    return Err(format!("pipe: {e}"));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
    let mut f = f.ok_or("pipe indisponible")?;
    f.write_all(&corps).map_err(|e| e.to_string())?;
    f.flush().ok();

    let mut ligne = Vec::new();
    let mut b = [0u8; 1];
    loop {
        match f.read(&mut b) {
            Ok(0) => break,
            Ok(_) => {
                if b[0] == b'\n' {
                    break;
                }
                ligne.push(b[0]);
            }
            Err(e) => return Err(format!("lecture: {e}")),
        }
    }
    serde_json::from_slice(&ligne).map_err(|e| format!("réponse illisible: {e}"))
}

#[cfg(not(windows))]
fn appel(_req: &Requete) -> Result<Reponse, String> {
    Err("service scelleur : Windows uniquement".into())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chemin_reel_resolu_stable_et_tolerant() {
        let f = std::env::temp_dir().join(format!("waly-sceau-test-{}.exe", std::process::id()));
        std::fs::write(&f, b"x").unwrap();
        let reel = chemin_reel(&f.to_string_lossy());
        assert!(std::path::Path::new(&reel).exists());
        assert!(!reel.starts_with(r"\\?\"), "{reel}");
        // Résoudre deux fois ne change rien : c'est CE chemin qu'on déclare.
        assert_eq!(chemin_reel(&reel), reel);
        let _ = std::fs::remove_file(&f);
        // Introuvable : rendu tel quel, sans paniquer.
        assert_eq!(chemin_reel("introuvable/waly-x.exe"), "introuvable/waly-x.exe");
    }
}
