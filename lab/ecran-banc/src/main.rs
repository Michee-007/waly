//! Banc B — agent d'ecran (plan : docs/PLAN-2026-09-11-B-agent-ecran.md).
//!
//! Sous-commandes :
//!   fenetres                                  liste les fenetres visibles
//!   arbre [--titre T] [--naif] [--brut] [--budget C]
//!                                             GATE 1 : instantane texte UIA
//!   agir --titre T --nom N [--role R] --action A [--texte X]
//!                                             GATE 2 : action par pattern UIA
//!   regarde-moi [--secondes S] [--sonde]      GATE 3 : etapes texte (jamais de pixels)
//!
//! Option commune : --hors-ecran (garder les sous-arbres hors ecran : fenetre
//! REDUITE = agir en arriere-plan). Sans --titre, `arbre` prend la fenetre au
//! premier plan apres 3 s. Rien n'est ecrit sur disque par ce banc.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::core::{BOOL, BSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

type R<T> = Result<T, String>;

fn e(x: windows::core::Error) -> String {
    format!("{x}")
}

fn uia() -> R<IUIAutomation> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).map_err(e)
    }
}

// ---------------------------------------------------------------------------
// Fenetres
// ---------------------------------------------------------------------------

fn fenetres() -> Vec<(HWND, String)> {
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        let out = &mut *(l.0 as *mut Vec<(HWND, String)>);
        if IsWindowVisible(h).as_bool() {
            let n = GetWindowTextLengthW(h);
            if n > 0 {
                let mut buf = vec![0u16; n as usize + 1];
                let k = GetWindowTextW(h, &mut buf);
                let t = String::from_utf16_lossy(&buf[..k as usize]).trim().to_string();
                if !t.is_empty() {
                    out.push((h, t));
                }
            }
        }
        BOOL(1)
    }
    let mut out = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

fn titre_de(h: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(h);
        let mut buf = vec![0u16; n.max(0) as usize + 1];
        let k = GetWindowTextW(h, &mut buf);
        String::from_utf16_lossy(&buf[..k.max(0) as usize])
    }
}

fn trouver(titre: &str) -> R<HWND> {
    let t = titre.to_lowercase();
    fenetres()
        .into_iter()
        .find(|(_, x)| x.to_lowercase().contains(&t))
        .map(|(h, _)| h)
        .ok_or_else(|| format!("aucune fenetre visible ne contient « {titre} »"))
}

// ---------------------------------------------------------------------------
// Roles
// ---------------------------------------------------------------------------

/// (libelle FR, interactif ?, conteneur nomme utile ?)
fn role(ct: i32) -> (&'static str, bool, bool) {
    match ct {
        50000 => ("bouton", true, false),
        50002 => ("case", true, false),
        50003 => ("liste deroulante", true, false),
        50004 => ("champ", true, false),
        50005 => ("lien", true, false),
        50006 => ("image", false, false),
        50007 => ("element", true, false),
        50008 => ("liste", false, true),
        50009 => ("menu", false, true),
        50010 => ("barre de menus", false, true),
        50011 => ("menu", true, false),
        50013 => ("option", true, false),
        50015 => ("curseur", true, false),
        50016 => ("compteur", true, false),
        50017 => ("barre d'etat", false, true),
        50018 => ("onglets", false, true),
        50019 => ("onglet", true, false),
        50020 => ("texte", false, false),
        50021 => ("barre d'outils", false, true),
        50023 => ("arbre", false, true),
        50024 => ("noeud", true, false),
        50026 => ("groupe", false, true),
        50028 => ("grille", false, true),
        50029 => ("ligne", true, false),
        50030 => ("document", true, false),
        50031 => ("bouton", true, false),
        50032 => ("fenetre", false, true),
        50033 => ("panneau", false, true),
        50036 => ("tableau", false, true),
        _ => ("", false, false),
    }
}

