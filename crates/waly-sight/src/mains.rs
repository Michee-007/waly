//! L'hôte des MAINS d'écran (B, 2026-09-11) : implémente
//! `waly_core::mains_ecran::HoteEcran` sur UI Automation (`crate::uia`).
//! Feature `mains` — le desktop l'active ; l'exemple `agent_ecran` joue le
//! MÊME code sans l'app (E2E du banc).
//!
//! Vit sur UN thread (COM `!Send`) : le worker desktop. La dernière lecture
//! est la référence des ids `[n]` que le modèle cite dans `agir_ecran`.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

use waly_core::mains_ecran::{bloc, CibleEcran, HoteEcran};

use crate::uia::{fenetre_utilisateur, Action, Cible, Instantane, Lecteur, BUDGET_DEFAUT};

pub struct HoteUia {
    lecteur: RefCell<Option<Lecteur>>,
    dernier: RefCell<Option<Instantane>>,
    /// Partage d'écran ouvert par l'utilisateur (mode ▣ Écran).
    actif: Arc<AtomicBool>,
    /// Fenêtre choisie dans le pop-up (0 = celle que l'utilisateur regarde).
    cible: Arc<AtomicI64>,
}

impl HoteUia {
    pub fn new(actif: Arc<AtomicBool>, cible: Arc<AtomicI64>) -> Self {
        Self { lecteur: RefCell::new(None), dernier: RefCell::new(None), actif, cible }
    }

    fn avec_lecteur<T>(&self, f: impl FnOnce(&Lecteur) -> Result<T, String>) -> Result<T, String> {
        let mut garde = self.lecteur.borrow_mut();
        if garde.is_none() {
            *garde = Some(Lecteur::new()?);
        }
        f(garde.as_ref().expect("lecteur posé ci-dessus"))
    }

    /// Lit la fenêtre choisie, sinon celle que l'utilisateur regarde (la plus
    /// haute hors Waly), et la garde comme référence des ids.
    pub fn instantane(&self) -> Result<Instantane, String> {
        let h = self.cible.load(Ordering::Relaxed);
        let hwnd = if h != 0 {
            h as isize
        } else {
            fenetre_utilisateur().map(|(h, _)| h).ok_or("aucune fenêtre à lire (tout est réduit ?)")?
        };
        let inst = self.avec_lecteur(|l| l.lire(hwnd, BUDGET_DEFAUT))?;
        *self.dernier.borrow_mut() = Some(inst.clone());
        Ok(inst)
    }
}

impl HoteEcran for HoteUia {
    fn actif(&self) -> bool {
        self.actif.load(Ordering::Relaxed)
    }

    fn lire(&self) -> Result<String, String> {
        let i = self.instantane()?;
        Ok(bloc(&i.fenetre, &i.texte))
    }

    fn resoudre(&self, id: usize) -> Result<CibleEcran, String> {
        let garde = self.dernier.borrow();
        let inst = garde.as_ref().ok_or("rien n'a encore été lu — appelle lire_ecran")?;
        let c = inst.cible(id)?;
        let mdp = inst.element(id).is_some_and(|e| e.mdp);
        Ok(CibleEcran {
            cible: serde_json::to_value(&c).map_err(|e| e.to_string())?,
            role: c.role,
            nom: c.nom,
            fenetre: c.fenetre,
            mdp,
        })
    }

    fn executer(&self, cible: &serde_json::Value, action: &str, texte: Option<&str>) -> Result<String, String> {
        let c: Cible = serde_json::from_value(cible.clone()).map_err(|e| format!("cible illisible: {e}"))?;
        let a = Action::parse(action).ok_or("action inconnue")?;
        let constat = self.avec_lecteur(|l| l.agir(&c, a, texte))?;
        // L'écran a changé : l'ancienne lecture n'est plus la référence des ids.
        *self.dernier.borrow_mut() = None;
        Ok(constat)
    }
}
