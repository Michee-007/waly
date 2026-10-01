//! Registre d'outils natifs + socle de sûreté au dispatch.
//!
//! Ordre STRICT porté de l'ancien monde (voir safety.rs) : mur financier par
//! nom → mur financier par intention → validation nom+args → gate de risque →
//! anti-boucle → exécution. Toute erreur est renvoyée AU MODÈLE (résultat
//! d'outil) pour qu'il se corrige — budget de retente géré par la boucle.
//!
//! Validation CALIBRÉE (corpus réel de l'ancien monde) : types et enums
//! vérifiés sur les champs PRÉSENTS, mais `required` n'est PAS exigé (les
//! outils gèrent leurs défauts — l'exiger rejetait 27/36 appels vivants).

use crate::llm::{ToolCall, ToolSpec};
use crate::safety::{
    self, Capabilities, LoopGuard, Risk, FIN_REFUSAL, MOVEMENT_CLARIFY,
};

/// Un outil natif exécutable localement.
pub trait Tool {
    fn spec(&self) -> ToolSpec;
    fn capabilities(&self) -> Capabilities {
        Capabilities::default() // lecture pure par défaut
    }
    fn run(&self, args: &serde_json::Value) -> Result<String, String>;
    /// Message à INJECTER dans la conversation APRÈS les résultats d'outils
    /// du round (moment vision R4 : l'image suit le résultat de `regarder`).
    /// L'appel CONSOMME l'injection (intérieur mutable côté outil).
    fn take_injection(&self) -> Option<crate::llm::Msg> {
        None
    }
    /// L'outil figure-t-il au catalogue CE tour ? (mains d'écran : seulement
    /// pendant un partage d'écran — pas d'outil fantôme, pas de tokens payés
    /// pour rien.) Un appel à un outil indisponible reste traité : l'outil
    /// répond honnêtement.
    fn disponible(&self) -> bool {
        true
    }
    /// Outil SENSIBLE : transforme les arguments du modèle en arguments
    /// PERSISTÉS dans l'approbation (ex. un id d'écran éphémère → l'identité
    /// stable de l'élément + un libellé humain « libelle »). Refuser ici
    /// (murs, validation) évite qu'une demande absurde soit présentée à
    /// l'humain.
    fn preparer(&self, args: &serde_json::Value) -> Result<serde_json::Value, Dispatched> {
        Ok(args.clone())
    }
}

/// Issue d'un dispatch — la boucle agentique en tire le budget de retente.
#[derive(Debug, PartialEq)]
pub enum Dispatched {
    /// Résultat d'exécution (succès ou erreur runtime de l'outil).
    Done(String),
    /// Appel invalide (nom inconnu, args) : re-promptable, budget 1.
    Rejected(String),
    /// Refus de sûreté définitif (murs financiers, boucle) : pas de retente.
    Blocked(String),
    /// Outil sensible : exécution suspendue à une confirmation humaine.
    NeedsApproval(String),
}

/// Contexte d'un tour utilisateur pour le dispatch.
pub struct TurnCtx<'a> {
    /// Message utilisateur BRUT du tour (mur financier par intention).
    pub user_message: &'a str,
    pub loop_guard: &'a mut LoopGuard,
    /// Base des approbations : sans elle, les outils sensibles sont refusés
    /// tout court (fail-safe) au lieu d'être mis en attente.
    pub approvals: Option<&'a rusqlite::Connection>,
}

/// Nom de l'outil INTÉGRÉ de résolution des attentes (porté de
/// resolve_pending_tasks) — géré par le registre lui-même.
pub const RESOLVE_TOOL: &str = "resoudre_attentes";