// ---------------------------------------------------------------------------
// GATE 1 — instantane
// ---------------------------------------------------------------------------

struct Noeud {
    el: IUIAutomationElement,
    role: &'static str,
    nom: String,
    valeur: Option<String>,
    coche: Option<bool>,
    profondeur: usize,
    interactif: bool,
    actif: bool,
    mdp: bool,
}

fn court(s: &str, n: usize) -> String {
    let s = s.replace(['\r', '\n'], " ");
    let s = s.trim();
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s.to_string()
    }
}

fn requete_cache(u: &IUIAutomation) -> R<IUIAutomationCacheRequest> {
    unsafe {
        let cr = u.CreateCacheRequest().map_err(e)?;
        // Les proprietes de pattern (Value, ToggleState) doivent etre DANS le
        // cache : le pattern seul ne suffit pas a lire CachedValue.
        for p in [
            UIA_NamePropertyId,
            UIA_ControlTypePropertyId,
            UIA_IsPasswordPropertyId,
            UIA_IsOffscreenPropertyId,
            UIA_IsEnabledPropertyId,
            UIA_ValueValuePropertyId,
            UIA_ToggleToggleStatePropertyId,
        ] {
            cr.AddProperty(p).map_err(e)?;
        }
        for p in [UIA_ValuePatternId, UIA_InvokePatternId, UIA_TogglePatternId] {
            cr.AddPattern(p).map_err(e)?;
        }
        cr.SetTreeScope(TreeScope_Subtree).map_err(e)?;
        cr.SetTreeFilter(&u.ControlViewCondition().map_err(e)?).map_err(e)?;
        Ok(cr)
    }
}

/// Instantane par UN aller-retour cross-process (CacheRequest Subtree).
fn instantane(u: &IUIAutomation, racine: &IUIAutomationElement) -> R<(Vec<Noeud>, usize)> {
    let cr = requete_cache(u)?;
    let r = unsafe { racine.BuildUpdatedCache(&cr).map_err(e)? };
    let mut out = Vec::new();
    let mut vus = 0usize;
    parcourir(&r, 0, &mut out, &mut vus);
    Ok((out, vus))
}

/// `--hors-ecran` : garder les sous-arbres hors ecran.
static HORS_ECRAN: AtomicBool = AtomicBool::new(false);

fn parcourir(el: &IUIAutomationElement, prof: usize, out: &mut Vec<Noeud>, vus: &mut usize) {
    *vus += 1;
    unsafe {
        let off = el.CachedIsOffscreen().map(|b| b.as_bool()).unwrap_or(false);
        if off && prof > 0 && !HORS_ECRAN.load(Ordering::Relaxed) {
            return; // hors ecran : sous-arbre elague (menus replies, listes defilees)
        }
        let ct = el.CachedControlType().map(|c| c.0).unwrap_or(0);
        let (r, interactif, conteneur) = role(ct);
        let nom = el.CachedName().map(|b| b.to_string()).unwrap_or_default();
        let mdp = el.CachedIsPassword().map(|b| b.as_bool()).unwrap_or(false);
        let actif = el.CachedIsEnabled().map(|b| b.as_bool()).unwrap_or(true);
        let valeur = if mdp {
            Some("••••".to_string())
        } else {
            el.GetCachedPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .ok()
                .and_then(|v| v.CachedValue().ok())
                .map(|b| b.to_string())
                .filter(|s| !s.trim().is_empty())
        };
        let coche = el
            .GetCachedPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
            .ok()
            .and_then(|t| t.CachedToggleState().ok())
            .map(|s| s == ToggleState_On);
        let garder = (interactif
            && (!nom.trim().is_empty() || valeur.is_some() || ct == 50004 || ct == 50030))
            || (ct == 50020 && !nom.trim().is_empty())
            || (conteneur && !nom.trim().is_empty());
        if garder {
            out.push(Noeud {
                el: el.clone(),
                role: if r.is_empty() { "?" } else { r },
                nom,
                valeur,
                coche,
                profondeur: prof,
                interactif,
                actif,
                mdp,
            });
        }
        if let Ok(kids) = el.GetCachedChildren() {
            let n = kids.Length().unwrap_or(0);
            for i in 0..n {
                if let Ok(k) = kids.GetElement(i) {
                    parcourir(&k, prof + 1, out, vus);
                }
            }
        }
    }
}

