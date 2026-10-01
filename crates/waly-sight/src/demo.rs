//! « Regarde-moi » (B3) — apprendre en regardant l'utilisateur, en TEXTE.
//!
//! Plan B (`docs/PLAN-2026-09-11-B-agent-ecran.md`), banc `lab/ecran-banc`
//! (sous-commande `regarde-moi`). Compatible avec le mandat anti-Recall du
//! 20/07 parce que c'est l'INVERSE d'un enregistrement ambiant :
//! - une session que l'utilisateur OUVRE et FERME lui-même (indicateur
//!   visible côté app), bornée à [`DUREE_MAX_S`] ;
//! - aucun pixel : un clic devient « clic bouton « Enregistrer » — fenêtre
//!   « … » » (arbre d'accessibilité au point cliqué) ;
//! - le hook clavier ne retient QUE les raccourcis (Ctrl/Alt+…) et
//!   Entrée/Tab/Échap — un caractère tapé ne quitte JAMAIS le callback ; le
//!   texte saisi est relu dans le champ à la sortie du focus, sauf mot de
//!   passe (« •••• ») ;
//! - rien n'est écrit sur disque ici ; les clics sur les fenêtres de Waly
//!   (le bouton « Arrêter ») ne sont pas des étapes.

/// Borne dure d'une session (10 min) : au-delà elle s'arrête seule.
pub const DUREE_MAX_S: u64 = 600;
/// Au-delà, les étapes suivantes sont ignorées (une démonstration, pas un journal).
pub const ETAPES_MAX: usize = 200;

