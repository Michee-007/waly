//! L'écran en TEXTE — lecture et action par l'arbre d'accessibilité Windows
//! (UI Automation). Plan B : `docs/PLAN-2026-09-11-B-agent-ecran.md`, banc :
//! `lab/ecran-banc/` (gates 1-2 vertes le 2026-09-11).
//!
//! Pourquoi le texte d'abord : un tour visuel coûte 86 s sur la machine de
//! référence (vision déléguée), un tour texte ~3,5 s. UIA donne le nom exact
//! de chaque bouton/champ/onglet, sa valeur, son état — et des poignées pour
//! AGIR (Invoke, Value, Toggle, SelectionItem, ExpandCollapse) sans bouger la
//! souris ni voler le premier plan (mesuré : fenêtre réduite, jamais activée).
//!
//! Vie privée : rien n'est persisté ici ; un champ mot de passe n'est JAMAIS
//! lu (rendu `••••`) ni écrit. COM sur `uiautomationcore.dll`, DLL système
//! signée (piège 3 : jamais d'exe tiers).
//!
//! La partie pure (rôles, rendu sous budget, identité d'un élément) compile et
//! se teste partout ; la partie COM est `cfg(windows)`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Budget de rendu par défaut (caractères, ~1 430 tokens) — GATE 1 : tenu
/// partout après compaction, pire cas Zoom 3 570 éléments → 4 581 car.
pub const BUDGET_DEFAUT: usize = 5000;

/// Un élément GARDÉ de l'arbre (interactif nommé, texte, conteneur nommé).
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub role: &'static str,
    pub nom: String,
    pub valeur: Option<String>,
    pub coche: Option<bool>,
    pub profondeur: usize,
    pub interactif: bool,
    pub actif: bool,
    pub mdp: bool,
}

/// Instantané d'une fenêtre : le texte vu par le modèle et la table des
/// éléments (l'id `[n]` du texte = index dans `elements`).
#[derive(Clone, Debug)]
pub struct Instantane {
    pub hwnd: isize,
    pub fenetre: String,
    pub elements: Vec<Element>,
    pub texte: String,
    /// « complet », « k=8 »… : la compaction appliquée (honnêteté du rendu).
    pub mode: &'static str,
}

/// Identité STABLE d'un élément, persistable dans une approbation : on ne
/// garde jamais de poignée COM entre la demande et l'exécution — l'élément
/// est re-cherché (même fenêtre, même rôle, même nom, même rang) au moment
/// d'agir. Introuvable ⟹ aucune action (garde « pas d'aveugle »).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cible {
    pub hwnd: isize,
    pub fenetre: String,
    pub role: String,
    pub nom: String,
    pub rang: usize,
}

/// Les gestes possibles (un par pattern UIA).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Cliquer,
    Saisir,
    Cocher,
    Choisir,
    Deplier,
    Replier,
}

impl Action {
    pub const NOMS: [&'static str; 6] = ["cliquer", "saisir", "cocher", "choisir", "deplier", "replier"];

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_lowercase().as_str() {
            "cliquer" | "invoquer" | "clic" => Self::Cliquer,
            "saisir" | "ecrire" | "écrire" | "taper" => Self::Saisir,
            "cocher" | "decocher" | "décocher" | "basculer" => Self::Cocher,
            "choisir" | "selectionner" | "sélectionner" => Self::Choisir,
            "deplier" | "déplier" | "ouvrir" => Self::Deplier,
            "replier" | "fermer" => Self::Replier,
            _ => return None,
        })
    }

    /// Verbe humain pour le libellé d'approbation.
    pub fn verbe(self) -> &'static str {
        match self {
            Self::Cliquer => "cliquer sur",
            Self::Saisir => "écrire dans",
            Self::Cocher => "cocher/décocher",
            Self::Choisir => "choisir",
            Self::Deplier => "déplier",
            Self::Replier => "replier",
        }
    }
}

impl Instantane {
    /// Résout l'id `[n]` du texte en identité stable.
    pub fn cible(&self, id: usize) -> Result<Cible, String> {
        let el = self
            .elements
            .get(id)
            .filter(|e| e.interactif)
            .ok_or_else(|| format!("aucun élément actionnable [{id}] dans la dernière lecture — relis l'écran"))?;
        let rang = self.elements[..id]
            .iter()
            .filter(|e| e.interactif && e.role == el.role && e.nom == el.nom)
            .count();
        Ok(Cible {
            hwnd: self.hwnd,
            fenetre: self.fenetre.clone(),
            role: el.role.to_string(),
            nom: el.nom.clone(),
            rang,
        })
    }

