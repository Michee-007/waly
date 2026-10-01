//! Sûreté portée de l'ancien monde (waly-backend, inventaire 2026-07-05).
//!
//! Ordre STRICT avant tout dispatch — cet ordre est la spec :
//! 1. mur financier par nom d'outil (même halluciné) ;
//! 2. mur financier par intention du message utilisateur ;
//! 3. validation nom + arguments (calibrée : types/enums des champs présents,
//!    `required` NON exigé — voir tools.rs) ;
//! 4. gate de risque par catégorie (read / write / sensitive) ;
//! 5. anti-boucle (répétition, ping-pong).
//!
//! Frontière du mur financier : LIRE (solde, transactions, budget) est
//! autorisé ; TOUT mouvement d'argent est refusé, sans exception ni grant.

/// Message de refus des opérations financières (porté de FIN_REFUSAL).
pub const FIN_REFUSAL: &str = "Je ne peux pas exécuter d'opération financière \
    (virement, paiement, achat, investissement) — je peux seulement consulter \
    tes comptes et t'aider à suivre ton budget.";

/// Message de clarification quand l'intention ressemble à un mouvement
/// d'argent mais route vers un outil de lecture/suivi.
pub const MOVEMENT_CLARIFY: &str = "On dirait que tu me demandes de déplacer \
    de l'argent — ça, je ne le fais jamais. Si tu veux juste noter ou \
    consulter quelque chose, dis-le moi autrement.";

/// Tokens interdits dans un NOM d'outil (mur 1). Les noms d'outils sont
/// tokenisés sur les non-lettres car `_` casse les frontières de mots.
const FORBIDDEN_TOKENS: &[&str] = &[
    "transfer", "virement", "virer", "wire", "sendmoney", "withdraw", "retrait",
    "deposit", "depot", "dépôt", "pay", "payer", "payment", "buy", "acheter",
    "achat", "sell", "vendre", "vente", "trade", "invest", "investir", "crypto",
    "bitcoin", "ethereum", "epargne", "épargne", "epargner", "épargner",
    "placement", "placer", "savings", "save",
];

/// Sous-chaînes interdites (attrape `dotransferfunds` sans séparateur).
const FORBIDDEN_SUBSTRINGS: &[&str] = &["transfer", "crypto", "invest", "epargne", "épargne"];

/// Mur 1 : ce nom d'outil est-il une exécution financière ?
pub fn is_financial_execution(tool_name: &str) -> bool {
    let lower = tool_name.to_lowercase();
    let tokens: Vec<&str> = lower.split(|c: char| !c.is_alphabetic()).filter(|t| !t.is_empty()).collect();
    if tokens.iter().any(|t| FORBIDDEN_TOKENS.contains(t)) {
        return true;
    }
    FORBIDDEN_SUBSTRINGS.iter().any(|s| lower.contains(s))
}

/// Verbes de mouvement d'argent (tolérants aux déformations STT : « vir »).
const MOVEMENT_VERBS: &[&str] = &[
    "vire", "virer", "vir", "paie", "paye", "payer", "transfere", "transfère",
    "transferer", "transférer", "rembourse", "rembourser", "envoie", "envoi",
    "envoyer", "retire", "retirer", "depose", "dépose", "deposer", "déposer",
    "achete", "achète", "acheter", "investis", "investir", "place", "placer",
];

/// Marqueurs de contexte argent dans le message.
const MONEY_TOKENS: &[&str] = &[
    "€", "$", "euro", "euros", "argent", "fonds", "balle", "balles", "fric",
    "thune", "sous", "compte", "livret",
];

/// Mur 2 : le message utilisateur exprime-t-il un mouvement d'argent ?
/// (verbe de mouvement ET contexte argent — « 3k » compte comme montant).
pub fn is_money_movement_intent(message: &str) -> bool {
    let lower = message.to_lowercase();
    let words: Vec<&str> =
        lower.split(|c: char| !c.is_alphanumeric() && c != '€' && c != '$').filter(|w| !w.is_empty()).collect();
    let has_verb = words.iter().any(|w| MOVEMENT_VERBS.contains(w));
    if !has_verb {
        return false;
    }
    let has_money = MONEY_TOKENS.iter().any(|t| lower.contains(t))
        || words.iter().any(|w| is_amount_shorthand(w));
    has_money
}

/// « 50 » seul est ambigu, mais « 3k », « 50e », « 100eur » sont des montants.
fn is_amount_shorthand(word: &str) -> bool {
    let Some(last) = word.chars().last() else { return false };
    let head: String = word.chars().take(word.chars().count() - 1).collect();
    !head.is_empty()
        && head.chars().all(|c| c.is_ascii_digit())
        && matches!(last, 'k' | 'K' | 'e' | '€' | '$')
}

