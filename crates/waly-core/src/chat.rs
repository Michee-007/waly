//! Boucle agentique : LLM → tool calls → socle de sûreté → dispatch →
//! réinjection → réponse finale. Bornée (pas de « projet ouvert » pour un
//! 4B — RFC D4). Portée de l'ancien monde : budget de retente de validation
//! = 1 (le 2e appel invalide arrête le tour), filet final sans outils.

use crate::llm::{LlmClient, Msg, Turn};
use crate::native_tools::CadrageEcran;
use crate::safety::LoopGuard;
use crate::tools::{Dispatched, Registry, TurnCtx};

/// Allers-retours outils max dans UN tour utilisateur (ancien monde : 10 ;
/// 4 pour un 4B local — un 4B qui boucle est un 4B perdu, et chaque
/// itération coûte ~1,3 s de préfill).
pub const MAX_TOOL_ROUNDS: usize = 4;

/// Joue un tour utilisateur complet. `messages` contient déjà le message
/// utilisateur en dernier ; les échanges outils y sont ajoutés au fil de
/// l'eau. `on_tool` est notifié de chaque dispatch (trace/UI).
pub fn run_turn(
    llm: &LlmClient,
    registry: &Registry,
    messages: &mut Vec<Msg>,
    approvals: Option<&rusqlite::Connection>,
    mut on_tool: impl FnMut(&str, &str),
) -> Result<String, String> {
    let user_message = messages
        .iter()
        .rev()
        .find_map(|m| match m {
            Msg::User(c) => Some(c.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let specs = registry.specs();
    let mut loop_guard = LoopGuard::new();
    let mut rejects = 0usize;

    for _ in 0..MAX_TOOL_ROUNDS {
        // Vision adaptative : images rendues visibles au cerveau qui ne voit
        // pas (décrites par le modèle vision délégué) — vision.rs.
        crate::vision::rendre_visible(messages);
        let mut issue = llm.chat(messages, &specs)?;
        // Appel d'outil ÉCRIT en texte par un petit modèle : rejoué comme un
        // vrai appel, par le même dispatch (rattrapage.rs).
        let rattrape = match &issue {
            Turn::Text(t) => crate::rattrapage::appel_ecrit_en_texte(t, |n| registry.connait(n)),
            Turn::ToolCalls(_) => None,
        };
        if let Some(call) = rattrape {
            issue = Turn::ToolCalls(vec![call]);
        }
        match issue {
            Turn::Text(text) => {
                messages.push(Msg::Assistant(text.clone()));
                return Ok(text);
            }
            Turn::ToolCalls(calls) => {
                messages.push(Msg::AssistantToolCalls(calls.clone()));
                let mut vus = std::collections::HashSet::new();
                for call in &calls {
                    if let Some(raison) = garde_appels(&mut vus, call) {
                        messages.push(Msg::ToolResult { call_id: call.id.clone(), content: raison.into() });
                        continue;
                    }
                    let mut ctx = TurnCtx {
                        user_message: &user_message,
                        loop_guard: &mut loop_guard,
                        approvals,
                    };
                    let (result, halt) = match registry.dispatch(call, &mut ctx) {
                        Dispatched::Done(out) => (out, false),
                        Dispatched::Blocked(msg) => (format!("refus: {msg}"), false),
                        Dispatched::NeedsApproval(msg) => (format!("en attente: {msg}"), false),
                        Dispatched::Rejected(msg) => {
                            rejects += 1;
                            (format!("ERREUR: {msg}"), rejects > 1)
                        }
                    };
                    on_tool(&call.name, &result);
                    messages.push(Msg::ToolResult {
                        call_id: call.id.clone(),
                        content: result,
                    });
                    if halt {
                        // 2e appel invalide : on arrête les frais (porté).
                        return finalize_without_tools(llm, messages);
                    }
                }
                injecter_apres_round(registry, messages);
            }
        }
    }
    // Le modèle n'a pas conclu dans le budget : forcer une réponse texte.
    finalize_without_tools(llm, messages)
}

/// Détection d'intention visuelle (raccourci du mode appel) : quand la
/// caméra est active et que le message parle de VOIR, l'appelant joint
/// l'image AU message — UN tour de modèle, ZÉRO outil (~5 s au lieu de
/// ~10-12, mesuré R4 ch. 4-5). Fiabilité avant tout : le 4B répond parfois
/// de MÉMOIRE (description d'un tour précédent) sans rappeler `regarder`
/// (vécu 2026-07-08 en appel réel) — l'image jointe le force à re-regarder.
pub fn intention_visuelle(message: &str) -> bool {
    let m = message.to_lowercase();
    [
        "regarde",
        "tu vois",
        "vois-tu",
        "je te montre",
        "qu'est-ce que je tiens",
        "devant la caméra",
        "devant la camera",
    ]
    .iter()
    .any(|k| m.contains(k))
}

/// Intention de regarder l'ÉCRAN (R5), distincte de la caméra. Retourne le
/// cadrage voulu, `None` si le message ne parle pas de l'écran. ⚠ L'hôte teste
/// ceci AVANT `intention_visuelle` : « regarde mon écran » contient « regarde »
/// (caméra) mais l'écran gagne. Accents ET sans-accent (robustesse STT).
pub fn intention_ecran(message: &str) -> Option<CadrageEcran> {
    let m = message.to_lowercase();
    // « tout l'écran » l'emporte sur le cadrage fenêtre.
    let plein = [
        "tout mon écran",
        "tout mon ecran",
        "tout l'écran",
        "tout l'ecran",
        "écran entier",
        "ecran entier",
        "tout l'affichage",
        "toutes mes fenêtres",
        "toutes mes fenetres",
    ];
    let ecran = [
        "mon écran",
        "mon ecran",
        "l'écran",
        "l'ecran",
        "à l'écran",
        "a l'ecran",
        "sur l'écran",
        "sur l'ecran",
        "cet écran",
        "cet ecran",
        "ce code",
        "cette erreur",
        "ce message d'erreur",
        "cette page",
        "cette fenêtre",
        "cette fenetre",
        "ce qui est affiché",
        "ce qui est affiche",
        "ce qui s'affiche",
        "ce bouton",
        "ce texte",
        "cette ligne",
        "cette image",
        "ce menu",
        "cette erreur",
        "ce que tu vois",
        "que vois-tu",
        "qu'est-ce que tu vois",
    ];
    if plein.iter().any(|k| m.contains(k)) {
        Some(CadrageEcran::Plein)
    } else if ecran.iter().any(|k| m.contains(k)) {
        Some(CadrageEcran::Fenetre)
    } else {
        None
    }
}

/// Routage OCR-first (chantier 1) : lire le texte exact (rapide, sans VLM) ou
/// comprendre le visuel (image jointe au VLM).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeEcran {
    /// Texte OCR seul au prompt — tour texte rapide (< 3 s), pas d'image.
    Lecture,
    /// Image jointe (+ OCR en aide) — le VLM voit — tour ~5-8 s.
    Comprehension,
}

/// Décide lecture vs compréhension : compréhension si l'OCR est vide/peu sûr,
/// ou si le message demande explicitement le VISUEL (mise en page, couleurs,
/// « à quoi ça ressemble »…). Sinon lecture (le cas dominant « lis-moi ça »).
pub fn mode_ecran(message: &str, conf: f32, ocr_vide: bool) -> ModeEcran {
    let m = message.to_lowercase();
    let visuel = [
        "décris",
        "decris",
        "ressemble",
        "à quoi",
        "a quoi",
        "c'est quoi",
        "couleur",
        "schéma",
        "schema",
        "graphique",
        "dessin",
        "mise en page",
        "interface",
        "capture d'écran",
        "montre-moi ce que tu vois",
    ];
    if ocr_vide || conf < 0.5 || visuel.iter().any(|k| m.contains(k)) {
        ModeEcran::Comprehension
    } else {
        ModeEcran::Lecture
    }
}

/// Construit le tour écran du RACCOURCI (image jointe = ZÉRO outil, discipline
/// R4.5). Renvoie `(contexte, image)` :
///   - `contexte` = bloc OCR (si texte) + consigne de brièveté, à mettre DANS
///     le contexte du tour — **jamais persisté** (règle vie privée : l'OCR brut
///     ne va pas en base ; l'hôte persiste le message BRUT, pas le contexte).
///   - `image` = data-URL à joindre en mode compréhension, sinon `None`.
/// Mutualisé desktop/voix (ch. 3).
pub fn fusion_ecran(message: &str, capture: crate::native_tools::CaptureEcran) -> (String, Option<String>) {
    // Brièveté imposée + ignore l'overlay Waly (le pop-up/cadre sont SON
    // interface, pas le contenu de l'utilisateur — vécu terrain 2026-07-10 : Waly ne
    // décrivait que son propre pop-up).
    const BREF: &str = "(Réponds brièvement, une ou deux phrases. Ignore le pop-up Waly \
et le cadre de l'écran — ils sont ton interface ; décris le travail de ton utilisateur.)\n";
    let ocr = capture.texte_ocr.trim();
    let vide = ocr.is_empty();
    match mode_ecran(message, capture.conf, vide) {
        ModeEcran::Lecture => {
            (format!("[Texte lu à l'écran (OCR) :\n{ocr}\n]\n{BREF}"), None)
        }
        ModeEcran::Comprehension => {
            let aide = if vide {
                String::new()
            } else {
                format!("[Texte lu à l'écran (OCR, en aide) :\n{ocr}\n]\n")
            };
            (format!("{aide}{BREF}"), Some(capture.image.data_url))
        }
    }
}

/// Une seule image vit dans l'historique : les précédentes deviennent un
/// marqueur texte. ⚠ C'est une MUTATION de l'historique — sous le cache FLM
/// append-only (GATE A R4.5) elle fait re-payer tout le préfill du tour où
/// elle s'applique : assumé, les tours vision sont rares et coûtent déjà
/// ~5 s ; la garder bornerait le ctx 8192 (300-800 tok/image). Public : le
/// raccourci d'intention du desktop (image jointe au message SANS round
/// d'outil) applique la même discipline avant de pousser la sienne.
pub fn degrader_images(messages: &mut [Msg]) {
    for m in messages.iter_mut() {
        if matches!(m, Msg::UserImage { .. }) {
            *m = Msg::User("[image plus ancienne retirée du contexte]".into());
        }
    }
}

/// Pousse les injections post-round (image de `regarder`).
fn injecter_apres_round(registry: &Registry, messages: &mut Vec<Msg>) {
    let injections = registry.take_injections();
    if injections.is_empty() {
        return;
    }
    degrader_images(messages);
    messages.extend(injections);
}

/// Dernier appel SANS outils pour forcer du texte (filet final porté).
fn finalize_without_tools(llm: &LlmClient, messages: &mut Vec<Msg>) -> Result<String, String> {
    match llm.chat(messages, &[])? {
        Turn::Text(text) => {
            messages.push(Msg::Assistant(text.clone()));
            Ok(text)
        }
        Turn::ToolCalls(_) => Err("le modele boucle sur les outils sans conclure".into()),
    }
}

/// Plafond d'appels DISTINCTS exécutés par réponse du modèle.
pub const APPELS_PAR_REPONSE: usize = 4;

/// Garde d'une réponse du modèle (vécu 2026-10-01 : le 4B émet parfois le
/// MÊME appel deux fois, ou une rafale — rappel posé deux fois, tâche créée
/// en double) : un appel identique (même nom, mêmes arguments) n'est exécuté
/// qu'une fois ; au-delà de [`APPELS_PAR_REPONSE`] appels distincts, le reste
/// est ignoré. `Some(raison)` = ne pas exécuter, répondre `raison`.
pub fn garde_appels(
    vus: &mut std::collections::HashSet<(String, String)>,
    call: &crate::llm::ToolCall,
) -> Option<&'static str> {
    // Arguments canoniques : l'ordre des clés ne doit pas masquer un doublon.
    let args = serde_json::from_str::<serde_json::Value>(&call.arguments)
        .map(|v| v.to_string())
        .unwrap_or_else(|_| call.arguments.trim().to_string());
    let cle = (call.name.clone(), args);
    if vus.contains(&cle) {
        return Some("doublon ignore : appel identique deja execute juste au-dessus");
    }
    if vus.len() >= APPELS_PAR_REPONSE {
        return Some("ignore : trop d'appels d'un coup — appuie-toi sur les resultats ci-dessus");
    }
    vus.insert(cle);
    None
}

/// Issue d'un tour STREAMING (voix).
pub struct StreamedTurn {
    /// Texte prononçable du tour (déjà livré delta par delta).
    pub text: String,
    /// Interrompu par barge-in : `text` est PARTIEL mais a été prononcé —
    /// il doit entrer dans l'historique tel quel (leçon waly-voice).
    pub interrupted: bool,
    /// EOF serveur avant `[DONE]` : réponse peut-être coupée, à signaler.
    pub truncated: bool,
    /// Au moins un round était un appel d'outil ÉCRIT en texte, rattrapé
    /// (rattrapage.rs) : ses deltas sont partis vers l'affichage, mais
    /// `text` ne contient PAS ce JSON — c'est `text` qu'il faut persister.
    pub rattrape: bool,
}

/// Variante streaming de [`run_turn`] pour la voix : les deltas de TEXTE
/// sortent vers `on_delta` (clauses → TTS) au fil de l'eau — y compris le
/// texte qu'un modèle émet AVANT un appel d'outil ; les rounds d'outils se
/// jouent entre deux. `on_delta("")` sonde le barge-in pendant les attentes ;
/// retourner `false` interrompt tout le tour.
pub fn run_turn_stream(
    llm: &LlmClient,
    registry: &Registry,
    messages: &mut Vec<Msg>,
    approvals: Option<&rusqlite::Connection>,
    on_delta: impl FnMut(&str) -> bool,
    on_tool: impl FnMut(&str, &str),
) -> Result<StreamedTurn, String> {
    run_turn_stream_with(llm, registry, messages, approvals, MAX_TOOL_ROUNDS, on_delta, on_tool)
}

/// Comme [`run_turn_stream`] mais avec un budget de rounds choisi par
/// l'appelant : le mode agent du desktop (R3 ch. 3) travaille plus longtemps
/// qu'un tour conversationnel (chaque round re-paie ~2 s de préfill — c'est
/// le prix assumé du travail de fond, pas celui d'une réplique).
pub fn run_turn_stream_with(
    llm: &LlmClient,
    registry: &Registry,
    messages: &mut Vec<Msg>,
    approvals: Option<&rusqlite::Connection>,
    max_rounds: usize,
    mut on_delta: impl FnMut(&str) -> bool,
    mut on_tool: impl FnMut(&str, &str),
) -> Result<StreamedTurn, String> {
    use crate::llm::StreamTurn;

    let user_message = messages
        .iter()
        .rev()
        .find_map(|m| match m {
            Msg::User(c) => Some(c.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let specs = registry.specs();
    let mut loop_guard = LoopGuard::new();
    let mut rejects = 0usize;
    let mut spoken = String::new();
    let mut a_rattrape = false;

    for round in 0..=max_rounds {
        // Vision adaptative (vision.rs) : une image injectée au round précédent
        // (outil regarder) ou jointe au message est rendue visible au cerveau
        // qui ne voit pas — point UNIQUE pour tous les chemins d'image.
        crate::vision::rendre_visible(messages);
        // Dernier round : sans outils, pour forcer une conclusion en texte.
        let tools = if round == max_rounds { &[] } else { specs.as_slice() };
        let mut issue = llm.chat_stream(messages, tools, &mut on_delta)?;
        // Rattrapage d'un appel ÉCRIT en texte (hors dernier round, qui doit
        // conclure en texte). Ses deltas sont déjà partis vers l'affichage :
        // l'appelant remplace la bulle par la réponse finale, qui ne contient
        // PAS le JSON (il n'entre ni dans `spoken` ni dans l'historique).
        let rattrape = match &issue {
            // Réflexion approfondie : l'appel écrit peut suivre une réflexion.
            StreamTurn::Text { text, .. } if round < max_rounds => {
                crate::rattrapage::appel_ecrit_en_texte(&crate::reflexion::retirer(text), |n| {
                    registry.connait(n)
                })
            }
            _ => None,
        };
        if let Some(call) = rattrape {
            a_rattrape = true;
            issue = StreamTurn::ToolCalls { text: String::new(), calls: vec![call] };
        }
        match issue {
            StreamTurn::Text { text, complete } => {
                spoken.push_str(&text);
                messages.push(Msg::Assistant(text));
                return Ok(StreamedTurn {
                    text: spoken,
                    interrupted: false,
                    truncated: !complete,
                    rattrape: a_rattrape,
                });
            }
            StreamTurn::Aborted(partial) => {
                spoken.push_str(&partial);
                if !spoken.is_empty() {
                    // Le partiel a été PRONONCÉ : l'historique doit le savoir.
                    messages.push(Msg::Assistant(spoken.clone()));
                }
                return Ok(StreamedTurn {
                    text: spoken,
                    interrupted: true,
                    truncated: false,
                    rattrape: a_rattrape,
                });
            }
            StreamTurn::ToolCalls { text, calls } => {
                if !text.is_empty() {
                    // Déjà prononcé : l'historique doit le porter aussi.
                    spoken.push_str(&text);
                    messages.push(Msg::Assistant(text));
                }
                messages.push(Msg::AssistantToolCalls(calls.clone()));
                let mut vus = std::collections::HashSet::new();
                for call in &calls {
                    if let Some(raison) = garde_appels(&mut vus, call) {
                        messages.push(Msg::ToolResult { call_id: call.id.clone(), content: raison.into() });
                        continue;
                    }
                    let mut ctx = TurnCtx {
                        user_message: &user_message,
                        loop_guard: &mut loop_guard,
                        approvals,
                    };
                    let (result, halt) = match registry.dispatch(call, &mut ctx) {
                        Dispatched::Done(out) => (out, false),
                        Dispatched::Blocked(msg) => (format!("refus: {msg}"), false),
                        Dispatched::NeedsApproval(msg) => (format!("en attente: {msg}"), false),
                        Dispatched::Rejected(msg) => {
                            rejects += 1;
                            (format!("ERREUR: {msg}"), rejects > 1)
                        }
                    };
                    on_tool(&call.name, &result);
                    messages.push(Msg::ToolResult { call_id: call.id.clone(), content: result });
                    if halt {
                        // Conclusion non-streamée (un seul delta) : le tour
                        // s'arrête là de toute façon.
                        let text = finalize_without_tools(llm, messages)?;
                        on_delta(&text);
                        spoken.push_str(&text);
                        return Ok(StreamedTurn {
                            text: spoken,
                            interrupted: false,
                            truncated: false,
                            rattrape: a_rattrape,
                        });
                    }
                }
                injecter_apres_round(registry, messages);
            }
        }
    }
    Err("le modele boucle sur les outils sans conclure".into())
}

#[cfg(test)]
mod tests_ecran {
    use super::*;
    use crate::native_tools::{CaptureEcran, ImageCapturee};

    #[test]
    fn intention_ecran_fenetre_plein_ou_rien() {
        assert_eq!(intention_ecran("regarde mon écran"), Some(CadrageEcran::Fenetre));
        assert_eq!(intention_ecran("lis cette erreur à l'écran"), Some(CadrageEcran::Fenetre));
        assert_eq!(intention_ecran("regarde tout mon écran"), Some(CadrageEcran::Plein));
        assert_eq!(intention_ecran("montre l'écran entier"), Some(CadrageEcran::Plein));
        // Caméra, pas écran :
        assert_eq!(intention_ecran("regarde ce que je tiens"), None);
        assert_eq!(intention_ecran("quelle heure est-il"), None);
    }

    #[test]
    fn mode_lecture_par_defaut_comprehension_si_visuel_ou_ocr_faible() {
        assert_eq!(mode_ecran("lis-moi cette erreur", 0.8, false), ModeEcran::Lecture);
        assert_eq!(mode_ecran("décris l'interface", 0.8, false), ModeEcran::Comprehension);
        assert_eq!(mode_ecran("lis ça", 0.2, false), ModeEcran::Comprehension); // conf faible
        assert_eq!(mode_ecran("lis ça", 0.9, true), ModeEcran::Comprehension); // OCR vide
    }

    fn capture(texte: &str, conf: f32) -> CaptureEcran {
        CaptureEcran {
            image: ImageCapturee { data_url: "data:image/png;base64,AA==".into(), largeur: 10, hauteur: 10 },
            texte_ocr: texte.into(),
            conf,
        }
    }

    #[test]
    fn fusion_lecture_contexte_ocr_sans_image() {
        let (contexte, img) = fusion_ecran("lis-moi l'erreur", capture("error E0432", 0.9));
        assert!(img.is_none());
        assert!(contexte.contains("error E0432"));
        assert!(contexte.contains("brièvement"));
    }

    #[test]
    fn fusion_comprehension_joint_image() {
        let (contexte, img) = fusion_ecran("décris cette page", capture("titre", 0.9));
        assert!(img.is_some()); // data-URL à joindre
        assert!(contexte.contains("titre")); // OCR en aide
    }
}