#[derive(Default)]
pub struct Registry {
    tools: Vec<Box<dyn Tool>>,
    /// Sélection d'outils (selection.rs) : `None` = tous les outils ;
    /// `Some(groupes)` = seulement ces groupes (+ le socle). Cumulatif.
    selection: std::cell::RefCell<Option<std::collections::BTreeSet<String>>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enregistre un outil ; vérifie les invariants de capacités (fail-fast).
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        tool.capabilities().check_invariants(&tool.spec().name);
        self.tools.push(tool);
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        let sel = self.selection.borrow();
        let mut specs: Vec<ToolSpec> = self
            .tools
            .iter()
            .filter(|t| t.disponible())
            .map(|t| t.spec())
            .filter(|s| match sel.as_ref() {
                None => true,
                Some(g) => {
                    let gr = crate::selection::groupe(&s.name);
                    gr == crate::selection::SOCLE || g.contains(&gr)
                }
            })
            .collect();
        // L'outil de résolution n'existe que s'il y a des sensibles à gater
        // (et, en mode sélection, que si l'hôte a ouvert le groupe « attentes »).
        let resolution_visible = sel.as_ref().map_or(true, |g| g.contains("attentes"));
        if resolution_visible && self.tools.iter().any(|t| t.capabilities().risk() == Risk::Sensitive) {
            specs.push(ToolSpec {
                name: RESOLVE_TOOL.into(),
                description:
                    "SEUL outil qui traite une demande en attente (#id) : confirmes=[ids] \
                     si l'utilisateur a dit oui, rejetes=[ids] s'il refuse ou annule. \
                     Aucun autre outil ne peut annuler une demande."
                        .into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "confirmes": {"type": "array", "items": {"type": "integer"}},
                        "rejetes": {"type": "array", "items": {"type": "integer"}},
                    },
                }),
            });
        }
        specs
    }

    /// Passe en mode sélection (`Some(groupes)`, cumulatif) ou tous outils
    /// (`None`). Voir selection.rs.
    pub fn selectionner(&self, groupes: Option<std::collections::BTreeSet<String>>) {
        *self.selection.borrow_mut() = groupes;
    }

    /// Ajoute des groupes à la sélection en cours (sans effet en mode tous).
    /// Renvoie `true` si le bloc d'outils a changé.
    pub fn elargir(&self, groupes: impl IntoIterator<Item = String>) -> bool {
        let mut sel = self.selection.borrow_mut();
        let Some(g) = sel.as_mut() else { return false };
        let avant = g.len();
        g.extend(groupes);
        g.len() != avant
    }

    /// Groupes sélectionnés (`None` = tous les outils).
    pub fn groupes(&self) -> Option<std::collections::BTreeSet<String>> {
        self.selection.borrow().clone()
    }

    /// L'outil existe-t-il (catalogue OU résolution intégrée) ? Sert au
    /// rattrapage des appels écrits en texte (rattrapage.rs).
    pub fn connait(&self, name: &str) -> bool {
        name == RESOLVE_TOOL || self.tools.iter().any(|t| t.spec().name == name)
    }

    /// « Outil inconnu » avec les VOISINS du nom halluciné (vécu :
    /// `creer_fichier` pour `ecrire_fichier`) — le modèle se corrige au round
    /// suivant au lieu de conclure dans le vide. Voisin = partage un mot de
    /// 4 lettres ou plus ; classés par longueur de mots communs.
    fn inconnu(&self, name: &str) -> String {
        let mots: Vec<&str> = name.split('_').filter(|m| m.len() >= 4).collect();
        let mut voisins: Vec<(usize, String)> = self
            .tools
            .iter()
            .map(|t| t.spec().name)
            .map(|n| (mots.iter().filter(|m| n.contains(*m)).map(|m| m.len()).sum(), n))
            .filter(|(score, _)| *score > 0)
            .collect();
        // Tri stable : à score égal, l'ordre du catalogue.
        voisins.sort_by(|a, b| b.0.cmp(&a.0));
        let voisins: Vec<String> = voisins.into_iter().take(4).map(|(_, n)| n).collect();
        if voisins.is_empty() {
            format!("outil inconnu \u{ab} {name} \u{bb}")
        } else {
            format!(
                "outil inconnu \u{ab} {name} \u{bb} — outils proches : {}",
                voisins.join(", ")
            )
        }
    }

    /// Ramasse (et consomme) les messages à injecter après un round d'outils.
    pub fn take_injections(&self) -> Vec<crate::llm::Msg> {
        self.tools.iter().filter_map(|t| t.take_injection()).collect()
    }

    pub fn dispatch(&self, call: &ToolCall, ctx: &mut TurnCtx) -> Dispatched {
        // 1. Mur financier par NOM — avant même de chercher l'outil : un nom
        // halluciné du genre transfer_money est refusé, pas « inconnu ».
        if safety::is_financial_execution(&call.name) {
            return Dispatched::Blocked(FIN_REFUSAL.into());
        }
        // 2. Mur financier par INTENTION du message, sur routage financier.
        if safety::is_financial_route(&call.name)
            && safety::is_money_movement_intent(ctx.user_message)
        {
            return Dispatched::Blocked(MOVEMENT_CLARIFY.into());
        }
        // Outil intégré de résolution (après les murs : ils priment TOUJOURS).
        if call.name == RESOLVE_TOOL {
            return self.resolve_pending(call, ctx);
        }
        // 3. Validation : nom connu (pas de fuzzy-match), args objet JSON,
        // types/enums des champs présents.
        let Some(tool) = self.tools.iter().find(|t| t.spec().name == call.name) else {
            return Dispatched::Rejected(self.inconnu(&call.name));
        };
        // Un outil masqué par la sélection reste APPELABLE (le modèle le
        // connaît parfois) : son groupe entre dans la sélection pour la suite.
        self.elargir([crate::selection::groupe(&call.name)]);
        let args: serde_json::Value = match serde_json::from_str(&call.arguments) {
            Ok(v) => v,
            Err(e) => return Dispatched::Rejected(format!("arguments JSON invalides: {e}")),
        };
        if !args.is_object() {
            return Dispatched::Rejected("les arguments doivent etre un objet JSON".into());
        }
        let spec = tool.spec();
        if let Err(e) = validate_present_fields(&spec.parameters, &args) {
            return Dispatched::Rejected(e);
        }
        // 4. Gate de risque : sensible ⟹ confirmation humaine avant exécution.
        // La demande est PERSISTÉE (24 h) : elle survit au tour et se résout
        // par resoudre_attentes après le oui explicite. Sans base : refus
        // (fail-safe, jamais d'exécution directe).
        if tool.capabilities().risk() == Risk::Sensitive {
            let Some(conn) = ctx.approvals else {
                return Dispatched::Blocked(format!(
                    "l'action \u{ab} {} \u{bb} exige une confirmation humaine et les approbations sont indisponibles",
                    call.name
                ));
            };
            // L'outil prépare ce qui sera PERSISTÉ et montré à l'humain
            // (identité stable, libellé) — ou refuse avant toute attente.
            let prepared = match tool.preparer(&args) {
                Ok(v) => v,
                Err(d) => return d,
            };
            let libelle = prepared
                .get("libelle")
                .and_then(|v| v.as_str())
                .map(|l| format!(" ({l})"))
                .unwrap_or_default();
            return match crate::store::create_pending(conn, &call.name, &prepared.to_string()) {
                Ok(id) => Dispatched::NeedsApproval(format!(
                    "demande #{id} enregistree : \u{ab} {} \u{bb}{libelle} ne sera executee QUE si \
                     l'utilisateur dit explicitement oui — demande-lui, puis appelle \
                     {RESOLVE_TOOL} avec sa reponse",
                    call.name
                )),
                Err(e) => Dispatched::Blocked(format!("approbation impossible: {e}")),
            };
        }
        // 5. Anti-boucle : répétition ≥3 ou ping-pong.
        if ctx.loop_guard.record(&call.name, &call.arguments) {
            return Dispatched::Blocked(
                "appel repete en boucle — change d'approche ou reponds avec ce que tu as".into(),
            );
        }
        // 6. Exécution ; l'erreur runtime reste un résultat lisible.
        match tool.run(&args) {
            Ok(out) => Dispatched::Done(out),
            Err(e) => Dispatched::Done(format!("erreur: {e}")),
        }
    }
}