/// Outils dont le ROUTAGE sous intention de mouvement doit clarifier au lieu
/// d'exécuter (lecture bancaire + filets « dépense » que le modèle invente).
const FINANCIAL_ROUTE_TOOLS: &[&str] = &[
    "resume_bancaire", "transactions_recentes", "solde_compte", "alerte_budget",
    // filets : noms que le modèle hallucine pour « enregistrer une dépense »
    "log_expense", "add_expense", "update_expense", "track_expense",
    "record_expense", "set_expense", "depense", "ajouter_depense",
];

pub fn is_financial_route(tool_name: &str) -> bool {
    FINANCIAL_ROUTE_TOOLS.contains(&tool_name.to_lowercase().as_str())
}

// ── Mur financier des MAINS D'ÉCRAN (B, 2026-09-11) ────────────────────────
// Un clic sur « Payer » est un paiement, quel que soit l'outil qui clique. Liste
// DÉDIÉE aux libellés d'interface : celle des noms d'outils contient « save »
// (épargner) qui bloquerait le bouton anglais « Save ». Faux positifs assumés
// (« Moyens de paiement ») : l'utilisateur clique lui-même, jamais l'inverse.

const UI_FIN_TOKENS: &[&str] = &[
    "pay", "payer", "paye", "paie", "paiement", "paiements", "payment", "payments",
    "acheter", "achat", "achats", "buy", "purchase", "commander", "checkout",
    "panier", "virement", "virements", "virer", "transfer", "transferer",
    "transférer", "invest", "investir", "crypto", "bitcoin", "souscrire",
    "subscribe", "abonner", "abonnement", "donate", "vendre", "sell", "withdraw",
];
const UI_FIN_SUBSTRINGS: &[&str] = &["paiement", "payment", "transfer", "crypto", "invest", "checkout"];

/// Ce libellé d'élément d'interface (bouton, lien…) déclenche-t-il de l'argent ?
pub fn is_financial_ui_label(label: &str) -> bool {
    let lower = label.to_lowercase();
    let tokens = lower.split(|c: char| !c.is_alphabetic()).filter(|t| !t.is_empty());
    tokens.clone().any(|t| UI_FIN_TOKENS.contains(&t))
        || UI_FIN_SUBSTRINGS.iter().any(|s| lower.contains(s))
}

/// Le texte à écrire ressemble-t-il à un numéro de carte (13-19 chiffres,
/// espaces/tirets tolérés) ? Waly n'en saisit jamais (règle produit).
pub fn looks_like_card_number(text: &str) -> bool {
    let mut run = 0usize;
    for c in text.chars() {
        if c.is_ascii_digit() {
            run += 1;
            if run >= 13 {
                return true;
            }
        } else if c != ' ' && c != '-' {
            run = 0;
        }
    }
    false
}

#[cfg(test)]
mod tests_ui {
    use super::*;

    #[test]
    fn libelles_financiers_bloques() {
        for l in ["Payer", "Confirmer le paiement", "Acheter maintenant", "Buy now",
                  "Passer au checkout", "Commander", "Faire un virement", "Pay €12"] {
            assert!(is_financial_ui_label(l), "{l} devrait etre bloque");
        }
    }

    #[test]
    fn libelles_courants_passent() {
        for l in ["Save", "Enregistrer", "Envoyer", "OK", "Annuler", "Fichier",
                  "Palette de commandes", "Placer l'image", "Ajouter la date"] {
            assert!(!is_financial_ui_label(l), "{l} devrait passer");
        }
    }

    #[test]
    fn numeros_de_carte() {
        assert!(looks_like_card_number("4970 1012 3456 7890"));
        assert!(looks_like_card_number("4970-1012-3456-7890"));
        assert!(!looks_like_card_number("rapport 2026-09-11"));
        assert!(!looks_like_card_number("06 12 34 56 78")); // 10 chiffres : un téléphone
    }
}

// ── Gate de risque ──────────────────────────────────────────────────────────

/// Catégorie de risque d'un outil (dérivée de sa déclaration de capacités).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    /// Lecture pure : aucune confirmation.
    Read,
    /// Écriture réversible (mémoire, note, tâche) : pas de confirmation.
    Write,
    /// Irréversible ou effet externe : confirmation humaine AVANT exécution.
    Sensitive,
}

/// Capacités déclarées par outil — UNE seule table (fusion de l'ancien
/// tools.js `writesMemory` + capabilities.js `irreversible`/`isPayment`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Capabilities {
    pub writes_memory: bool,
    pub irreversible: bool,
    pub is_payment: bool,
}

