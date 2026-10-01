//! Les MAINS sur l'écran (B, 2026-09-11) — lire la fenêtre de l'utilisateur en
//! TEXTE (arbre d'accessibilité) et y agir, chaque action approuvée par lui.
//! Plan : `docs/PLAN-2026-09-11-B-agent-ecran.md`.
//!
//! waly-core reste découplé de waly-sight (même règle que la caméra et la
//! capture d'écran) : l'hôte — le desktop — implémente [`HoteEcran`].
//!
//! Sûreté (ordre du dispatch de tools.rs, rien de neuf à contourner) :
//! - `lire_ecran` = lecture pure, SEULEMENT pendant un partage d'écran ouvert
//!   par l'utilisateur (hors partage : absent du catalogue) ;
//! - `agir_ecran` = SENSIBLE : l'id éphémère `[n]` est traduit AVANT la mise
//!   en attente en identité stable + libellé humain (« cliquer sur bouton
//!   « Enregistrer » — fenêtre « … » »), c'est ce libellé que l'humain
//!   approuve ; mur financier sur le LIBELLÉ de l'élément (un clic sur
//!   « Payer » est un paiement) et sur le texte (numéro de carte) ; un champ
//!   mot de passe n'est jamais écrit.

use std::rc::Rc;

use crate::llm::{Msg, ToolSpec};
use crate::safety::{self, Capabilities, FIN_REFUSAL};
use crate::tools::{Dispatched, Registry, Tool};

/// Gestes possibles (miroir de `waly_sight::uia::Action`).
pub const ACTIONS: [&str; 6] = ["cliquer", "saisir", "cocher", "choisir", "deplier", "replier"];

/// Balises d'une lecture d'écran dans le contexte : une lecture n'a de valeur
/// qu'au tour où elle est faite — les anciennes sont retirées (comme les
/// images : `chat::degrader_images`) pour que l'historique ne gonfle pas de
/// ~1 300 tokens par tour et que Waly ne réponde jamais d'après un vieil écran.
pub const DEBUT: &str = "⟦écran⟧";
pub const FIN: &str = "⟦/écran⟧";

/// Élément résolu depuis la dernière lecture, prêt à être approuvé.
pub struct CibleEcran {
    /// Identité stable sérialisée (opaque pour waly-core, relue par l'hôte).
    pub cible: serde_json::Value,
    pub role: String,
    pub nom: String,
    pub fenetre: String,
    pub mdp: bool,
}

/// Ce que l'hôte fournit (desktop : `waly_sight::uia`).
pub trait HoteEcran {
    /// Un partage d'écran est-il ouvert par l'utilisateur ?
    fn actif(&self) -> bool;
    /// Lit la fenêtre cible MAINTENANT ; retourne le texte balisé ([`bloc`]).
    /// L'hôte garde cette lecture comme référence des ids `[n]`.
    fn lire(&self) -> Result<String, String>;
    /// Résout un id de la dernière lecture.
    fn resoudre(&self, id: usize) -> Result<CibleEcran, String>;
    /// Exécute une action APPROUVÉE sur une cible re-cherchée maintenant.
    fn executer(&self, cible: &serde_json::Value, action: &str, texte: Option<&str>)
        -> Result<String, String>;
}

/// Bloc de contexte d'une lecture d'écran (balisé pour être retiré plus tard).
pub fn bloc(fenetre: &str, texte: &str) -> String {
    format!(
        "{DEBUT}Fenêtre « {fenetre} » lue MAINTENANT (arbre d'accessibilité : texte \
         exact ; [n] = élément actionnable avec agir_ecran) :\n{texte}{FIN}"
    )
}