/// Fusionne les étapes consécutives identiques (double-clic, clics répétés)
/// en « … (×n) ». Pure, testée partout.
pub fn compacter(etapes: Vec<String>) -> Vec<String> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for e in etapes {
        match out.last_mut() {
            Some((prec, n)) if *prec == e => *n += 1,
            _ => out.push((e, 1)),
        }
    }
    out.into_iter()
        .map(|(e, n)| if n > 1 { format!("{e} (×{n})") } else { e })
        .collect()
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::uia::{court, role};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Accessibility::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_MENU, VK_SHIFT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetAncestor, PeekMessageW, SetWindowsHookExW,
        TranslateMessage, UnhookWindowsHookEx, WindowFromPoint, GA_ROOT, KBDLLHOOKSTRUCT, MSG,
        MSLLHOOKSTRUCT, PM_REMOVE, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_LBUTTONDOWN,
        WM_SYSKEYDOWN,
    };

    enum Evt {
        Clic(i32, i32),
        Touche(String),
    }

    /// Canal des callbacks de hook (fonctions `extern`, sans capture) — UNE
    /// session à la fois ; `None` hors session.
    static CANAL: Mutex<Option<Sender<Evt>>> = Mutex::new(None);

    fn envoyer(ev: Evt) {
        if let Ok(g) = CANAL.lock() {
            if let Some(tx) = g.as_ref() {
                let _ = tx.send(ev);
            }
        }
    }

    unsafe extern "system" fn hook_souris(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if code >= 0 && w.0 as u32 == WM_LBUTTONDOWN {
            let ms = &*(l.0 as *const MSLLHOOKSTRUCT);
            envoyer(Evt::Clic(ms.pt.x, ms.pt.y));
        }
        CallNextHookEx(None, code, w, l)
    }

    fn bas(vk: VIRTUAL_KEY) -> bool {
        unsafe { GetAsyncKeyState(vk.0 as i32) < 0 }
    }

    /// Raccourcis et touches de navigation SEULEMENT — jamais un caractère.
    unsafe extern "system" fn hook_clavier(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if code >= 0 && (w.0 as u32 == WM_KEYDOWN || w.0 as u32 == WM_SYSKEYDOWN) {
            let vk = (*(l.0 as *const KBDLLHOOKSTRUCT)).vkCode;
            let (ctrl, alt) = (bas(VK_CONTROL), bas(VK_MENU));
            let touche = match vk {
                0x0D => Some("Entrée".to_string()),
                0x09 => Some("Tab".to_string()),
                0x1B => Some("Échap".to_string()),
                0x70..=0x7B => Some(format!("F{}", vk - 0x6F)),
                0x30..=0x39 | 0x41..=0x5A if ctrl || alt => Some(((vk as u8) as char).to_string()),
                _ => None,
            };
            if let Some(t) = touche {
                let mut s = String::new();
                if ctrl {
                    s.push_str("Ctrl+");
                }
                if alt {
                    s.push_str("Alt+");
                }
                if bas(VK_SHIFT) && (ctrl || alt) {
                    s.push_str("Maj+");
                }
                s.push_str(&t);
                envoyer(Evt::Touche(s));
            }
        }
        CallNextHookEx(None, code, w, l)
    }

    /// Une session « regarde-moi » en cours.
    pub struct Session {
        stop: Arc<AtomicBool>,
        pompe: Option<JoinHandle<Result<Vec<String>, String>>>,
    }

    impl Session {
        /// Ouvre la session (hooks posés sur un thread de pompe dédié).
        pub fn demarrer() -> Result<Self, String> {
            {
                let mut g = CANAL.lock().map_err(|_| "canal empoisonné")?;
                if g.is_some() {
                    return Err("une session « regarde-moi » est déjà ouverte".into());
                }
                let (tx, rx) = channel::<Evt>();
                *g = Some(tx);
                let stop = Arc::new(AtomicBool::new(false));
                let s = stop.clone();
                let pompe = std::thread::spawn(move || pomper(rx, s));
                return Ok(Self { stop, pompe: Some(pompe) });
            }
        }

        /// Ferme la session et rend les étapes (texte seulement, compactées).
        pub fn arreter(mut self) -> Result<Vec<String>, String> {
            self.stop.store(true, Ordering::SeqCst);
            let h = self.pompe.take().ok_or("session déjà fermée")?;
            h.join().map_err(|_| "session en panique".to_string())?
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            // Filet : une session abandonnée ne laisse JAMAIS de hook posé.
            self.stop.store(true, Ordering::SeqCst);
            if let Some(h) = self.pompe.take() {
                let _ = h.join();
            }
        }
    }

    fn pomper(rx: Receiver<Evt>, stop: Arc<AtomicBool>) -> Result<Vec<String>, String> {
        let ouvrier = std::thread::spawn(move || observer(rx));
        let fin = Instant::now() + Duration::from_secs(DUREE_MAX_S);
        let pose = unsafe {
            (|| -> Result<_, String> {
                let hinst = GetModuleHandleW(None).map_err(|e| e.to_string())?;
                let hs = SetWindowsHookExW(WH_MOUSE_LL, Some(hook_souris), Some(hinst.into()), 0)
                    .map_err(|e| e.to_string())?;
                let hk = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_clavier), Some(hinst.into()), 0) {
                    Ok(h) => h,
                    Err(e) => {
                        let _ = UnhookWindowsHookEx(hs);
                        return Err(e.to_string());
                    }
                };
                Ok((hs, hk))
            })()
        };
        if let Ok((hs, hk)) = &pose {
            let mut msg = MSG::default();
            while !stop.load(Ordering::SeqCst) && Instant::now() < fin {
                unsafe {
                    while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            unsafe {
                let _ = UnhookWindowsHookEx(*hs);
                let _ = UnhookWindowsHookEx(*hk);
            }
        }
        // Fermer le canal : l'ouvrier fait sa dernière relecture et rend.
        if let Ok(mut g) = CANAL.lock() {
            *g = None;
        }
        let etapes = ouvrier.join().map_err(|_| "observateur en panique".to_string())?;
        pose?;
        Ok(compacter(etapes))
    }

    fn titre(h: HWND) -> String {
        crate::uia::titre(h.0 as isize)
    }

    unsafe fn decrire(el: &IUIAutomationElement) -> (String, bool) {
        let ct = el.CurrentControlType().map(|c| c.0).unwrap_or(0);
        let (r, _, _) = role(ct);
        let nom = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
        let mdp = el.CurrentIsPassword().map(|b| b.as_bool()).unwrap_or(false);
        let r = if r.is_empty() { "élément" } else { r };
        let d = if nom.trim().is_empty() { r.to_string() } else { format!("{r} « {} »", court(&nom, 60)) };
        (d, mdp)
    }

    unsafe fn valeur(el: &IUIAutomationElement) -> Option<String> {
        el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .ok()
            .and_then(|v| v.CurrentValue().ok())
            .map(|b| b.to_string())
    }

    type Focus = Option<(IUIAutomationElement, String, bool, Option<String>)>;

    /// Transforme les événements bruts en étapes texte (thread UIA dédié).
    fn observer(rx: Receiver<Evt>) -> Vec<String> {
        let u: IUIAutomation = unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            match CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
                Ok(u) => u,
                Err(_) => return Vec::new(),
            }
        };
        let mut etapes: Vec<String> = Vec::new();
        let mut note = |t: String| {
            if etapes.len() < ETAPES_MAX {
                etapes.push(t);
            }
        };
        let relire = |focus: &mut Focus, note: &mut dyn FnMut(String)| {
            if let Some((el, desc, mdp, v0)) = focus.take() {
                let v1 = unsafe { valeur(&el) };
                if v1 != v0 {
                    if mdp {
                        note(format!("saisie dans {desc} : •••• (mot de passe, non retenu)"));
                    } else if let Some(v) = v1 {
                        note(format!("saisie dans {desc} : « {} »", court(&v, 80)));
                    }
                }
            }
        };
        let mut focus: Focus = None;
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(Evt::Clic(x, y)) => unsafe {
                    let pt = POINT { x, y };
                    let fen = titre(GetAncestor(WindowFromPoint(pt), GA_ROOT));
                    if fen.starts_with("Waly") {
                        continue; // le bouton « Arrêter » n'est pas une étape
                    }
                    let fen = court(&fen, 50);
                    match u.ElementFromPoint(pt) {
                        Ok(el) => note(format!("clic {} — fenêtre « {fen} »", decrire(&el).0)),
                        Err(_) => note(format!("clic — fenêtre « {fen} »")),
                    }
                },
                Ok(Evt::Touche(t)) => {
                    // Un raccourci valide souvent une saisie : relire d'abord.
                    relire(&mut focus, &mut note);
                    note(format!("touche {t}"));
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if let Ok(f) = unsafe { u.GetFocusedElement() } {
                let meme = focus.as_ref().is_some_and(|(el, ..)| unsafe {
                    u.CompareElements(el, &f).map(|b| b.as_bool()).unwrap_or(false)
                });
                if !meme {
                    relire(&mut focus, &mut note);
                    let ct = unsafe { f.CurrentControlType().map(|c| c.0).unwrap_or(0) };
                    if matches!(ct, 50003 | 50004 | 50030) {
                        let (d, mdp) = unsafe { decrire(&f) };
                        let v0 = unsafe { valeur(&f) };
                        focus = Some((f, d, mdp, v0));
                    }
                }
            }
        }
        relire(&mut focus, &mut note);
        etapes
    }
}

#[cfg(windows)]
pub use imp::Session;

#[cfg(not(windows))]
pub struct Session;
#[cfg(not(windows))]
impl Session {
    pub fn demarrer() -> Result<Self, String> {
        Err("« regarde-moi » : Windows seulement".into())
    }
    pub fn arreter(self) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compacter_fusionne_les_repetitions_consecutives() {
        let e = |s: &str| s.to_string();
        let out = compacter(vec![e("clic A"), e("clic A"), e("touche Ctrl+S"), e("clic A")]);
        assert_eq!(out, vec![e("clic A (×2)"), e("touche Ctrl+S"), e("clic A")]);
        assert!(compacter(Vec::new()).is_empty());
    }
}