    pub fn element(&self, id: usize) -> Option<&Element> {
        self.elements.get(id)
    }
}

/// (libellé FR, interactif ?, conteneur nommé utile ?) d'un ControlType UIA.
pub fn role(ct: i32) -> (&'static str, bool, bool) {
    match ct {
        50000 | 50031 => ("bouton", true, false),
        50002 => ("case", true, false),
        50003 => ("liste déroulante", true, false),
        50004 => ("champ", true, false),
        50005 => ("lien", true, false),
        50007 => ("élément", true, false),
        50008 => ("liste", false, true),
        50009 => ("menu", false, true),
        50010 => ("barre de menus", false, true),
        50011 => ("menu", true, false),
        50013 => ("option", true, false),
        50015 => ("curseur", true, false),
        50016 => ("compteur", true, false),
        50017 => ("barre d'état", false, true),
        50018 => ("onglets", false, true),
        50019 => ("onglet", true, false),
        50020 => ("texte", false, false),
        50021 => ("barre d'outils", false, true),
        50023 => ("arbre", false, true),
        50024 => ("nœud", true, false),
        50026 => ("groupe", false, true),
        50028 => ("grille", false, true),
        50029 => ("ligne", true, false),
        50030 => ("document", true, false),
        50032 => ("fenêtre", false, true),
        50033 => ("panneau", false, true),
        50036 => ("tableau", false, true),
        _ => ("", false, false),
    }
}

/// Règle d'élagage (pure) : qu'est-ce qui mérite une ligne ?
pub fn garder(ct: i32, nom: &str, a_valeur: bool) -> bool {
    let (_, interactif, conteneur) = role(ct);
    let nomme = !nom.trim().is_empty();
    (interactif && (nomme || a_valeur || ct == 50004 || ct == 50030))
        || (ct == 50020 && nomme)
        || (conteneur && nomme)
}

pub fn court(s: &str, n: usize) -> String {
    let s = s.replace(['\r', '\n'], " ");
    let s = s.trim();
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s.to_string()
    }
}

/// Parent (dans la liste gardée) de chaque élément, d'après les profondeurs DFS.
fn parents(els: &[Element]) -> Vec<Option<usize>> {
    let mut pile: Vec<(usize, usize)> = Vec::new();
    els.iter()
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

/// Rendu avec au plus `k` frères de même rôle par parent (le reste résumé en
/// « … +N autres ») et des noms coupés à `lg` caractères.
fn rendre_avec(els: &[Element], k: usize, lg: usize) -> String {
    let par = parents(els);
    let mut compte: HashMap<(Option<usize>, &str), usize> = HashMap::new();
    let mut rang = vec![0usize; els.len()];
    for (i, n) in els.iter().enumerate() {
        let c = compte.entry((par[i], n.role)).or_insert(0);
        rang[i] = *c;
        *c += 1;
    }
    let mut cache = vec![false; els.len()];
    let mut s = String::new();
    for (i, n) in els.iter().enumerate() {
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
            Some(true) => " (cochée)",
            Some(false) => " (non cochée)",
            None => "",
        };
        let off = if n.actif { "" } else { " (désactivé)" };
        let mdp = if n.mdp { " (mot de passe)" } else { "" };
        s.push_str(&format!("{ind}{id}{}{nom}{val}{coche}{off}{mdp}\n", n.role));
    }
    s
}

/// Rendu sous BUDGET (caractères) : complet s'il tient, sinon compaction
/// progressive des frères répétés, puis troncature honnête.
pub fn rendre(els: &[Element], budget: usize) -> (String, &'static str) {
    let plein = rendre_avec(els, usize::MAX, 80);
    if plein.chars().count() <= budget {
        return (plein, "complet");
    }
    for (k, lg, etiquette) in [(8, 60, "k=8"), (5, 50, "k=5"), (3, 40, "k=3"), (2, 40, "k=2")] {
        let s = rendre_avec(els, k, lg);
        if s.chars().count() <= budget {
            return (s, etiquette);
        }
    }
    let s = rendre_avec(els, 1, 40);
    let mut t: String = s.chars().take(budget).collect();
    t.push_str("\n… (lecture tronquée)\n");
    (t, "tronqué")
}