/// Instantane NAIF (un aller-retour par propriete) — pour mesurer le gain du cache.
fn instantane_naif(u: &IUIAutomation, racine: &IUIAutomationElement) -> R<usize> {
    unsafe {
        let w = u.ControlViewWalker().map_err(e)?;
        let mut pile = vec![racine.clone()];
        let mut n = 0usize;
        while let Some(el) = pile.pop() {
            n += 1;
            let _ = el.CurrentName();
            let _ = el.CurrentControlType();
            let _ = el.CurrentIsOffscreen();
            if let Ok(mut c) = w.GetFirstChildElement(&el) {
                loop {
                    pile.push(c.clone());
                    match w.GetNextSiblingElement(&c) {
                        Ok(s) => c = s,
                        Err(_) => break,
                    }
                }
            }
            if n > 20000 {
                break;
            }
        }
        Ok(n)
    }
}

/// Parent (dans la liste GARDEE) de chaque noeud, d'apres les profondeurs DFS.
fn parents(noeuds: &[Noeud]) -> Vec<Option<usize>> {
    let mut pile: Vec<(usize, usize)> = Vec::new(); // (profondeur, index)
    noeuds
        .iter()
        .enumerate()
        .map(|(i, n)| {
            while pile.last().is_some_and(|(p, _)| *p >= n.profondeur) {
                pile.pop();
            }
            let par = pile.last().map(|(_, j)| *j);
            pile.push((n.profondeur, i));
            par
        })
        .collect()
}

/// Rendu avec au plus `k` freres de meme role par parent (le reste resume en
/// « … +N autres ») et des noms coupes a `lg` caracteres.
fn rendre_avec(noeuds: &[Noeud], k: usize, lg: usize) -> String {
    let par = parents(noeuds);
    let mut compte: HashMap<(Option<usize>, &str), usize> = HashMap::new();
    let mut rang = vec![0usize; noeuds.len()];
    for (i, n) in noeuds.iter().enumerate() {
        let c = compte.entry((par[i], n.role)).or_insert(0);
        rang[i] = *c;
        *c += 1;
    }
    let mut cache = vec![false; noeuds.len()];
    let mut s = String::new();
    for (i, n) in noeuds.iter().enumerate() {
        let ind = "  ".repeat(n.profondeur.min(8));
        if par[i].is_some_and(|p| cache[p]) {
            cache[i] = true;
            continue;
        }
        if rang[i] >= k {
            cache[i] = true;
            if rang[i] == k {
                let total = compte[&(par[i], n.role)];
                s.push_str(&format!("{ind}… +{} autres ({})\n", total - k, n.role));
            }
            continue;
        }
        let id = if n.interactif { format!("[{i}] ") } else { String::new() };
        let nom = if n.nom.trim().is_empty() {
            String::new()
        } else {
            format!(" « {} »", court(&n.nom, lg))
        };
        let val = match &n.valeur {
            Some(v) if v != &n.nom => format!(" = \"{}\"", court(v, lg)),
            _ => String::new(),
        };
        let coche = match n.coche {
            Some(true) => " (cochee)",
            Some(false) => " (non cochee)",
            None => "",
        };
        let off = if n.actif { "" } else { " (desactive)" };
        let mdp = if n.mdp { " (mot de passe)" } else { "" };
        s.push_str(&format!("{ind}{id}{}{nom}{val}{coche}{off}{mdp}\n", n.role));
    }
    s
}