impl Registry {
    /// Résolution des attentes après réponse de l'utilisateur. Confirmé :
    /// claim ATOMIQUE (un double « oui » ne ré-exécute jamais) puis exécution
    /// DIRECTE de l'outil — le gate sensible est passé (c'est l'approbation),
    /// mais les murs financiers et la validation restent re-vérifiés.
    fn resolve_pending(&self, call: &ToolCall, ctx: &mut TurnCtx) -> Dispatched {
        let Some(conn) = ctx.approvals else {
            return Dispatched::Blocked("approbations indisponibles".into());
        };
        let args: serde_json::Value = match serde_json::from_str(&call.arguments) {
            Ok(v) => v,
            Err(e) => return Dispatched::Rejected(format!("arguments JSON invalides: {e}")),
        };
        let ids = |key: &str| -> Vec<i64> {
            args[key]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_i64()).collect())
                .unwrap_or_default()
        };
        let mut report: Vec<String> = Vec::new();
        for id in ids("rejetes") {
            match crate::store::reject_pending(conn, id) {
                Ok(true) => report.push(format!("#{id}: annulee")),
                Ok(false) => report.push(format!("#{id}: introuvable ou deja traitee")),
                Err(e) => report.push(format!("#{id}: erreur ({e})")),
            }
        }
        for id in ids("confirmes") {
            report.push(self.confirm_one(conn, id));
        }
        if report.is_empty() {
            return Dispatched::Rejected(
                "aucun id fourni — passer confirmes:[...] et/ou rejetes:[...]".into(),
            );
        }
        Dispatched::Done(report.join("\n"))
    }

    /// Confirme UNE attente : claim atomique → exécution approuvée → statut.
    /// Retourne la ligne de compte-rendu (commence par « #id: erreur » ou
    /// contient « erreur » si l'exécution a échoué).
    fn confirm_one(&self, conn: &rusqlite::Connection, id: i64) -> String {
        let pending = match crate::store::claim_pending(conn, id) {
            Ok(Some(p)) => p,
            Ok(None) => return format!("#{id}: introuvable, expiree ou deja traitee"),
            Err(e) => return format!("#{id}: erreur ({e})"),
        };
        let outcome = self.execute_approved(&pending);
        let success = !outcome.starts_with("erreur");
        if let Err(e) = crate::store::finish_pending(conn, id, success) {
            return format!("#{id}: executee mais statut non enregistre ({e})");
        }
        format!("#{id} ({}): {outcome}", pending.tool_name)
    }

    /// Résolution DIRECTE d'une attente depuis l'UI (boutons HITL, R3 ch. 3) :
    /// même chemin que l'outil `resoudre_attentes` (claim atomique, murs
    /// financiers et validation re-vérifiés par `execute_approved`) mais SANS
    /// passer par le LLM — un clic est déjà une réponse humaine explicite.
    pub fn resolve_one(&self, conn: &rusqlite::Connection, id: i64, approve: bool) -> String {
        if !approve {
            return match crate::store::reject_pending(conn, id) {
                Ok(true) => format!("#{id}: annulee"),
                Ok(false) => format!("#{id}: introuvable ou deja traitee"),
                Err(e) => format!("#{id}: erreur ({e})"),
            };
        }
        self.confirm_one(conn, id)
    }

    fn execute_approved(&self, pending: &crate::store::Pending) -> String {
        if safety::is_financial_execution(&pending.tool_name) {
            return format!("erreur: {FIN_REFUSAL}");
        }
        let Some(tool) = self.tools.iter().find(|t| t.spec().name == pending.tool_name) else {
            return format!("erreur: outil inconnu \u{ab} {} \u{bb}", pending.tool_name);
        };
        let args = match serde_json::from_str::<serde_json::Value>(&pending.tool_args) {
            Ok(v) if v.is_object() => v,
            Ok(_) => return "erreur: arguments non-objet".into(),
            Err(e) => return format!("erreur: arguments illisibles ({e})"),
        };
        if let Err(e) = validate_present_fields(&tool.spec().parameters, &args) {
            return format!("erreur: {e}");
        }
        match tool.run(&args) {
            Ok(out) => out,
            Err(e) => format!("erreur: {e}"),
        }
    }
}