/// Consigne d'un tour avec lecture d'écran (desktop et E2E : UNE source).
/// Vécu E2E 2026-09-11 : sans « toi-même », après deux gestes approuvés le
/// 4B demandait à l'utilisateur de cliquer le troisième à sa place.
pub const CONSIGNE: &str = "(Réponds d'après CETTE lecture de l'écran, faite maintenant — \
jamais un souvenir. Pour agir : agir_ecran avec l'id [n], TOI-MÊME, un geste par \
action, jusqu'au bout de la tâche demandée ; l'utilisateur approuve chaque geste. Ne \
lui demande jamais de cliquer à ta place. Écris EXACTEMENT le texte demandé, sans le \
modifier. Bref.)";

/// Variante LECTURE SEULE (la voix n'a pas de mains) — mêmes balises.
pub fn bloc_lecture_seule(fenetre: &str, texte: &str) -> String {
    format!("{DEBUT}Fenêtre « {fenetre} » lue MAINTENANT (arbre d'accessibilité : texte exact) :\n{texte}{FIN}")
}

/// Retire les lectures d'écran anciennes de l'historique (messages
/// utilisateur ET résultats de `lire_ecran`). À appeler AVANT d'ajouter la
/// lecture du tour : une seule lecture vivante, la fraîche.
pub fn degrader_lectures(messages: &mut [Msg]) {
    for m in messages.iter_mut() {
        let t = match m {
            Msg::User(t) | Msg::ToolResult { content: t, .. } => t,
            Msg::UserImage { texte, .. } => texte,
            _ => continue,
        };
        if let Some(nouveau) = retirer_blocs(t) {
            *t = nouveau;
        }
    }
}

fn retirer_blocs(t: &str) -> Option<String> {
    if !t.contains(DEBUT) {
        return None;
    }
    let mut out = String::new();
    let mut reste = t;
    while let Some(i) = reste.find(DEBUT) {
        out.push_str(&reste[..i]);
        out.push_str("[lecture d'écran plus ancienne retirée]");
        reste = match reste[i..].find(FIN) {
            Some(j) => &reste[i + j + FIN.len()..],
            None => "",
        };
    }
    out.push_str(reste);
    Some(out)
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

/// Libellé HUMAIN d'une action — c'est ce texte que l'utilisateur approuve.
pub fn libelle(action: &str, c: &CibleEcran, texte: Option<&str>) -> String {
    let quoi = if c.nom.trim().is_empty() {
        c.role.clone()
    } else {
        format!("{} « {} »", c.role, court(&c.nom, 60))
    };
    let ou = format!("fenêtre « {} »", court(&c.fenetre, 50));
    match action {
        "saisir" => format!("écrire « {} » dans {quoi} — {ou}", court(texte.unwrap_or(""), 80)),
        "cliquer" => format!("cliquer sur {quoi} — {ou}"),
        "cocher" => format!("cocher/décocher {quoi} — {ou}"),
        "choisir" => match texte {
            Some(t) => format!("choisir « {} » dans {quoi} — {ou}", court(t, 60)),
            None => format!("choisir {quoi} — {ou}"),
        },
        "deplier" => format!("déplier {quoi} — {ou}"),
        "replier" => format!("replier {quoi} — {ou}"),
        autre => format!("{autre} {quoi} — {ou}"),
    }
}

const INACTIF: &str =
    "le partage d'écran n'est pas actif — l'utilisateur doit ouvrir le mode Écran";

/// Enregistre `lire_ecran` + `agir_ecran`. Présents au catalogue seulement
/// pendant un partage d'écran (`Tool::disponible`).
pub fn register_mains_ecran(registry: &mut Registry, hote: Rc<dyn HoteEcran>) {
    registry.register(Box::new(LireEcran { hote: hote.clone() }));
    registry.register(Box::new(AgirEcran { hote }));
}

struct LireEcran {
    hote: Rc<dyn HoteEcran>,
}

impl Tool for LireEcran {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "lire_ecran".into(),
            description: "Lit la fenetre que l'utilisateur regarde, en texte exact \
                          (boutons, champs, valeurs). Les [n] servent a agir_ecran."
                .into(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        }
    }
    fn disponible(&self) -> bool {
        self.hote.actif()
    }
    fn run(&self, _args: &serde_json::Value) -> Result<String, String> {
        if !self.hote.actif() {
            return Err(INACTIF.into());
        }
        self.hote.lire()
    }
}