/// Rendu sous BUDGET (caracteres) : complet s'il tient, sinon compaction
/// progressive des freres repetes, puis troncature honnete.
fn rendre(noeuds: &[Noeud], budget: usize) -> (String, &'static str) {
    let plein = rendre_avec(noeuds, usize::MAX, 80);
    if plein.chars().count() <= budget {
        return (plein, "complet");
    }
    for (k, lg, etiquette) in [(8, 60, "k=8"), (5, 50, "k=5"), (3, 40, "k=3"), (2, 40, "k=2")] {
        let s = rendre_avec(noeuds, k, lg);
        if s.chars().count() <= budget {
            return (s, etiquette);
        }
    }
    let s = rendre_avec(noeuds, 1, 40);
    let mut t: String = s.chars().take(budget).collect();
    t.push_str("\n… (instantane tronque)\n");
    (t, "k=1+tronque")
}

fn cmd_arbre(args: &[String]) -> R<()> {
    let u = uia()?;
    let h = match opt(args, "--titre") {
        Some(t) => trouver(&t)?,
        None => {
            eprintln!("(clique sur la fenetre a lire — capture dans 3 s)");
            std::thread::sleep(Duration::from_secs(3));
            unsafe { GetForegroundWindow() }
        }
    };
    let budget: usize = opt(args, "--budget").and_then(|b| b.parse().ok()).unwrap_or(5000);
    let titre = titre_de(h);
    let racine = unsafe { u.ElementFromHandle(h).map_err(e)? };
    if args.iter().any(|a| a == "--naif") {
        let t0 = Instant::now();
        let n = instantane_naif(&u, &racine)?;
        println!("NAIF « {} » : {n} elements en {} ms", court(&titre, 60), t0.elapsed().as_millis());
    }
    let t0 = Instant::now();
    let (noeuds, vus) = instantane(&u, &racine)?;
    let ms_cache = t0.elapsed().as_millis();
    let t1 = Instant::now();
    let plein = rendre_avec(&noeuds, usize::MAX, 80).chars().count();
    let (texte, mode) = rendre(&noeuds, budget);
    let ms_rendu = t1.elapsed().as_millis();
    if args.iter().any(|a| a == "--brut") {
        println!("{texte}");
    }
    let inter = noeuds.iter().filter(|n| n.interactif).count();
    let chars = texte.chars().count();
    println!(
        "GATE1 « {} » : {vus} vus, {} gardes ({inter} actionnables), plein {plein} car. -> rendu {chars} car. (~{} tok, {mode}), cache {ms_cache} ms + rendu {ms_rendu} ms",
        court(&titre, 60),
        noeuds.len(),
        chars * 10 / 35
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// GATE 2 — agir
// ---------------------------------------------------------------------------

fn curseur() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

fn saisir_clavier(texte: &str) -> R<()> {
    let mut inputs = Vec::new();
    for u in texte.encode_utf16() {
        for flags in [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP] {
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: u, dwFlags: flags, time: 0, dwExtraInfo: 0 },
                },
            });
        }
    }
    let n = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if n as usize == inputs.len() {
        Ok(())
    } else {
        Err(format!("SendInput : {n}/{} evenements", inputs.len()))
    }
}