/// Valide types + enums des champs PRÉSENTS contre un JSON-Schema minimal.
/// Tolérant comme l'Ajv coerceTypes de l'ancien monde : une chaîne numérique
/// passe pour un nombre, un nombre passe pour une chaîne ; `null` = absent.
fn validate_present_fields(schema: &serde_json::Value, args: &serde_json::Value) -> Result<(), String> {
    let Some(props) = schema["properties"].as_object() else { return Ok(()) };
    let Some(obj) = args.as_object() else { return Ok(()) };
    for (name, value) in obj {
        if value.is_null() {
            continue; // null = non fourni (porté tel quel)
        }
        let Some(prop) = props.get(name) else { continue }; // extras tolérés
        if let Some(allowed) = prop["enum"].as_array() {
            if !allowed.contains(value) {
                let vals: Vec<String> = allowed.iter().map(ToString::to_string).collect();
                return Err(format!(
                    "valeur invalide pour {name}: {value} (attendu: {})",
                    vals.join(", ")
                ));
            }
        }
        if let Some(ty) = prop["type"].as_str() {
            if !type_matches(ty, value) {
                return Err(format!("type invalide pour {name}: attendu {ty}"));
            }
        }
    }
    Ok(())
}

fn type_matches(ty: &str, value: &serde_json::Value) -> bool {
    match ty {
        "string" => value.is_string() || value.is_number(), // coercion
        "number" => {
            value.is_number()
                || value.as_str().is_some_and(|s| s.trim().parse::<f64>().is_ok())
        }
        "integer" => {
            value.is_i64()
                || value.is_u64()
                || value.as_f64().is_some_and(|f| f.fract() == 0.0)
                || value.as_str().is_some_and(|s| s.trim().parse::<i64>().is_ok())
        }
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;
    impl Tool for Echo {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: "echo".into(),
                description: "repete le texte".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "texte": {"type": "string"},
                        "priorite": {"type": "integer"},
                        "niveau": {"type": "string", "enum": ["bas", "haut"]},
                    },
                    "required": ["texte"],
                }),
            }
        }
        fn run(&self, args: &serde_json::Value) -> Result<String, String> {
            Ok(args["texte"].as_str().unwrap_or("(vide)").to_owned())
        }
    }

    /// Outil sensible : compte ses exécutions (prouve le « jamais sans oui »).
    struct Envoyer {
        executions: std::rc::Rc<std::cell::Cell<u32>>,
    }
    impl Tool for Envoyer {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: "envoyer_message".into(),
                description: "envoie un message".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"a": {"type": "string"}, "texte": {"type": "string"}},
                }),
            }
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities { irreversible: true, ..Default::default() }
        }
        fn run(&self, args: &serde_json::Value) -> Result<String, String> {
            self.executions.set(self.executions.get() + 1);
            Ok(format!("envoye a {}", args["a"].as_str().unwrap_or("?")))
        }
    }

    fn call(name: &str, arguments: &str) -> ToolCall {
        ToolCall { id: "c1".into(), name: name.into(), arguments: arguments.into() }
    }

    fn ctx<'a>(msg: &'a str, guard: &'a mut LoopGuard) -> TurnCtx<'a> {
        TurnCtx { user_message: msg, loop_guard: guard, approvals: None }
    }

    #[test]
    fn dispatch_execute_un_appel_valide() {
        let mut r = Registry::new();
        r.register(Box::new(Echo));
        let mut g = LoopGuard::new();
        assert_eq!(
            r.dispatch(&call("echo", r#"{"texte":"salut"}"#), &mut ctx("dis salut", &mut g)),
            Dispatched::Done("salut".into())
        );
    }

    #[test]
    fn required_absent_passe_mais_enum_et_type_valident() {
        let mut r = Registry::new();
        r.register(Box::new(Echo));
        let mut g = LoopGuard::new();
        // `texte` requis absent : PASSE (validation calibrée).
        assert_eq!(
            r.dispatch(&call("echo", "{}"), &mut ctx("m", &mut g)),
            Dispatched::Done("(vide)".into())
        );
        // enum halluciné : rejeté.
        assert!(matches!(
            r.dispatch(&call("echo", r#"{"niveau":"urgent"}"#), &mut ctx("m", &mut g)),
            Dispatched::Rejected(e) if e.contains("niveau")
        ));
        // type objet là où on attend un entier : rejeté ; chaîne numérique : passe.
        assert!(matches!(
            r.dispatch(&call("echo", r#"{"priorite":{"a":1}}"#), &mut ctx("m", &mut g)),
            Dispatched::Rejected(_)
        ));
        assert!(matches!(
            r.dispatch(&call("echo", r#"{"priorite":"3"}"#), &mut ctx("m", &mut g)),
            Dispatched::Done(_)
        ));
        // null = absent.
        assert!(matches!(
            r.dispatch(&call("echo", r#"{"niveau":null}"#), &mut ctx("m", &mut g)),
            Dispatched::Done(_)
        ));
    }

    #[test]
    fn mur_financier_par_nom_avant_outil_inconnu() {
        let r = Registry::new();
        let mut g = LoopGuard::new();
        match r.dispatch(&call("transfer_money", "{}"), &mut ctx("vire 50e", &mut g)) {
            Dispatched::Blocked(m) => assert!(m.contains("financière")),
            other => panic!("blocage attendu, recu {other:?}"),
        }
    }

    #[test]
    fn mur_intention_sur_routage_financier() {
        let r = Registry::new();
        let mut g = LoopGuard::new();
        // « vire 50 € » routé vers un filet dépense → clarification, pas d'exécution.
        match r.dispatch(&call("log_expense", r#"{"montant":50}"#), &mut ctx("vire 50 € a paul", &mut g)) {
            Dispatched::Blocked(m) => assert!(m.contains("déplacer")),
            other => panic!("blocage attendu, recu {other:?}"),
        }
    }

    #[test]
    fn sensible_sans_base_d_approbations_est_refuse() {
        // Fail-safe : pas de persistance possible => pas d'exécution du tout.
        let execs = std::rc::Rc::new(std::cell::Cell::new(0));
        let mut r = Registry::new();
        r.register(Box::new(Envoyer { executions: execs.clone() }));
        let mut g = LoopGuard::new();
        assert!(matches!(
            r.dispatch(&call("envoyer_message", "{}"), &mut ctx("envoie", &mut g)),
            Dispatched::Blocked(_)
        ));
        assert_eq!(execs.get(), 0);
    }

    #[test]
    fn flux_approbation_complet() {
        let conn = crate::store::open(":memory:").unwrap();
        let execs = std::rc::Rc::new(std::cell::Cell::new(0));
        let mut r = Registry::new();
        r.register(Box::new(Envoyer { executions: execs.clone() }));
        // L'outil de résolution apparaît dans les specs (il y a du sensible).
        assert!(r.specs().iter().any(|s| s.name == RESOLVE_TOOL));

        let mut g = LoopGuard::new();
        let mut c = TurnCtx { user_message: "envoie", loop_guard: &mut g, approvals: Some(&conn) };
        // 1. L'appel sensible est mis en attente, PAS exécuté.
        let d = r.dispatch(&call("envoyer_message", r#"{"a":"Paul","texte":"salut"}"#), &mut c);
        let Dispatched::NeedsApproval(msg) = d else { panic!("attente attendue: {d:?}") };
        assert!(msg.contains("#1"), "{msg}");
        assert_eq!(execs.get(), 0);
        assert_eq!(crate::store::list_pending(&conn).unwrap().len(), 1);

        // 2. Oui explicite : l'action s'exécute UNE fois.
        let d = r.dispatch(&call(RESOLVE_TOOL, r#"{"confirmes":[1]}"#), &mut c);
        let Dispatched::Done(out) = d else { panic!("done attendu: {d:?}") };
        assert!(out.contains("envoye a Paul"), "{out}");
        assert_eq!(execs.get(), 1);
        assert!(crate::store::list_pending(&conn).unwrap().is_empty());

        // 3. Double « oui » : ne ré-exécute JAMAIS (claim atomique).
        let d = r.dispatch(&call(RESOLVE_TOOL, r#"{"confirmes":[1]}"#), &mut c);
        let Dispatched::Done(out) = d else { panic!("done attendu: {d:?}") };
        assert!(out.contains("deja traitee"), "{out}");
        assert_eq!(execs.get(), 1);

        // 4. Refus : l'action rejetée ne s'exécute pas.
        r.dispatch(&call("envoyer_message", r#"{"a":"Lea"}"#), &mut c);
        let d = r.dispatch(&call(RESOLVE_TOOL, r#"{"rejetes":[2]}"#), &mut c);
        assert!(matches!(d, Dispatched::Done(out) if out.contains("annulee")));
        assert_eq!(execs.get(), 1);

        // 5. Expirée : non claimable.
        r.dispatch(&call("envoyer_message", r#"{"a":"Zoe"}"#), &mut c);
        conn.execute(
            "UPDATE pending_approvals SET expires_at=datetime('now','-1 hour') WHERE id=3",
            [],
        )
        .unwrap();
        let d = r.dispatch(&call(RESOLVE_TOOL, r#"{"confirmes":[3]}"#), &mut c);
        assert!(matches!(d, Dispatched::Done(out) if out.contains("expiree")));
        assert_eq!(execs.get(), 1);
    }

    #[test]
    fn anti_boucle_bloque_la_troisieme_repetition() {
        let mut r = Registry::new();
        r.register(Box::new(Echo));
        let mut g = LoopGuard::new();
        let mut c = ctx("m", &mut g);
        let a = call("echo", r#"{"texte":"x"}"#);
        assert!(matches!(r.dispatch(&a, &mut c), Dispatched::Done(_)));
        assert!(matches!(r.dispatch(&a, &mut c), Dispatched::Done(_)));
        assert!(matches!(r.dispatch(&a, &mut c), Dispatched::Blocked(_)));
    }

    #[test]
    fn outil_inconnu_et_args_invalides_sont_rejetes() {
        let r = Registry::new();
        let mut g = LoopGuard::new();
        assert!(matches!(
            r.dispatch(&call("meteo", "{}"), &mut ctx("m", &mut g)),
            Dispatched::Rejected(e) if e.contains("inconnu")
        ));
        let mut g = LoopGuard::new();
        assert!(matches!(
            r.dispatch(&call("meteo", "pas json"), &mut ctx("m", &mut g)),
            Dispatched::Rejected(_)
        ));
    }
}