/// Une lecture est-elle assez riche pour se passer de l'image ? (sinon :
/// canvas, PDF image, jeu → repli OCR/VLM du mode Écran R5).
pub fn est_riche(inst: &Instantane) -> bool {
    let actionnables = inst.elements.iter().filter(|e| e.interactif).count();
    let textes: usize = inst.elements.iter().map(|e| e.nom.len()).sum();
    actionnables >= 4 || textes >= 200
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::{Interface, BOOL, BSTR};
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
        VIRTUAL_KEY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, IsIconic,
        IsWindow, IsWindowVisible, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
    };

    fn err(e: windows::core::Error) -> String {
        format!("{e}")
    }

    /// Le lecteur UIA — vit sur UN thread (COM, `!Send`) : le worker desktop.
    pub struct Lecteur {
        u: IUIAutomation,
    }

    impl Lecteur {
        pub fn new() -> Result<Self, String> {
            unsafe {
                // MTA ; si le thread est déjà STA l'appel échoue sans gravité
                // (les appels synchrones UIA marchent dans les deux).
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                let u: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
                    .or_else(|_| CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER))
                    .map_err(err)?;
                // Une appli figée ne doit pas figer Waly : délais bornés.
                if let Ok(u2) = u.cast::<IUIAutomation2>() {
                    let _ = u2.SetConnectionTimeout(2000);
                    let _ = u2.SetTransactionTimeout(4000);
                }
                Ok(Self { u })
            }
        }

        fn requete(&self) -> Result<IUIAutomationCacheRequest, String> {
            unsafe {
                let cr = self.u.CreateCacheRequest().map_err(err)?;
                // Les propriétés de pattern DOIVENT être dans le cache (le
                // pattern seul ne suffit pas à lire CachedValue — vécu banc).
                for p in [
                    UIA_NamePropertyId,
                    UIA_ControlTypePropertyId,
                    UIA_IsPasswordPropertyId,
                    UIA_IsOffscreenPropertyId,
                    UIA_IsEnabledPropertyId,
                    UIA_ValueValuePropertyId,
                    UIA_ToggleToggleStatePropertyId,
                ] {
                    cr.AddProperty(p).map_err(err)?;
                }
                for p in [UIA_ValuePatternId, UIA_TogglePatternId] {
                    cr.AddPattern(p).map_err(err)?;
                }
                cr.SetTreeScope(TreeScope_Subtree).map_err(err)?;
                cr.SetTreeFilter(&self.u.ControlViewCondition().map_err(err)?).map_err(err)?;
                Ok(cr)
            }
        }

        /// Arbre gardé d'une fenêtre, par UN aller-retour cross-process. Les
        /// sous-arbres hors écran sont élagués, SAUF si la fenêtre est
        /// réduite (agir en arrière-plan : tout y est « hors écran »).
        fn noeuds(&self, h: HWND) -> Result<Vec<(IUIAutomationElement, Element)>, String> {
            unsafe {
                let hors_ecran = IsIconic(h).as_bool();
                let racine = self.u.ElementFromHandle(h).map_err(err)?;
                let r = racine.BuildUpdatedCache(&self.requete()?).map_err(err)?;
                let mut out = Vec::new();
                parcourir(&r, 0, hors_ecran, &mut out);
                Ok(out)
            }
        }

        /// Lit une fenêtre et rend son texte sous budget.
        pub fn lire(&self, hwnd: isize, budget: usize) -> Result<Instantane, String> {
            let h = HWND(hwnd as *mut core::ffi::c_void);
            if unsafe { !IsWindow(Some(h)).as_bool() } {
                return Err("cette fenêtre n'existe plus".into());
            }
            let elements: Vec<Element> = self.noeuds(h)?.into_iter().map(|(_, e)| e).collect();
            let (texte, mode) = rendre(&elements, budget);
            Ok(Instantane { hwnd, fenetre: titre(hwnd), elements, texte, mode })
        }

        /// Agit sur une cible RE-CHERCHÉE maintenant (jamais une poignée
        /// ancienne). Retourne le constat relu après l'action.
        pub fn agir(&self, c: &Cible, action: Action, texte: Option<&str>) -> Result<String, String> {
            let h = HWND(c.hwnd as *mut core::ffi::c_void);
            if unsafe { !IsWindow(Some(h)).as_bool() } {
                return Err(format!("la fenêtre « {} » est fermée — aucune action faite", c.fenetre));
            }
            let noeuds = self.noeuds(h)?;
            let (el, info) = noeuds
                .iter()
                .filter(|(_, e)| e.interactif && e.role == c.role && e.nom == c.nom)
                .nth(c.rang)
                .ok_or("l'élément a changé ou disparu — relis l'écran (aucune action faite)")?;
            if !info.actif {
                return Err(format!("{} « {} » est désactivé — aucune action faite", info.role, court(&info.nom, 60)));
            }
            unsafe {
                match action {
                    Action::Cliquer => {
                        if let Ok(p) = el.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) {
                            p.Invoke().map_err(err)?;
                        } else if let Ok(p) =
                            el.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
                        {
                            p.Select().map_err(err)?;
                        } else if let Ok(p) = el.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) {
                            p.Toggle().map_err(err)?;
                        } else {
                            return Err("cet élément ne se clique pas par accessibilité".into());
                        }
                        Ok("fait".into())
                    }
                    Action::Cocher => {
                        let p = el
                            .GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
                            .map_err(|_| "cet élément ne se coche pas".to_string())?;
                        p.Toggle().map_err(err)?;
                        let etat = p.CurrentToggleState().map(|s| s == ToggleState_On).unwrap_or(false);
                        Ok(if etat { "maintenant cochée".into() } else { "maintenant décochée".into() })
                    }
                    Action::Choisir => choisir(&self.u, el, texte),
                    Action::Deplier | Action::Replier => {
                        let p = el
                            .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
                            .map_err(|_| "cet élément ne se déplie pas".to_string())?;
                        if action == Action::Deplier { p.Expand() } else { p.Collapse() }.map_err(err)?;
                        Ok(if action == Action::Deplier { "déplié".into() } else { "replié".into() })
                    }
                    Action::Saisir => {
                        if info.mdp {
                            return Err("champ mot de passe : Waly n'y écrit jamais".into());
                        }
                        let t = texte.ok_or("texte à écrire manquant")?;
                        let mut voie = "";
                        if let Ok(v) = el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
                            if v.SetValue(&BSTR::from(t)).is_ok() {
                                voie = "valeur";
                            }
                        }
                        if voie.is_empty() {
                            // Repli clavier : exige le focus (la fenêtre passe
                            // au premier plan) — dit tel quel dans le constat.
                            el.SetFocus().map_err(err)?;
                            saisir_clavier(t)?;
                            voie = "clavier";
                        }
                        let relu = el
                            .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                            .ok()
                            .and_then(|v| v.CurrentValue().ok())
                            .map(|b| b.to_string());
                        let suffixe = if voie == "clavier" { " (au clavier : fenêtre mise au premier plan)" } else { "" };
                        Ok(match relu {
                            Some(v) => format!("écrit ; valeur relue : « {} »{suffixe}", court(&v, 120)),
                            None => format!("écrit{suffixe}"),
                        })
                    }
                }
            }
        }
    }

    /// Choisir : un élément de liste se sélectionne ; une LISTE DÉROULANTE
    /// reçoit l'option `texte` — par sa valeur si elle est éditable, sinon en
    /// la dépliant pour sélectionner l'option du même nom (vécu E2E
    /// 2026-09-11 : le 4B « choisissait » la liste elle-même, sans pattern).
    unsafe fn choisir(
        u: &IUIAutomation,
        el: &IUIAutomationElement,
        texte: Option<&str>,
    ) -> Result<String, String> {
        if let Ok(p) = el.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId) {
            p.Select().map_err(err)?;
            return Ok("choisi".into());
        }
        let t = texte.ok_or("indique l'option à choisir (texte)")?;
        if let Ok(v) = el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
            let lecture_seule = v.CurrentIsReadOnly().map(|b| b.as_bool()).unwrap_or(true);
            if !lecture_seule && v.SetValue(&BSTR::from(t)).is_ok() {
                let relu = v.CurrentValue().map(|b| b.to_string()).unwrap_or_default();
                return Ok(format!("choisi ; valeur relue : « {} »", court(&relu, 80)));
            }
        }
        let deplie = el
            .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
            .ok();
        if let Some(p) = &deplie {
            let _ = p.Expand();
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        let voulu = t.trim().to_lowercase();
        let tous = el
            .FindAll(TreeScope_Descendants, &u.CreateTrueCondition().map_err(err)?)
            .map_err(err)?;
        let mut fait = None;
        for i in 0..tous.Length().unwrap_or(0) {
            let Ok(o) = tous.GetElement(i) else { continue };
            let nom = o.CurrentName().map(|b| b.to_string()).unwrap_or_default();
            if nom.trim().to_lowercase() != voulu {
                continue;
            }
            if let Ok(s) = o.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId) {
                if s.Select().is_ok() {
                    fait = Some(nom);
                    break;
                }
            }
            if let Ok(s) = o.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) {
                if s.Invoke().is_ok() {
                    fait = Some(nom);
                    break;
                }
            }
        }
        if let Some(p) = &deplie {
            let _ = p.Collapse();
        }
        match fait {
            Some(n) => Ok(format!("choisi « {} »", court(&n, 60))),
            None => Err(format!("option « {} » introuvable dans la liste — aucune action faite", court(t, 60))),
        }
    }

    unsafe fn parcourir(
        el: &IUIAutomationElement,
        prof: usize,
        hors_ecran: bool,
        out: &mut Vec<(IUIAutomationElement, Element)>,
    ) {
        let off = el.CachedIsOffscreen().map(|b| b.as_bool()).unwrap_or(false);
        if off && prof > 0 && !hors_ecran {
            return; // menus repliés, listes défilées : élagués
        }
        let ct = el.CachedControlType().map(|c| c.0).unwrap_or(0);
        let (r, interactif, _) = role(ct);
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
        if garder(ct, &nom, valeur.is_some()) {
            out.push((
                el.clone(),
                Element {
                    role: if r.is_empty() { "?" } else { r },
                    nom,
                    valeur,
                    coche,
                    profondeur: prof,
                    interactif,
                    actif,
                    mdp,
                },
            ));
        }
        if let Ok(kids) = el.GetCachedChildren() {
            let n = kids.Length().unwrap_or(0);
            for i in 0..n {
                if let Ok(k) = kids.GetElement(i) {
                    parcourir(&k, prof + 1, hors_ecran, out);
                }
            }
        }
    }

    fn saisir_clavier(texte: &str) -> Result<(), String> {
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
            Err(format!("saisie clavier incomplète ({n}/{})", inputs.len()))
        }
    }

    pub fn titre(hwnd: isize) -> String {
        unsafe {
            let h = HWND(hwnd as *mut core::ffi::c_void);
            let n = GetWindowTextLengthW(h);
            let mut buf = vec![0u16; n.max(0) as usize + 1];
            let k = GetWindowTextW(h, &mut buf);
            String::from_utf16_lossy(&buf[..k.max(0) as usize]).trim().to_string()
        }
    }

    /// La fenêtre que l'utilisateur regarde : la plus haute dans l'ordre Z,
    /// visible, non réduite, non masquée (UWP suspendue), pas une fenêtre
    /// outil, titrée, et PAS une fenêtre de Waly (il ne se lit pas lui-même).
    pub fn fenetre_utilisateur() -> Option<(isize, String)> {
        unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
            let out = &mut *(l.0 as *mut Option<(isize, String)>);
            if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
                return BOOL(1);
            }
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
            if ex & WS_EX_TOOLWINDOW.0 != 0 {
                return BOOL(1);
            }
            let mut masque: u32 = 0;
            let _ = DwmGetWindowAttribute(
                h,
                DWMWA_CLOAKED,
                &mut masque as *mut u32 as *mut _,
                std::mem::size_of::<u32>() as u32,
            );
            if masque != 0 {
                return BOOL(1);
            }
            let t = titre(h.0 as isize);
            if t.is_empty() || t.starts_with("Waly") || t == "Program Manager" {
                return BOOL(1);
            }
            *out = Some((h.0 as isize, t));
            BOOL(0) // première trouvée = la plus haute : stop
        }
        let mut out: Option<(isize, String)> = None;
        unsafe {
            let _ = EnumWindows(Some(cb), LPARAM(&mut out as *mut _ as isize));
        }
        out
    }
}