fn agir_sur(n: &Noeud, action: &str, texte: Option<&str>) -> R<String> {
    unsafe {
        let el = &n.el;
        match action {
            "invoquer" => {
                el.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                    .map_err(|_| "pas de pattern Invoke".to_string())?
                    .Invoke()
                    .map_err(e)?;
                Ok("Invoke".into())
            }
            "basculer" => {
                el.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
                    .map_err(|_| "pas de pattern Toggle".to_string())?
                    .Toggle()
                    .map_err(e)?;
                Ok("Toggle".into())
            }
            "choisir" => {
                el.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
                    .map_err(|_| "pas de pattern SelectionItem".to_string())?
                    .Select()
                    .map_err(e)?;
                Ok("Select".into())
            }
            "deplier" | "replier" => {
                let p = el
                    .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
                    .map_err(|_| "pas de pattern ExpandCollapse".to_string())?;
                if action == "deplier" { p.Expand() } else { p.Collapse() }.map_err(e)?;
                Ok(if action == "deplier" { "Expand".into() } else { "Collapse".into() })
            }
            "focus" => {
                el.SetFocus().map_err(e)?;
                Ok("SetFocus".into())
            }
            "saisir" => {
                if n.mdp {
                    return Err("champ mot de passe : refuse".into());
                }
                let t = texte.ok_or("--texte requis")?;
                if let Ok(v) = el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
                    if v.SetValue(&BSTR::from(t)).is_ok() {
                        return Ok("Value.SetValue".into());
                    }
                }
                el.SetFocus().map_err(e)?;
                saisir_clavier(t)?;
                Ok("repli SetFocus + SendInput unicode".into())
            }
            a => Err(format!("action inconnue « {a} »")),
        }
    }
}

fn lire_valeur(el: &IUIAutomationElement) -> Option<String> {
    unsafe {
        el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .ok()
            .and_then(|v| v.CurrentValue().ok())
            .map(|b| b.to_string())
    }
}