impl Capabilities {
    /// Invariant fail-fast porté : paiement ⟹ irréversible.
    pub fn check_invariants(&self, name: &str) {
        if self.is_payment {
            assert!(self.irreversible, "outil {name}: isPayment exige irreversible");
        }
    }

    pub fn risk(&self) -> Risk {
        if self.irreversible {
            Risk::Sensitive
        } else if self.writes_memory {
            Risk::Write
        } else {
            Risk::Read
        }
    }
}

/// Capacités d'un outil INCONNU : irréversible par défaut (fail-safe porté).
pub fn unknown_capabilities() -> Capabilities {
    Capabilities { writes_memory: false, irreversible: true, is_payment: false }
}

// ── Anti-boucle ─────────────────────────────────────────────────────────────

/// Détecteur de boucle d'outils : même appel répété ou ping-pong A→B→A→B.
/// Réinitialisé à chaque tour utilisateur.
#[derive(Default)]
pub struct LoopGuard {
    history: Vec<String>, // empreintes nom+args dans l'ordre d'appel
}

impl LoopGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enregistre l'appel et dit s'il doit être BLOQUÉ.
    /// Bloqué si : même empreinte vue ≥ 2 fois déjà (3e exécution), ou
    /// ping-pong strict A,B,A,B sur les 4 derniers (ce call inclus).
    pub fn record(&mut self, tool_name: &str, args: &str) -> bool {
        let fp = format!("{tool_name}:{args}");
        let repeats = self.history.iter().filter(|h| **h == fp).count();
        self.history.push(fp.clone());
        if repeats >= 2 {
            return true;
        }
        let n = self.history.len();
        if n >= 4 {
            let w = &self.history[n - 4..];
            if w[0] == w[2] && w[1] == w[3] && w[0] != w[1] {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mur1_bloque_les_noms_financiers_meme_hallucines() {
        for name in
            ["transfer_money", "faire_virement", "pay_bill", "buy_crypto", "epargner_auto", "dotransferfunds"]
        {
            assert!(is_financial_execution(name), "{name} devrait etre bloque");
        }
    }

    #[test]
    fn mur1_laisse_la_lecture_bancaire() {
        for name in ["resume_bancaire", "solde_compte", "transactions_recentes", "alerte_budget", "heure"] {
            assert!(!is_financial_execution(name), "{name} devrait passer");
        }
    }

    #[test]
    fn mur2_detecte_le_mouvement_d_argent() {
        assert!(is_money_movement_intent("vire 50 € à Paul"));
        assert!(is_money_movement_intent("tu peux payer 3k a mon proprio"));
        assert!(is_money_movement_intent("rembourse 20 balles a Lea"));
        // Déformation STT vécue : « vir 50 euros ».
        assert!(is_money_movement_intent("vir 50 euros a paul"));
    }

    #[test]
    fn mur2_laisse_les_questions_et_le_suivi() {
        assert!(!is_money_movement_intent("combien j'ai depense ce mois ?"));
        assert!(!is_money_movement_intent("quel est mon solde ?"));
        // NB : « j'ai paye 30 euros hier » matche (verbe+argent) — assumé :
        // le mur 2 ne s'applique QUE si le modele route vers un outil
        // financier, jamais a la conversation libre.
    }

    #[test]
    fn gate_de_risque_et_invariants() {
        let read = Capabilities::default();
        assert_eq!(read.risk(), Risk::Read);
        let write = Capabilities { writes_memory: true, ..Default::default() };
        assert_eq!(write.risk(), Risk::Write);
        let sens = Capabilities { irreversible: true, ..Default::default() };
        assert_eq!(sens.risk(), Risk::Sensitive);
        assert_eq!(unknown_capabilities().risk(), Risk::Sensitive);
    }

    #[test]
    #[should_panic]
    fn paiement_sans_irreversible_est_un_bug() {
        Capabilities { is_payment: true, ..Default::default() }.check_invariants("t");
    }

    #[test]
    fn anti_boucle_repetition() {
        let mut g = LoopGuard::new();
        assert!(!g.record("meteo", r#"{"ville":"Paris"}"#));
        assert!(!g.record("meteo", r#"{"ville":"Paris"}"#)); // 2e : warn, passe
        assert!(g.record("meteo", r#"{"ville":"Paris"}"#)); // 3e : bloque
        assert!(!g.record("meteo", r#"{"ville":"Lyon"}"#)); // args differents : ok
    }

    #[test]
    fn anti_boucle_ping_pong() {
        let mut g = LoopGuard::new();
        assert!(!g.record("a", "{}"));
        assert!(!g.record("b", "{}"));
        assert!(!g.record("a", "{}"));
        assert!(g.record("b", "{}")); // A,B,A,B complet
    }
}