struct AgirEcran {
    hote: Rc<dyn HoteEcran>,
}

impl Tool for AgirEcran {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "agir_ecran".into(),
            description: "Agit dans la fenetre lue : id=[n] de la lecture, action, \
                          texte = ce qu'il faut ecrire (saisir) ou l'option voulue \
                          (choisir). L'utilisateur approuve chaque action."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {"type": "integer"},
                    "action": {"type": "string", "enum": ACTIONS},
                    "texte": {"type": "string"},
                },
            }),
        }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { irreversible: true, ..Default::default() }
    }
    fn disponible(&self) -> bool {
        self.hote.actif()
    }
    fn preparer(&self, args: &serde_json::Value) -> Result<serde_json::Value, Dispatched> {
        if !self.hote.actif() {
            return Err(Dispatched::Rejected(INACTIF.into()));
        }
        let id = args["id"]
            .as_u64()
            .or_else(|| args["id"].as_str().and_then(|s| s.trim().parse().ok()))
            .ok_or_else(|| Dispatched::Rejected("id [n] requis (celui de la lecture lire_ecran)".into()))?;
        let action = args["action"]
            .as_str()
            .filter(|a| ACTIONS.contains(a))
            .ok_or_else(|| Dispatched::Rejected(format!("action requise : {}", ACTIONS.join(", "))))?;
        let texte = args["texte"].as_str().filter(|t| !t.is_empty());
        if action == "saisir" && texte.is_none() {
            return Err(Dispatched::Rejected("texte requis pour saisir".into()));
        }
        let c = self.hote.resoudre(id as usize).map_err(Dispatched::Rejected)?;
        // Une liste déroulante se choisit PAR son option (vécu E2E : sans
        // texte, deux approbations pour une action impossible).
        if action == "choisir" && c.role == "liste déroulante" && texte.is_none() {
            return Err(Dispatched::Rejected(
                "pour une liste déroulante, mets l'option voulue dans texte (ex. texte=\"PDF\")".into(),
            ));
        }
        // Murs : le libellé de l'élément, puis le texte à écrire.
        if safety::is_financial_ui_label(&c.nom) || texte.is_some_and(safety::looks_like_card_number) {
            return Err(Dispatched::Blocked(FIN_REFUSAL.into()));
        }
        if action == "saisir" && c.mdp {
            return Err(Dispatched::Blocked(
                "champ mot de passe : Waly n'y écrit jamais — l'utilisateur le remplit lui-même".into(),
            ));
        }
        Ok(serde_json::json!({
            "action": action,
            "texte": texte,
            "cible": c.cible,
            "libelle": libelle(action, &c, texte),
        }))
    }
    fn run(&self, args: &serde_json::Value) -> Result<String, String> {
        if !self.hote.actif() {
            return Err("le partage d'écran est fermé — rouvre le mode Écran pour que l'action se fasse".into());
        }
        let cible = args.get("cible").filter(|c| c.is_object()).ok_or("action non préparée")?;
        let action = args["action"].as_str().ok_or("action manquante")?;
        self.hote.executer(cible, action, args["texte"].as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ToolCall;
    use crate::safety::LoopGuard;
    use crate::tools::{TurnCtx, RESOLVE_TOOL};
    use std::cell::{Cell, RefCell};

    /// Hôte factice : une lecture fixe, compte les exécutions.
    struct Faux {
        actif: Cell<bool>,
        executions: RefCell<Vec<(serde_json::Value, String, Option<String>)>>,
    }

    impl HoteEcran for Faux {
        fn actif(&self) -> bool {
            self.actif.get()
        }
        fn lire(&self) -> Result<String, String> {
            Ok(bloc("Cible", "[1] bouton « Enregistrer »\n[2] bouton « Payer »\n[3] champ « Mot de passe »\n"))
        }
        fn resoudre(&self, id: usize) -> Result<CibleEcran, String> {
            let (role, nom, mdp) = match id {
                1 => ("bouton", "Enregistrer", false),
                2 => ("bouton", "Payer", false),
                3 => ("champ", "Mot de passe", true),
                4 => ("champ", "Nom du fichier", false),
                5 => ("liste déroulante", "Format", false),
                _ => return Err(format!("aucun élément actionnable [{id}]")),
            };
            Ok(CibleEcran {
                cible: serde_json::json!({"hwnd": 7, "role": role, "nom": nom, "rang": 0}),
                role: role.into(),
                nom: nom.into(),
                fenetre: "Cible".into(),
                mdp,
            })
        }
        fn executer(&self, cible: &serde_json::Value, action: &str, texte: Option<&str>) -> Result<String, String> {
            self.executions.borrow_mut().push((cible.clone(), action.into(), texte.map(Into::into)));
            Ok("fait".into())
        }
    }

    fn monter(actif: bool) -> (Registry, Rc<Faux>) {
        let faux = Rc::new(Faux { actif: Cell::new(actif), executions: RefCell::new(Vec::new()) });
        let mut r = Registry::new();
        register_mains_ecran(&mut r, faux.clone());
        (r, faux)
    }

    fn call(name: &str, args: &str) -> ToolCall {
        ToolCall { id: "c".into(), name: name.into(), arguments: args.into() }
    }

    #[test]
    fn hors_partage_les_mains_sont_absentes_du_catalogue() {
        let (r, faux) = monter(false);
        assert!(!r.specs().iter().any(|s| s.name.ends_with("_ecran")));
        faux.actif.set(true);
        let noms: Vec<String> = r.specs().into_iter().map(|s| s.name).collect();
        assert!(noms.contains(&"lire_ecran".into()) && noms.contains(&"agir_ecran".into()), "{noms:?}");
    }

    #[test]
    fn agir_passe_par_une_approbation_au_libelle_humain() {
        let conn = crate::store::open(":memory:").unwrap();
        let (r, faux) = monter(true);
        let mut g = LoopGuard::new();
        let mut c = TurnCtx { user_message: "clique sur enregistrer", loop_guard: &mut g, approvals: Some(&conn) };
        let d = r.dispatch(&call("agir_ecran", r#"{"id":1,"action":"cliquer"}"#), &mut c);
        let Dispatched::NeedsApproval(msg) = d else { panic!("attente attendue : {d:?}") };
        assert!(msg.contains("cliquer sur bouton « Enregistrer » — fenêtre « Cible »"), "{msg}");
        assert!(faux.executions.borrow().is_empty(), "rien avant le oui");
        // Ce qui est persisté : l'identité stable + le libellé, pas l'id éphémère seul.
        let p = &crate::store::list_pending(&conn).unwrap()[0];
        let args: serde_json::Value = serde_json::from_str(&p.tool_args).unwrap();
        assert_eq!(args["cible"]["nom"], "Enregistrer");
        assert!(args["libelle"].as_str().unwrap().starts_with("cliquer sur"));
        // Oui explicite → UNE exécution, sur la cible persistée.
        let d = r.dispatch(&call(RESOLVE_TOOL, &format!(r#"{{"confirmes":[{}]}}"#, p.id)), &mut c);
        assert!(matches!(d, Dispatched::Done(ref o) if o.contains("fait")), "{d:?}");
        let ex = faux.executions.borrow();
        assert_eq!(ex.len(), 1);
        assert_eq!(ex[0].0["nom"], "Enregistrer");
        assert_eq!(ex[0].1, "cliquer");
    }

    #[test]
    fn murs_payer_carte_et_mot_de_passe_bloquent_avant_toute_attente() {
        let conn = crate::store::open(":memory:").unwrap();
        let (r, faux) = monter(true);
        let mut g = LoopGuard::new();
        let mut c = TurnCtx { user_message: "vas-y", loop_guard: &mut g, approvals: Some(&conn) };
        for args in [
            r#"{"id":2,"action":"cliquer"}"#,                               // « Payer »
            r#"{"id":4,"action":"saisir","texte":"4970 1012 3456 7890"}"#, // carte
            r#"{"id":3,"action":"saisir","texte":"hunter2"}"#,             // mot de passe
        ] {
            let d = r.dispatch(&call("agir_ecran", args), &mut c);
            assert!(matches!(d, Dispatched::Blocked(_)), "{args} -> {d:?}");
        }
        assert!(crate::store::list_pending(&conn).unwrap().is_empty(), "aucune demande présentée");
        assert!(faux.executions.borrow().is_empty());
    }

    #[test]
    fn appels_invalides_sont_rejetes_pour_correction() {
        let conn = crate::store::open(":memory:").unwrap();
        let (r, _) = monter(true);
        let mut g = LoopGuard::new();
        let mut c = TurnCtx { user_message: "m", loop_guard: &mut g, approvals: Some(&conn) };
        for args in [
            r#"{"action":"cliquer"}"#,               // pas d'id
            r#"{"id":99,"action":"cliquer"}"#,       // id inconnu
            r#"{"id":4,"action":"saisir"}"#,         // saisir sans texte
            r#"{"id":1,"action":"supprimer"}"#,      // enum hallucinée
        ] {
            let d = r.dispatch(&call("agir_ecran", args), &mut c);
            assert!(matches!(d, Dispatched::Rejected(_)), "{args} -> {d:?}");
        }
        // Un id en chaîne numérique passe (coercion calibrée) ; « [1] » est
        // rejeté par la validation de type — le modèle se corrige au round suivant.
        let d = r.dispatch(&call("agir_ecran", r#"{"id":"[1]","action":"cliquer"}"#), &mut c);
        assert!(matches!(d, Dispatched::Rejected(_)), "{d:?}");
        let d = r.dispatch(&call("agir_ecran", r#"{"id":"1","action":"cliquer"}"#), &mut c);
        assert!(matches!(d, Dispatched::NeedsApproval(_)), "{d:?}");
    }

    #[test]
    fn choisir_dans_une_liste_deroulante_exige_l_option() {
        let conn = crate::store::open(":memory:").unwrap();
        let (r, _) = monter(true);
        let mut g = LoopGuard::new();
        let mut c = TurnCtx { user_message: "format pdf", loop_guard: &mut g, approvals: Some(&conn) };
        let d = r.dispatch(&call("agir_ecran", r#"{"id":5,"action":"choisir"}"#), &mut c);
        assert!(matches!(d, Dispatched::Rejected(ref m) if m.contains("texte")), "{d:?}");
        let d = r.dispatch(&call("agir_ecran", r#"{"id":5,"action":"choisir","texte":"PDF"}"#), &mut c);
        let Dispatched::NeedsApproval(msg) = d else { panic!("{d:?}") };
        assert!(msg.contains("choisir « PDF » dans liste déroulante « Format »"), "{msg}");
    }

    #[test]
    fn une_seule_lecture_vivante_dans_l_historique() {
        let mut m = vec![
            Msg::User(format!("[12:00] {} que vois-tu ?", bloc("A", "[1] bouton « OK »\n"))),
            Msg::ToolResult { call_id: "x".into(), content: bloc("B", "[2] champ\n") },
            Msg::User("sans lecture".into()),
        ];
        degrader_lectures(&mut m);
        let Msg::User(t) = &m[0] else { panic!() };
        assert_eq!(t, "[12:00] [lecture d'écran plus ancienne retirée] que vois-tu ?");
        let Msg::ToolResult { content, .. } = &m[1] else { panic!() };
        assert!(!content.contains("champ"), "{content}");
        let Msg::User(t) = &m[2] else { panic!() };
        assert_eq!(t, "sans lecture");
    }
}