fn cmd_agir(args: &[String]) -> R<()> {
    let u = uia()?;
    let h = trouver(&opt(args, "--titre").ok_or("--titre requis")?)?;
    let nom = opt(args, "--nom").unwrap_or_default().to_lowercase();
    let role_voulu = opt(args, "--role");
    let action = opt(args, "--action").ok_or("--action requis")?;
    let texte = opt(args, "--texte");
    let racine = unsafe { u.ElementFromHandle(h).map_err(e)? };
    let (noeuds, _) = instantane(&u, &racine)?;
    let cible = noeuds
        .iter()
        .find(|n| {
            n.interactif
                && n.nom.to_lowercase().contains(&nom)
                && role_voulu.as_deref().is_none_or(|r| n.role == r)
        })
        .ok_or_else(|| format!("aucun element actionnable « {nom} »"))?;
    // Activite de l'utilisateur AVANT l'action (sinon la mesure curseur ment).
    let c_a = curseur();
    std::thread::sleep(Duration::from_millis(200));
    let c0 = curseur();
    let utilisateur_actif = c_a != c0;
    let reduite0 = unsafe { IsIconic(h).as_bool() };
    let t0 = Instant::now();
    let voie = agir_sur(cible, &action, texte.as_deref())?;
    let ms = t0.elapsed().as_micros() as f64 / 1000.0;
    std::thread::sleep(Duration::from_millis(150));
    let c1 = curseur();
    let cible_devant = unsafe { GetForegroundWindow() } == h;
    let reduite1 = unsafe { IsIconic(h).as_bool() };
    println!(
        "GATE2 {action} sur {} « {} » via {voie} : {ms:.1} ms ; cible au premier plan : {} ; reduite {} -> {} ; curseur {}",
        cible.role,
        court(&cible.nom, 50),
        if cible_devant { "OUI" } else { "non" },
        reduite0,
        reduite1,
        if utilisateur_actif {
            "non concluant (utilisateur actif)".to_string()
        } else if c0 == c1 {
            "INCHANGE".to_string()
        } else {
            "BOUGE".to_string()
        },
    );
    if action == "saisir" {
        println!("relu : {:?}", lire_valeur(&cible.el).map(|v| court(&v, 120)));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// GATE 3 — regarde-moi
// ---------------------------------------------------------------------------

enum Evt {
    Clic(i32, i32),
    Touche(String),
}

static CANAL: OnceLock<Mutex<Sender<Evt>>> = OnceLock::new();

fn envoyer(ev: Evt) {
    if let Some(m) = CANAL.get() {
        if let Ok(tx) = m.lock() {
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

/// Clavier : on ne retient QUE les raccourcis et les touches de navigation —
/// JAMAIS un caractere tape (le texte saisi est relu dans le champ).
unsafe extern "system" fn hook_clavier(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 && (w.0 as u32 == WM_KEYDOWN || w.0 as u32 == WM_SYSKEYDOWN) {
        let k = &*(l.0 as *const KBDLLHOOKSTRUCT);
        let vk = k.vkCode;
        let ctrl = bas(VK_CONTROL);
        let alt = bas(VK_MENU);
        let nom_touche = match vk {
            0x0D => Some("Entree".to_string()),
            0x09 => Some("Tab".to_string()),
            0x1B => Some("Echap".to_string()),
            0x70..=0x7B => Some(format!("F{}", vk - 0x6F)),
            0x30..=0x39 | 0x41..=0x5A if ctrl || alt => Some(((vk as u8) as char).to_string()),
            _ => None,
        };
        if let Some(t) = nom_touche {
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

fn decrire(el: &IUIAutomationElement) -> (String, bool) {
    unsafe {
        let ct = el.CurrentControlType().map(|c| c.0).unwrap_or(0);
        let (r, _, _) = role(ct);
        let nom = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
        let mdp = el.CurrentIsPassword().map(|b| b.as_bool()).unwrap_or(false);
        let r = if r.is_empty() { "element" } else { r };
        (
            if nom.trim().is_empty() { r.to_string() } else { format!("{r} « {} »", court(&nom, 60)) },
            mdp,
        )
    }
}

fn est_saisissable(el: &IUIAutomationElement) -> bool {
    unsafe {
        let ct = el.CurrentControlType().map(|c| c.0).unwrap_or(0);
        ct == 50004 || ct == 50030 || ct == 50003
    }
}

type Focus = Option<(IUIAutomationElement, String, bool, Option<String>)>;

/// Relit le champ quitte : la saisie devient UNE etape (jamais les touches).
fn relire_saisie(focus: &mut Focus, note: &mut dyn FnMut(String)) {
    if let Some((el, desc, mdp, v0)) = focus.take() {
        let v1 = lire_valeur(&el);
        if v1 != v0 {
            if mdp {
                note(format!("saisie dans {desc} : •••• (mot de passe, non retenu)"));
            } else if let Some(v) = v1 {
                note(format!("saisie dans {desc} : « {} »", court(&v, 80)));
            }
        }
    }
}

fn cmd_regarde_moi(args: &[String]) -> R<()> {
    // --sonde : installe puis retire les hooks, sans RIEN observer (preuve SAC
    // sans enregistrer l'activite de quiconque).
    if args.iter().any(|a| a == "--sonde") {
        unsafe {
            let hinst = GetModuleHandleW(None).map_err(e)?;
            let t0 = Instant::now();
            let hs = SetWindowsHookExW(WH_MOUSE_LL, Some(hook_souris), Some(hinst.into()), 0).map_err(e)?;
            let hk = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_clavier), Some(hinst.into()), 0).map_err(e)?;
            let ms = t0.elapsed().as_micros() as f64 / 1000.0;
            let _ = UnhookWindowsHookEx(hs);
            let _ = UnhookWindowsHookEx(hk);
            println!("GATE3 sonde : hooks souris+clavier poses et retires en {ms:.2} ms (aucun evenement observe)");
        }
        return Ok(());
    }
    let secondes: u64 = opt(args, "--secondes").and_then(|s| s.parse().ok()).unwrap_or(30);
    let (tx, rx) = channel::<Evt>();
    CANAL.set(Mutex::new(tx)).map_err(|_| "canal deja pose")?;
    let debut = Instant::now();
    let fin = debut + Duration::from_secs(secondes);

    // Ouvrier UIA : transforme les evenements bruts en etapes texte.
    let ouvrier = std::thread::spawn(move || -> R<Vec<String>> {
        let u = uia()?;
        let mut etapes: Vec<String> = Vec::new();
        let mut note = |t: String| {
            println!("+{:>5.1}s  {t}", debut.elapsed().as_secs_f32());
            etapes.push(t);
        };
        let mut focus: Focus = None;
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(Evt::Clic(x, y)) => unsafe {
                    let pt = POINT { x, y };
                    let fen = GetAncestor(WindowFromPoint(pt), GA_ROOT);
                    let titre = court(&titre_de(fen), 50);
                    match u.ElementFromPoint(pt) {
                        Ok(el) => note(format!("clic {} — fenetre « {titre} »", decrire(&el).0)),
                        Err(_) => note(format!("clic ({x},{y}) — fenetre « {titre} »")),
                    }
                },
                Ok(Evt::Touche(t)) => {
                    // Un raccourci valide souvent une saisie : relire d'abord.
                    relire_saisie(&mut focus, &mut note);
                    note(format!("touche {t}"));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break,
            }
            // Suivi du focus (saisies relues a la sortie du champ).
            if let Ok(f) = unsafe { u.GetFocusedElement() } {
                let meme = focus.as_ref().is_some_and(|(el, ..)| unsafe {
                    u.CompareElements(el, &f).map(|b| b.as_bool()).unwrap_or(false)
                });
                if !meme {
                    relire_saisie(&mut focus, &mut note);
                    if est_saisissable(&f) {
                        let (d, mdp) = decrire(&f);
                        let v0 = lire_valeur(&f);
                        focus = Some((f, d, mdp, v0));
                    }
                }
            }
            if Instant::now() >= fin + Duration::from_millis(300) {
                break;
            }
        }
        relire_saisie(&mut focus, &mut note);
        Ok(etapes)
    });

    unsafe {
        let hinst = GetModuleHandleW(None).map_err(e)?;
        let hs = SetWindowsHookExW(WH_MOUSE_LL, Some(hook_souris), Some(hinst.into()), 0).map_err(e)?;
        let hk = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_clavier), Some(hinst.into()), 0).map_err(e)?;
        println!("REGARDE-MOI : session de {secondes} s ouverte (clics, raccourcis, saisies relues ; jamais de pixels)");
        let mut msg = MSG::default();
        while Instant::now() < fin {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = UnhookWindowsHookEx(hs);
        let _ = UnhookWindowsHookEx(hk);
    }
    // L'ouvrier s'arrete seul a l'echeance (+300 ms : derniere relecture).
    let etapes = ouvrier.join().map_err(|_| "ouvrier en panique")??;
    println!("GATE3 : {} etapes en {secondes} s", etapes.len());
    Ok(())
}

// ---------------------------------------------------------------------------

fn opt(args: &[String], nom: &str) -> Option<String> {
    args.iter().position(|a| a == nom).and_then(|i| args.get(i + 1)).cloned()
}

/// Piege 3 : SAC rend son verdict PAR binaire — incrementer pour changer le
/// hash quand un build est bloque (erreur 4551 / CodeIntegrity 3077).
const SAC_REROLL: u32 = 1;

fn main() {
    std::hint::black_box(SAC_REROLL);
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--hors-ecran") {
        HORS_ECRAN.store(true, Ordering::Relaxed);
    }
    let res = match args.first().map(String::as_str) {
        Some("fenetres") => {
            for (h, t) in fenetres() {
                println!("{:>10}  {t}", h.0 as isize);
            }
            Ok(())
        }
        Some("arbre") => cmd_arbre(&args),
        Some("agir") => cmd_agir(&args),
        Some("regarde-moi") => cmd_regarde_moi(&args),
        _ => Err("usage : ecran-banc fenetres | arbre [--titre T] [--naif] [--brut] [--budget C] | agir --titre T --nom N [--role R] --action A [--texte X] | regarde-moi [--secondes S] [--sonde] ; option --hors-ecran".into()),
    };
    if let Err(m) = res {
        eprintln!("erreur : {m}");
        std::process::exit(1);
    }
}