#[cfg(windows)]
pub use imp::{fenetre_utilisateur, titre, Lecteur};

// Stubs non-Windows : `cargo check/test --workspace` reste vert sur l'hôte WSL.
#[cfg(not(windows))]
pub struct Lecteur;
#[cfg(not(windows))]
impl Lecteur {
    pub fn new() -> Result<Self, String> {
        Err("lecture d'écran par accessibilité : Windows seulement".into())
    }
    pub fn lire(&self, _hwnd: isize, _budget: usize) -> Result<Instantane, String> {
        Err("lecture d'écran par accessibilité : Windows seulement".into())
    }
    pub fn agir(&self, _c: &Cible, _a: Action, _t: Option<&str>) -> Result<String, String> {
        Err("action d'écran : Windows seulement".into())
    }
}
#[cfg(not(windows))]
pub fn fenetre_utilisateur() -> Option<(isize, String)> {
    None
}
#[cfg(not(windows))]
pub fn titre(_hwnd: isize) -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(role: &'static str, nom: &str, prof: usize, interactif: bool) -> Element {
        Element {
            role,
            nom: nom.into(),
            valeur: None,
            coche: None,
            profondeur: prof,
            interactif,
            actif: true,
            mdp: false,
        }
    }

    fn fenetre_formulaire() -> Vec<Element> {
        let mut mdp = el("champ", "Mot de passe", 1, true);
        mdp.mdp = true;
        mdp.valeur = Some("••••".into());
        let mut nom = el("champ", "Nom du fichier", 1, true);
        nom.valeur = Some("rapport.md".into());
        let mut case = el("case", "Ajouter la date", 1, true);
        case.coche = Some(true);
        vec![
            el("fenêtre", "Cible", 0, false),
            nom,
            mdp,
            case,
            el("bouton", "Enregistrer", 1, true),
            el("bouton", "Enregistrer", 1, true),
        ]
    }

    #[test]
    fn rendu_complet_montre_ids_valeurs_etats_et_masque_le_mot_de_passe() {
        let (t, mode) = rendre(&fenetre_formulaire(), BUDGET_DEFAUT);
        assert_eq!(mode, "complet");
        assert!(t.contains("[1] champ « Nom du fichier » = \"rapport.md\""), "{t}");
        assert!(t.contains("[2] champ « Mot de passe » = \"••••\" (mot de passe)"), "{t}");
        assert!(t.contains("[3] case « Ajouter la date » (cochée)"), "{t}");
        assert!(!t.contains("[0]"), "un conteneur n'a pas d'id : {t}");
    }

    #[test]
    fn compaction_sous_budget_resume_les_freres_repetes() {
        let mut els = vec![el("liste", "Résultats", 0, false)];
        for i in 0..200 {
            els.push(el("élément", &format!("Résultat numéro {i} avec un titre assez long"), 1, true));
        }
        let (t, mode) = rendre(&els, 1500);
        assert_ne!(mode, "complet");
        assert!(t.chars().count() <= 1500 + 30, "{} car.", t.chars().count());
        assert!(t.contains("autres (élément)"), "{t}");
        assert!(t.contains("[1] élément « Résultat numéro 0"), "les premiers restent adressables : {t}");
    }

    #[test]
    fn identite_stable_distingue_les_homonymes_par_rang() {
        let inst = Instantane {
            hwnd: 42,
            fenetre: "Cible".into(),
            elements: fenetre_formulaire(),
            texte: String::new(),
            mode: "complet",
        };
        let c = inst.cible(5).unwrap();
        assert_eq!((c.role.as_str(), c.nom.as_str(), c.rang), ("bouton", "Enregistrer", 1));
        assert_eq!(inst.cible(4).unwrap().rang, 0);
        // Un conteneur ou un id hors table n'est pas une cible.
        assert!(inst.cible(0).is_err());
        assert!(inst.cible(99).is_err());
        // L'identité survit à un aller-retour JSON (persistée dans l'approbation).
        let j = serde_json::to_value(&c).unwrap();
        assert_eq!(serde_json::from_value::<Cible>(j).unwrap(), c);
    }

    #[test]
    fn actions_et_elagage() {
        assert_eq!(Action::parse("Cliquer"), Some(Action::Cliquer));
        assert_eq!(Action::parse("écrire"), Some(Action::Saisir));
        assert_eq!(Action::parse("supprimer"), None);
        for n in Action::NOMS {
            assert!(Action::parse(n).is_some(), "{n}");
        }
        assert!(garder(50000, "OK", false)); // bouton nommé
        assert!(!garder(50000, "  ", false)); // bouton muet
        assert!(garder(50004, "", false)); // champ, même sans nom
        assert!(!garder(50026, "", false)); // groupe anonyme : traversé, pas rendu
        assert!(garder(50020, "Prêt", false)); // texte nommé
    }
}
