//! Waly desktop — coquille Tauri sur waly-core in-process.
//!
//! waly-core est MONO-THREAD (Rc<Connection>, !Send). On ne peut donc pas le
//! partager dans l'etat Tauri (Send+Sync exige). Solution : un THREAD WORKER
//! unique possede tout le stack (LlmClient + Registry + Connection), et les
//! commandes `invoke` lui parlent par canal mpsc (requete + canal de reponse).
//! Les tours sont de toute facon serialises (FLM ne sert qu'une requete).
//!
//! v4 (ch. 3) : espace AGENTIQUE — sessions-agents (kind='agent', budget de
//! rounds double, etat travail/attente/fini/echec) et HITL EN LIGNE : les
//! attentes (pending_approvals) sont servies a l'UI, un clic les resout par
//! `Registry::resolve_one` (claim atomique, murs re-verifies, SANS LLM), et
//! une session-agent REPREND son travail apres la decision.
//! Moteur requis : serveur LLM OpenAI-compat sur 127.0.0.1:42626
//! (`waly_core::llm::port_par_defaut()`, surcharge WALY_LLM_PORT).

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use tauri::ipc::Channel;
use tauri::{Emitter, Manager};
use waly_core::chat::{run_turn_stream_with, MAX_TOOL_ROUNDS};
use waly_core::llm::{BracketFilter, LlmClient, Msg};
use waly_core::native_tools::register_core_tools;
use waly_core::store;
use waly_core::tools::Registry;

/// Budget de rounds d'une session-agent : le double du conversationnel —
/// un agent qui travaille enchaine plus d'outils qu'une replique.
const AGENT_ROUNDS: usize = MAX_TOOL_ROUNDS * 2;

/// Evenements streames vers le front pendant un tour.
#[derive(Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StreamMsg {
    /// La session concernee (emis en tete d'un demarrage d'agent).
    Session { id: i64 },
    /// Un fragment de texte de la reponse, dans l'ordre.
    Delta { text: String },
    /// Un fragment de la REFLEXION (reflexion approfondie) : bloc repliable
    /// au-dessus de la reponse, jamais melange a elle.
    Reflexion { text: String },
    /// Qui repond a ce tour (seulement quand un modele exterieur est en jeu) :
    /// `cible` = "local" | "exterieur", le modele, la destination, la raison
    /// du routeur. L'UI l'affiche sous la reponse — une sortie se VOIT.
    Route { cible: String, modele: String, hote: String, raison: String },
    /// Un outil vient de tourner : nom + extrait du resultat (etape inline).
    Tool { name: String, info: String },
    /// Un fichier reel vient d'etre ecrit (Mains) : l'UI pose une carte
    /// cliquable (livrable), quel que soit l'outil porteur (ecrire_fichier
    /// direct ou via resoudre_attentes).
    Fichier { chemin: String },
    /// Fin de tour cote flux (l'invoke retourne le texte complet en parallele).
    Done { truncated: bool, interrupted: bool },
}

/// Messages envoyes au thread worker.
enum Cmd {
    Send {
        message: String,
        /// Image jointe par l'utilisateur (data URL, déjà réduite) — lot 1.
        image: Option<String>,
        /// Fichiers joints autorisés à partir vers un modèle extérieur pour
        /// CE message (accord explicite de l'utilisateur) — lot 3.
        fichiers: bool,
        chan: Channel<StreamMsg>,
        reply: Sender<Result<String, String>>,
    },
    /// Message reçu par une passerelle de messagerie (lot 3) depuis
    /// l'interlocuteur appairé : un tour du modèle LOCAL dans la conversation
    /// dédiée à la passerelle ; la réponse repart par elle.
    Passerelle {
        id: i64,
        texte: String,
        reply: Sender<Result<String, String>>,
    },
    /// Change le cerveau LOCAL en route (menu des modèles, lot 3) : clients
    /// recréés, sélection d'outils recalculée, fenêtre reconstruite.
    Modele {
        nom: String,
        reply: Sender<Result<(), String>>,
    },
    /// Historique de la session courante : (id de session, messages).
    History {
        reply: Sender<(i64, Vec<(String, String)>)>,
    },
    Sessions {
        kind: String,
        reply: Sender<Vec<store::SessionRow>>,
    },
    NewSession {
        reply: Sender<i64>,
    },
    SelectSession {
        id: i64,
        reply: Sender<Vec<(String, String)>>,
    },
    /// Supprime une conversation ou une mission (jamais le Fil principal) ;
    /// si c'etait la session courante, le worker se replace sur la derniere
    /// active. Repond l'id de la session courante apres suppression.
    DeleteSession {
        id: i64,
        reply: Sender<Result<i64, String>>,
    },
    Search {
        query: String,
        kind: String,
        reply: Sender<Vec<store::SessionRow>>,
    },
    Skills {
        reply: Sender<Vec<(String, String)>>,
    },
    /// Reconstruit la fenêtre (le système STABLE a changé : projet, réglage).
    Rebuild {
        reply: Sender<()>,
    },
    /// Pose un réglage (instructions, style) puis reconstruit la fenêtre :
    /// le système STABLE change -> un rebuild assumé (append-only).
    Reglage {
        cle: String,
        valeur: String,
        reply: Sender<Result<(), String>>,
    },
    Artifacts {
        reply: Sender<serde_json::Value>,
    },
    /// Demarre une session-agent sur un objectif (ch. 3).
    AgentStart {
        goal: String,
        /// Projet où ranger la mission AVANT son premier tour (contexte du projet).
        projet: Option<i64>,
        chan: Channel<StreamMsg>,
        reply: Sender<Result<String, String>>,
    },
    /// Attentes HITL en cours (cartes UI).
    Pendings {
        reply: Sender<Vec<(i64, String, String)>>,
    },
    /// Decision humaine sur une attente ; une session-agent courante REPREND
    /// son travail derriere (tour streame sur `chan`).
    Approve {
        id: i64,
        approve: bool,
        chan: Channel<StreamMsg>,
        reply: Sender<Result<String, String>>,
    },
    /// Mode appel (R4 ch. 5) : demarre la perception camera ; les evenements
    /// semantiques (arrivee/depart/attention/expression) partent vers l'UI
    /// sur `chan`. La reponse porte AUSSI la poignee de cliche : l'auto-vue
    /// de l'ecran d'appel la lit SANS passer par le worker (qui est seriel —
    /// la vignette gelerait pendant les tours LLM). Ces pixels ne servent
    /// que l'apercu local, rien n'est persiste.
    AppelStart {
        chan: Channel<waly_sight::perception::Event>,
        reply: Sender<Result<(String, waly_sight::perception::Cliche), String>>,
    },
    /// Arrete la perception (camera relachee) ; rend les stats de boucle.
    AppelStop {
        reply: Sender<String>,
    },
    /// Mode VOIX (R6b) : conversation vocale SANS camera (pattern « voice
    /// mode » — la page d'appel sert d'ecran, selfview cachee). Spawn
    /// waly-voice talk sur le Fil principal ; la veille est tuee (un micro).
    VoixStart {
        reply: Sender<Result<String, String>>,
    },
    /// Fin du mode voix : tue la voix (bouton OU eveil — les deux cas), la
    /// veille reprend l'ecoute legere.
    VoixStop {
        reply: Sender<String>,
    },
    /// Mode Ecran (R5 ch. 3) : ouvre une session de partage d'ecran (charge
    /// l'OCR, active la capture a la demande). Opt-in explicite = vie privee.
    EcranStart {
        reply: Sender<Result<String, String>>,
    },
    /// Ferme la session de partage d'ecran (libere l'OCR).
    EcranStop {
        reply: Sender<String>,
    },
    /// B3 « Regarde-moi » : ouvre une session de demonstration (hooks, etapes
    /// en TEXTE, jamais de pixels) — ouverte et fermee par l'utilisateur.
    RegardeStart {
        reply: Sender<Result<String, String>>,
    },
    /// Ferme la demonstration : rend les etapes, apprend la tache en fond.
    RegardeStop {
        reply: Sender<Result<Vec<String>, String>>,
    },
}

use waly_core::chat::intention_visuelle;

/// Etat Tauri : l'extremite d'envoi vers le worker (Send + Sync via Mutex)
/// + le drapeau d'interruption du tour en cours.
struct Core {
    tx: Mutex<Sender<Cmd>>,
    cancel: Arc<AtomicBool>,
    /// Poignee de cliche pendant un appel (None sinon) : l'apercu UI la lit
    /// en direct, hors du worker seriel.
    cliche: Mutex<Option<waly_sight::perception::Cliche>>,
    /// Pouls de la voix (R4.5 ch. 4) : etat de tour + niveau audio, POSTe
    /// par waly-voice sur /pouls, lu par l'eclipse de l'UI. Hors worker.
    pouls: Arc<Mutex<Pouls>>,
    /// Micro coupe (R5) : le pop-up le bascule, la voix le lit via /mic (Waly
    /// ne respecte pas le mute systeme — il lit le device brut).
    mic_muted: Arc<AtomicBool>,
    /// Cible de capture ecran (R5) : 0 = tout l'ecran, sinon le HANDLE d'une
    /// fenetre choisie dans le selecteur du pop-up. Lu par la capture + /cliche-ecran.
    cible_ecran: Arc<std::sync::atomic::AtomicI64>,
    /// Eveil (R6b) : instant du dernier « Waly » entendu par la veille
    /// (POST /eveil du processus voix). L'UI le sonde (core_eveil) pour
    /// jouer le « Souffle » — verdict design grave du 21/07.
    eveil: Arc<Mutex<Option<std::time::Instant>>>,
    /// Eveil ANNULE (R6b) : la confirmation STT a refute un eveil optimiste
    /// (score wake sature par un media). L'UI referme la page voix.
    eveil_annule: Arc<Mutex<Option<std::time::Instant>>>,
    /// Dictee en cours (UI 2026-09-14) : le micro n'est allume que pendant
    /// qu'elle existe.
    dictee: Mutex<Option<Dictee>>,
}

/// Une dictee locale : `waly-voice dicter` (micro + Parakeet), pilote par
/// stdin (« stop ») et lu sur stdout (ECOUTE / TEXTE / ERREUR).
struct Dictee {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    etat: Arc<Mutex<DicteeEtat>>,
}

#[derive(Default)]
struct DicteeEtat {
    ecoute: bool,
    texte: Option<String>,
    erreur: Option<String>,
    fini: bool,
}

/// Le pouls publie par le processus voix : ce que l'eclipse respire.
struct Pouls {
    /// "repos" | "ecoute" | "reflexion" | "parole".
    etat: String,
    /// Niveau audio lisse 0..1 (micro en ecoute, voix de Waly en parole).
    niveau: f32,
    /// Fraicheur : sans POST depuis > 1,5 s, l'UI retombe au repos (voix
    /// morte ou hors appel).
    maj: std::time::Instant,
}

impl Default for Pouls {
    fn default() -> Self {
        Self { etat: "repos".into(), niveau: 0.0, maj: std::time::Instant::now() }
    }
}

fn db_path() -> String {
    // WALY_DB › waly.toml [stockage] base › défaut — une seule vérité (store).
    store::chemin_par_defaut()
}

/// Prompt systeme STABLE de la fenetre (souvenirs + consigne d'attentes via
/// waly-core). ⚠ Discipline append-only (GATE A R4.5) : le cache FLM ne
/// survit que si ce message ne change PAS entre deux tours — ne le
/// (re)generer qu'aux rebuilds de fenetre, jamais par tour. Le dynamisme
/// (conscience d'appel, liste d'attentes) voyage dans l'en-tete frais du
/// dernier message utilisateur.
fn system_prompt(conn: &rusqlite::Connection, agent: bool, session: i64) -> Msg {
    let u = waly_core::user::designation();
    // Style de réponse (réglage) : rebuild provoqué par Cmd::Reglage.
    let longueur = waly_core::prompt::consigne_longueur(conn);
    let p = if agent {
        format!("Tu es Waly, l'agent personnel local de {u}, en mode TRAVAIL sur un \
         objectif. Avance par etapes avec tes outils (memoire, notes, taches, \
         rappels, fichiers reels) plutot que d'expliquer ce que tu pourrais faire. L'horodatage \
         [jour date heure] devant chaque message te donne la date et l'heure \
         ACTUELLES : tu les connais. Si une action exige une confirmation \
         humaine, elle est mise en attente : dis-le et continue ce qui peut \
         l'etre. Termine TOUJOURS par un bilan bref en francais : ce qui est \
         fait, ce qui attend une decision, ce qui reste.")
    } else {
        format!("Tu es Waly, l'assistant personnel local de {u}. Tu reponds en francais, \
         simplement et directement, {longueur}. L'horodatage [jour date \
         heure] devant chaque message de {u} te donne la date et l'heure ACTUELLES : \
         tu les connais, reponds directement. Quand cet en-tete dit « en appel \
         video », ta camera est active et tu es capable de voir. Quand il dit \
         « camera eteinte », tu ne vois RIEN : ne decris jamais une scene, une \
         personne ou un ecran, dis simplement que ta camera est eteinte. L'en-tete ne \
         te dit QUE la presence de {u}, son attention et son air — il ne \
         decrit PAS la scene : pour dire ce que tu vois, appelle D'ABORD \
         l'outil regarder et decris l'image recue ; ne devine jamais une scene \
         sans l'avoir appelee au tour courant, et ne commente ni l'outil ni \
         l'en-tete. Si l'en-tete contient « depuis ton dernier tour », c'est \
         ce que ta camera a VU entre-temps : reagis-y naturellement quand \
         c'est pertinent (une absence, un retour), sans reciter \
         l'en-tete ni les heures. Tu as des outils : \
         utilise-les quand ils servent la question, sinon reponds directement. \
         Quand {u} te confie une info durable sur lui, retiens-la avec \
         memoriser sans le lui demander.")
    };
    // Projet (lot 2) TOT dans le système : en queue d'un long prompt, le 4B
    // l'ignorait (vécu 2026-09-30 : ni l'instruction ni le fichier suivis).
    let p = p + &waly_core::prompt::contexte_projet(conn, session);
    // Résumé du début d'une longue conversation (lot 2) : ce que la fenêtre
    // ne montre plus.
    let p = p + &waly_core::resume::bloc_prompt(conn, session);
    let mut s = waly_core::prompt::inject_context(&p, conn);
    if !agent {
        // Journal visuel (R4.5 ch. 1) : ce que la camera a vu récemment
        // survit a la degradation des images — rafraichi au rebuild.
        s.push_str(&waly_core::prompt::memoire_visuelle(conn, session));
    }
    // Conscience du huis clos (R6a) : au REBUILD seulement (append-only), on
    // demande l'etat REEL du sceau et on ne l'affirme que s'il est tenu.
    s.push_str(waly_core::prompt::conscience_sceau(waly_core::sceau::actif()));
    Msg::System(s)
}

thread_local! {
    /// Posé par `rebuild_window` : la prochaine sélection d'outils repart du
    /// socle (nouvelle fenêtre = nouveau préfill de toute façon).
    static SELECTION_A_REMETTRE: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// État affiché dans Paramètres › Modèles : réglage, mode effectif, taille.
static OUTILS_ETAT: Mutex<Option<serde_json::Value>> = Mutex::new(None);

/// Calcule et applique le mode de sélection d'outils (démarrage, réglage).
fn appliquer_mode_outils(llm: &LlmClient, conn: &rusqlite::Connection, registry: &Registry) {
    let reglage = store::reglage(conn, "outils");
    let taille = llm.taille_milliards();
    let mode = waly_core::selection::mode(reglage.as_deref(), taille);
    registry.selectionner(match mode {
        waly_core::selection::ModeOutils::Selection => Some(Default::default()),
        waly_core::selection::ModeOutils::Tous => None,
    });
    SELECTION_A_REMETTRE.with(|c| c.set(true));
    if let Ok(mut e) = OUTILS_ETAT.lock() {
        *e = Some(serde_json::json!({
            "reglage": reglage.unwrap_or_else(|| "auto".into()),
            "mode": if mode == waly_core::selection::ModeOutils::Selection { "selection" } else { "tous" },
            "milliards": taille,
            "modele": llm.model,
        }));
    }
}

/// Rebatit la fenetre de contexte depuis le persiste : prompt systeme
/// REGENERE (souvenirs frais) + les 8 derniers messages de la session (les
/// tool-calls intermediaires sont elagues — meme discipline que la voix).
/// C'est le PALIER assume : un rebuild = un plein prefill au tour suivant
/// (cache FLM append-only) — on ne le fait qu'aux bornes (démarrage,
/// changement de session, fenetre > 25, 2e ecrivain pendant l'appel),
/// jamais par tour.
fn rebuild_window(messages: &mut Vec<Msg>, conn: &rusqlite::Connection, session: i64, agent: bool) {
    messages.clear();
    SELECTION_A_REMETTRE.with(|c| c.set(true));
    let systeme = system_prompt(conn, agent, session);
    // Diagnostic opt-in : ce que le modèle reçoit VRAIMENT (local, écrasé).
    if std::env::var("WALY_DEBUG_PROMPT").is_ok() {
        if let Msg::System(s) = &systeme {
            let _ = std::fs::write(std::env::temp_dir().join("waly-systeme.txt"), s);
        }
    }
    messages.push(systeme);
    for (role, content) in store::recent_messages_in(conn, session, 8).unwrap_or_default() {
        messages.push(match role.as_str() {
            "user" => Msg::User(content),
            _ => Msg::Assistant(content),
        });
    }
}

/// Titre auto d'une session : le premier message, tronque proprement.
fn title_from(message: &str) -> String {
    let mut t: String = message.chars().take(40).collect();
    if message.chars().count() > 40 {
        t.push('…');
    }
    t
}

/// Extrait court d'un resultat d'outil pour l'affichage inline des etapes.
/// Chemins des fichiers écrits d'après une issue d'outil (« cree : X (N
/// octets) » / « remplace : X (N octets) », y compris à l'intérieur d'un
/// compte-rendu resoudre_attentes) — la carte de livrable de l'UI.
fn fichiers_ecrits(out: &str) -> Vec<String> {
    let mut chemins = Vec::new();
    for ligne in out.lines() {
        for marqueur in ["cree : ", "remplace : "] {
            if let Some(i) = ligne.find(marqueur) {
                let reste = &ligne[i + marqueur.len()..];
                if let Some(j) = reste.rfind(" (") {
                    if reste.ends_with("octets)") {
                        chemins.push(reste[..j].trim().to_string());
                    }
                }
            }
        }
    }
    chemins
}

fn excerpt(s: &str) -> String {
    let one_line = s.replace('\n', " · ");
    let mut e: String = one_line.chars().take(90).collect();
    if one_line.chars().count() > 90 {
        e.push('…');
    }
    e
}

/// Joue UN tour streame complet dans la session courante : horodatage,
/// filtre anti-horodatage-singe, persistance de ce qui a ete AFFICHE,
/// titre auto (sessions chat neuves), etat agent (travail -> attente si le
/// tour a cree des demandes HITL, sinon fini ; echec sur erreur), fenetre
/// bornee. Retourne le texte affiche ("" = interrompu avant le premier mot).
#[allow(clippy::too_many_arguments)]
fn run_streamed_turn(
    llm: &LlmClient,
    registry: &Registry,
    conn: &rusqlite::Connection,
    messages: &mut Vec<Msg>,
    cancel: &AtomicBool,
    chan: &Channel<StreamMsg>,
    current: i64,
    user_message: &str,
    agent: bool,
    image_jointe: Option<String>,
    conscience_appel: Option<String>,
    delta_visuel: &str,
    // R5 : contexte OCR d'un tour écran — injecté DANS le tour, JAMAIS persisté
    // (seul `user_message` brut va en base ; règle vie privée : pas d'OCR sur disque).
    contexte_ecran: Option<String>,
    // Reflexion approfondie (lot 3) : consigne en queue du message, flux
    // separe (reflexion.rs). Jamais sur un tour vision ou ecran.
    reflexion: bool,
) -> Result<String, String> {
    let reflexion = reflexion && image_jointe.is_none();
    cancel.store(false, Ordering::SeqCst);
    // Paramètres › Utilisation : durée, premier mot, tokens du tour.
    let t0 = std::time::Instant::now();
    let premier = std::cell::Cell::new(None::<i64>);
    waly_core::llm::usage_reinit();
    // Append-only (GATE A R4.5) : le systeme ne bouge PAS ici — le frais
    // (conscience d'appel, attentes) part dans l'en-tete du message.
    let pendings_before = store::list_pending(conn).map(|p| p.len()).unwrap_or(0);
    if agent {
        store::set_agent_status(conn, current, "travail").ok();
    }
    let was_empty = store::recent_messages_in(conn, current, 1)
        .map(|v| v.is_empty())
        .unwrap_or(false);
    // Sélection d'outils : socle remis à chaque nouvelle fenêtre, puis les
    // groupes appelés par CE message s'ajoutent (cumulatif : le bloc
    // d'outils précède l'historique, le changer casse le cache de préfixe).
    if registry.groupes().is_some() {
        if SELECTION_A_REMETTRE.with(|c| c.replace(false)) {
            registry.selectionner(Some(Default::default()));
        }
        let connecteurs: Vec<String> = waly_core::mcp::actifs().into_iter().map(|s| s.nom).collect();
        let mut g = waly_core::selection::groupes_pour(user_message, &connecteurs);
        if contexte_ecran.is_some() {
            g.insert("ecran".into());
        }
        if store::list_pending(conn).map(|p| !p.is_empty()).unwrap_or(false) {
            g.insert("attentes".into());
        }
        registry.elargir(g);
    }
    let mut frais = waly_core::prompt::en_tete_frais(conn, conscience_appel.as_deref());
    // Projet : rappel COURT en queue (le 4B suit mal une instruction restée
    // au système — vécu 2026-09-30) ; le texte complet est au système.
    frais.push_str(&waly_core::prompt::rappel_projet(conn, current));
    // Competences apprises (2026-09-10) : une mission semblable a une mission
    // deja reussie recoit sa recette dans l'en-tete FRAIS (append-only R4.5).
    // Embedder absent en v1 (comme la memoire) : appariement mot-cle.
    // B3 (2026-09-11) : aussi pendant un partage d'ecran — une tache MONTREE
    // (« regarde-moi ») se rejoue avec les mains d'ecran, sous approbation.
    let mut competence_suivie: Option<String> = None;
    if agent || contexte_ecran.is_some() {
        if let Some(c) = waly_core::competences::pertinente(conn, &None, user_message) {
            frais.push_str(&waly_core::competences::fragment_frais(&c));
            competence_suivie = Some(c.key.clone());
        }
    }
    // Le contexte OCR (R5) se glisse APRÈS l'en-tête, AVANT le message brut :
    // le modèle le voit, mais seul `user_message` sera persisté (pas d'OCR en base).
    let corps = match &contexte_ecran {
        Some(c) => format!("{c}{user_message}"),
        None => user_message.to_string(),
    };
    let consigne = if reflexion { waly_core::reflexion::CONSIGNE } else { "" };
    let stamped = format!(
        "[{}{frais}{delta_visuel}] {corps}{consigne}",
        waly_core::clock::french_timestamp()
    );
    let base = messages.len();
    let vision = image_jointe.is_some();
    match image_jointe {
        // Raccourci d'intention : l'image accompagne le message (une seule
        // image vivante dans l'historique — meme discipline que l'outil).
        Some(data_url) => {
            waly_core::chat::degrader_images(messages);
            messages.push(Msg::UserImage { texte: stamped, data_url });
        }
        None => messages.push(Msg::User(stamped)),
    }
    // Tour vision (image jointe par le raccourci) : ZÉRO round d'outils —
    // le modèle doit décrire, pas outiller, et le bloc de 14 outils
    // (1 241 tok) re-préfillé coûterait ~3 s pour rien. Sans lui, le modèle
    // rappelait d'ailleurs `regarder` malgré l'image déjà jointe (tour à
    // 12 s au lieu de ~6, e2e 2026-07-08).
    let rounds = if vision {
        0
    } else if agent {
        AGENT_ROUNDS
    } else {
        MAX_TOOL_ROUNDS
    };
    // Le modele singe parfois l'horodatage `[...]` en tete de reponse :
    // filtre au fil de l'eau (lecon voix). `shown` = ce qui a REELLEMENT
    // ete affiche — c'est lui qu'on persiste, pas le brut du core.
    let mut filt = BracketFilter::new();
    let mut shown = String::new();
    // Reflexion approfondie : ce qui est entre balises part vers le bloc
    // repliable (`pensee`), le reste suit le chemin normal.
    let mut sep = waly_core::reflexion::Filtre::new();
    let mut pensee = String::new();
    // Journal visuel : un tour qui a VU (image jointe ou outil regarder)
    // laisse sa description en memoire (R4.5 ch. 1).
    let vu_outil = std::cell::Cell::new(false);
    // Outils REELLEMENT executes ce tour (ni refus, ni attente, ni erreur) —
    // le critere de distillation d'une competence en fin de mission.
    let outils_exec = std::cell::RefCell::new(Vec::<String>::new());
    let res = run_turn_stream_with(
        llm,
        registry,
        messages,
        Some(conn),
        rounds,
        |delta| {
            // `on_delta("")` = sonde d'interruption pendant les attentes.
            let out = if reflexion {
                let m = sep.feed(delta);
                if !m.reflexion.is_empty() {
                    pensee.push_str(&m.reflexion);
                    let _ = chan.send(StreamMsg::Reflexion { text: m.reflexion });
                }
                filt.feed(&m.reponse)
            } else {
                filt.feed(delta)
            };
            if !out.is_empty() {
                if premier.get().is_none() {
                    premier.set(Some(t0.elapsed().as_millis() as i64));
                }
                shown.push_str(&out);
                if chan.send(StreamMsg::Delta { text: out }).is_err() {
                    return false; // front parti (fenetre fermee) -> abandon
                }
            }
            !cancel.load(Ordering::Relaxed)
        },
        |name, out| {
            // Seule une VRAIE capture compte comme « vu » (sinon la réponse
            // inventée entrait au journal visuel — vécu 2026-09-14).
            if name == "regarder" && !out.starts_with("erreur") {
                vu_outil.set(true);
            }
            if !out.starts_with("refus:")
                && !out.starts_with("ERREUR:")
                && !out.starts_with("en attente:")
            {
                outils_exec.borrow_mut().push(name.to_string());
            }
            for chemin in fichiers_ecrits(out) {
                // La carte d'aperçu reste dans CETTE conversation quand on la rouvre.
                store::add_session_file(conn, current, &chemin).ok();
                let _ = chan.send(StreamMsg::Fichier { chemin });
            }
            let _ = chan.send(StreamMsg::Tool { name: name.into(), info: excerpt(out) });
        },
    );
    let fin = sep.finish();
    if !fin.reflexion.is_empty() {
        pensee.push_str(&fin.reflexion);
        let _ = chan.send(StreamMsg::Reflexion { text: fin.reflexion });
    }
    let mut rest = filt.feed(&fin.reponse);
    rest.push_str(&filt.finish());
    if !rest.is_empty() {
        shown.push_str(&rest);
        let _ = chan.send(StreamMsg::Delta { text: rest });
    }
    // A reflechi sans conclure (plafond atteint dans la reflexion, ou arret
    // apres la balise) : une relance courte, sans outils, pour la reponse.
    if reflexion
        && shown.trim().is_empty()
        && !pensee.trim().is_empty()
        && matches!(&res, Ok(t) if !t.interrupted)
        && !cancel.load(Ordering::Relaxed)
    {
        let mut suite = messages.clone();
        suite.push(Msg::User(waly_core::reflexion::RELANCE.into()));
        let mut court = LlmClient::new(&llm.host, llm.port, &llm.model);
        court.max_tokens = 600;
        match court.chat(&suite, &[]) {
            Ok(waly_core::llm::Turn::Text(t)) => {
                let t = BracketFilter::strip(&waly_core::reflexion::retirer(&t));
                if !t.is_empty() {
                    messages.push(Msg::Assistant(t.clone()));
                    let _ = chan.send(StreamMsg::Delta { text: t.clone() });
                    shown = t;
                }
            }
            Ok(_) => {}
            Err(e) => eprintln!("Waly: relance apres reflexion: {e}"),
        }
    }
    if reflexion {
        // La fenetre ne garde NI la consigne NI les reflexions : budget de
        // contexte (8k), et un modele qui les relirait les imiterait une fois
        // l'option coupee. Mutation en QUEUE (re-prefill court sur Ollama ;
        // plein tarif sur un cache une-case type FLM — assume, mode lent).
        if let Some(Msg::User(u)) = messages.get_mut(base) {
            *u = u.replace(waly_core::reflexion::CONSIGNE, "");
        }
        let mut i = base;
        while i < messages.len() {
            if let Msg::Assistant(t) = &messages[i] {
                let r = waly_core::reflexion::retirer(t);
                if r.is_empty() {
                    messages.remove(i);
                    continue;
                }
                messages[i] = Msg::Assistant(r);
            }
            i += 1;
        }
    }
    match res {
        Ok(turn) => {
            if shown.is_empty() {
                // Interrompu avant le premier mot : tour blanc.
                messages.truncate(base);
                if agent {
                    store::set_agent_status(conn, current, "fini").ok();
                }
                return Ok(String::new());
            }
            // Un partiel interrompu a ETE AFFICHE : il entre dans
            // l'historique tel quel (lecon voix). Exception : un appel
            // d'outil ecrit en texte a ete RATTRAPE — son JSON a defile a
            // l'ecran mais n'est pas la reponse ; on persiste alors le texte
            // du core (sans ce JSON), passe au meme filtre.
            let affiche = if turn.rattrape {
                let mut f = BracketFilter::new();
                let mut s = f.feed(&waly_core::reflexion::retirer(&turn.text));
                s.push_str(&f.finish());
                s
            } else {
                shown.clone()
            };
            store::append_message_in(conn, current, "user", user_message).ok();
            store::append_message_in(conn, current, "assistant", &affiche).ok();
            if !pensee.trim().is_empty() {
                store::add_session_reflexion(conn, current, &pensee).ok();
            }
            // Mesures seulement, jamais le contenu. Sans `usage` du serveur :
            // estimation ~4 caractères/token (marquée).
            let (entree, sortie, estime) = match waly_core::llm::usage_tour() {
                Some((p, c)) => (p as i64, c as i64, false),
                None => {
                    let chars: usize = messages.iter().map(|m| waly_core::llm::msg_to_json(m).to_string().len()).sum();
                    ((chars / 4) as i64, (affiche.chars().count() / 4) as i64, true)
                }
            };
            store::add_tour(
                conn,
                &store::Tour {
                    session_id: current,
                    modele: llm.model.clone(),
                    prompt_tokens: entree,
                    completion_tokens: sortie,
                    duree_ms: t0.elapsed().as_millis() as i64,
                    premier_ms: premier.get(),
                    estime,
                },
            )
            .ok();
            if vision || vu_outil.get() {
                store::visual_memory_add(
                    conn,
                    current,
                    "vu",
                    &waly_core::prompt::compacte(&affiche, 240),
                )
                .ok();
            }
            if was_empty && !agent {
                store::set_session_title(conn, current, &title_from(user_message)).ok();
            }
            if agent {
                let pendings_after = store::list_pending(conn).map(|p| p.len()).unwrap_or(0);
                let status = if pendings_after > pendings_before { "attente" } else { "fini" };
                store::set_agent_status(conn, current, status).ok();
                // Mission REUSSIE avec au moins un outil execute → distiller
                // une competence reutilisable, dans un thread A PART (sa
                // propre connexion — precedent « 2 ecrivains busy_timeout »
                // du mode appel) : le worker reste disponible.
                let outils = outils_exec.borrow().clone();
                if status == "fini" && !outils.is_empty() {
                    // Une compétence SUIVIE se relit (nouvelle version si l'usage
                    // l'améliore) ; sinon on en distille une neuve.
                    match &competence_suivie {
                        Some(key) => ameliorer_en_fond(llm, key.clone(), user_message.to_string(), outils, affiche.clone()),
                        None => distiller_en_fond(llm, user_message.to_string(), outils, affiche.clone()),
                    }
                }
            }
            // Session longue : borner la fenetre (le desktop vit des heures).
            // C'est un PALIER : le prochain tour re-paie son prefill, assume.
            if messages.len() > 25 {
                rebuild_window(messages, conn, current, agent);
            }
            let _ = chan.send(StreamMsg::Done {
                truncated: turn.truncated,
                interrupted: turn.interrupted,
            });
            // La bulle finale de l'UI est remplacee par CE texte : le JSON
            // rattrape qui a defile disparait de l'ecran aussi.
            Ok(affiche)
        }
        Err(e) => {
            // truncate, pas pop : des paires tool_call/tool_result ont pu
            // etre poussees avant l'echec.
            messages.truncate(base);
            if agent {
                store::set_agent_status(conn, current, "echec").ok();
            }
            Err(e)
        }
    }
}

/// Joue UN tour avec un modèle EXTÉRIEUR (lot 3, clé de l'utilisateur). Le
/// harness impose la règle : seul le texte de la conversation affichée sort —
/// ni système local (mémoire, projet, journal visuel, instructions), ni
/// outils, ni image ; les fichiers joints sont retirés sauf accord explicite
/// pour ce message. L'appel sort par la passerelle (Waly reste scellé) et
/// s'inscrit au journal du sceau.
#[allow(clippy::too_many_arguments)]
fn run_external_turn(
    conn: &rusqlite::Connection,
    cfg: &waly_core::exterieur::Config,
    messages: &mut Vec<Msg>,
    cancel: &AtomicBool,
    chan: &Channel<StreamMsg>,
    current: i64,
    user_message: &str,
    fichiers_autorises: bool,
) -> Result<String, String> {
    use waly_core::exterieur;
    cancel.store(false, Ordering::SeqCst);
    let t0 = std::time::Instant::now();
    let premier = std::cell::Cell::new(None::<i64>);
    let was_empty = store::recent_messages_in(conn, current, 1).map(|v| v.is_empty()).unwrap_or(false);
    let mut historique = store::recent_messages_in(conn, current, 40).unwrap_or_default();
    historique.push(("user".into(), user_message.to_string()));
    let sortants = exterieur::conversation_sortante(&historique, fichiers_autorises);
    let caracteres: usize = sortants.iter().map(|(_, t)| t.chars().count()).sum();
    let systeme = exterieur::systeme(waly_core::prompt::consigne_longueur(conn));
    let avec_fichier = fichiers_autorises && exterieur::contient_fichier(user_message);
    waly_core::sceau::noter(
        conn,
        "sortie",
        &format!(
            "{} via {} — {caracteres} caractères de la conversation envoyés{}",
            cfg.modele,
            cfg.hote(),
            if avec_fichier { ", fichier joint compris (autorisé pour ce message)" } else { "" }
        ),
    )
    .ok();
    let mut shown = String::new();
    let r = exterieur::converser(conn, cfg, &systeme, &sortants, |delta| {
        if !delta.is_empty() {
            if premier.get().is_none() {
                premier.set(Some(t0.elapsed().as_millis() as i64));
            }
            shown.push_str(delta);
            if chan.send(StreamMsg::Delta { text: delta.to_string() }).is_err() {
                return false;
            }
        }
        !cancel.load(Ordering::Relaxed)
    })?;
    if shown.trim().is_empty() {
        if r.interrompu {
            return Ok(String::new());
        }
        if !r.refus {
            return Err(format!("{} n'a rien répondu", cfg.modele));
        }
        shown = format!(
            "{} a décliné cette demande (garde-fous du fournisseur). Tu peux la poser au modèle local.",
            cfg.modele
        );
        let _ = chan.send(StreamMsg::Delta { text: shown.clone() });
    }
    let affiche = shown.trim().to_string();
    store::append_message_in(conn, current, "user", user_message).ok();
    store::append_message_in(conn, current, "assistant", &affiche).ok();
    let (entree, sortie, estime) = match (r.entree, r.sortie) {
        (Some(e), Some(s)) => (e as i64, s as i64, false),
        _ => ((caracteres / 4) as i64, (affiche.chars().count() / 4) as i64, true),
    };
    store::add_tour(
        conn,
        &store::Tour {
            session_id: current,
            modele: cfg.modele.clone(),
            prompt_tokens: entree,
            completion_tokens: sortie,
            duree_ms: t0.elapsed().as_millis() as i64,
            premier_ms: premier.get(),
            estime,
        },
    )
    .ok();
    if was_empty {
        store::set_session_title(conn, current, &title_from(user_message)).ok();
    }
    // La fenêtre du cerveau LOCAL suit le fil (ajout en queue, append-only).
    messages.push(Msg::User(format!("[{}] {user_message}", waly_core::clock::french_timestamp())));
    messages.push(Msg::Assistant(affiche.clone()));
    if messages.len() > 25 {
        rebuild_window(messages, conn, current, false);
    }
    let _ = chan.send(StreamMsg::Done { truncated: r.tronque, interrupted: r.interrompu });
    Ok(affiche)
}

/// Garde le cerveau CHARGÉ tant que l'app vit (2026-09-10, retour Michée :
/// « la réaction ne se fait pas à l'immédiat »). Mesuré : Ollama décharge le
/// modèle après 5 min d'inactivité → premier mot 4,7 s à froid (texte) et
/// 19,6 s (vision) contre 0,1-0,4 s à chaud ; et il IGNORE `keep_alive` sur
/// /v1. D'où un thread qui précharge au démarrage puis renouvelle toutes les
/// 4 min par l'API native (`garder_chaud`, sans calcul). FLM tient déjà son
/// modèle en permanence : sans objet sur le moteur A (no-op).
fn garder_chaud_en_fond(llm: &LlmClient) {
    let (host, port) = (llm.host.clone(), llm.port);
    std::thread::spawn(move || {
        loop {
            // Le cerveau peut changer en route (menu des modèles).
            let llm = LlmClient::new(&host, port, &waly_core::llm::modele_par_defaut());
            if let Err(e) = llm.garder_chaud() {
                eprintln!("Waly: garder chaud: {e}");
            }
            std::thread::sleep(std::time::Duration::from_secs(240));
        }
    });
}

/// Charge tout de suite le cerveau qui vient d'être choisi (le fil
/// périodique le renouvellera ensuite).
fn garder_chaud_en_fond_une_fois(llm: &LlmClient) {
    let llm = LlmClient::new(&llm.host, llm.port, &llm.model);
    std::thread::spawn(move || {
        if let Err(e) = llm.garder_chaud() {
            eprintln!("Waly: chargement du modele choisi: {e}");
        }
    });
}

/// Préchauffe le cache de préfixe [système + outils] en fond (thread à
/// part, le worker reste disponible) — voir `LlmClient::prechauffer_prefixe`.
fn prechauffer_prefixe_en_fond(llm: &LlmClient, messages: &[Msg], registry: &Registry) {
    let Some(systeme) = messages.first().cloned() else { return };
    let specs = registry.specs();
    let (host, port, model) = (llm.host.clone(), llm.port, llm.model.clone());
    std::thread::spawn(move || {
        let t0 = std::time::Instant::now();
        match LlmClient::new(&host, port, &model).prechauffer_prefixe(&systeme, &specs) {
            Ok(()) => eprintln!("Waly: prefixe prechauffe ({:.1} s)", t0.elapsed().as_secs_f64()),
            Err(e) => eprintln!("Waly: prechauffage du prefixe: {e}"),
        }
    });
}

/// Distille une compétence en arrière-plan après une mission réussie
/// (2026-09-10) : un tour LLM court sur SA connexion (WAL multi-écrivains,
/// busy_timeout 5 s) — le worker n'attend pas. Best-effort : tout échec se
/// journalise et s'oublie (une compétence manquée n'est pas une panne).
fn distiller_en_fond(llm: &LlmClient, demande: String, outils: Vec<String>, issue: String) {
    let (host, port, model) = (llm.host.clone(), llm.port, llm.model.clone());
    let path = db_path();
    std::thread::spawn(move || {
        let llm = LlmClient::new(&host, port, &model);
        let conn = match store::open(&path) {
            Ok(c) => c,
            Err(e) => return eprintln!("Waly: competences: base: {e}"),
        };
        match waly_core::competences::apprendre(&llm, &conn, &None, &demande, &outils, &issue) {
            Ok(Some(titre)) => eprintln!("Waly: competence apprise « {titre} »"),
            Ok(None) => {}
            Err(e) => eprintln!("Waly: competences: {e}"),
        }
    });
}

/// Relit une compétence suivie par une mission réussie (lot 2) — en fond, sur
/// sa propre connexion, comme `distiller_en_fond`.
fn ameliorer_en_fond(llm: &LlmClient, key: String, demande: String, outils: Vec<String>, issue: String) {
    let (host, port, model) = (llm.host.clone(), llm.port, llm.model.clone());
    let path = db_path();
    std::thread::spawn(move || {
        let llm = LlmClient::new(&host, port, &model);
        let conn = match store::open(&path) {
            Ok(c) => c,
            Err(e) => return eprintln!("Waly: competences: base: {e}"),
        };
        match waly_core::competences::ameliorer(&llm, &conn, &key, &demande, &outils, &issue) {
            Ok(Some(raison)) => eprintln!("Waly: competence « {key} » amelioree : {raison}"),
            Ok(None) => {}
            Err(e) => eprintln!("Waly: competences: {e}"),
        }
    });
}

/// B3 : apprend une DEMONSTRATION en fond (tour LLM court sur sa propre
/// connexion, comme `distiller_en_fond`) — le worker reste disponible.
fn apprendre_demo_en_fond(llm: &LlmClient, etapes: Vec<String>) {
    let (host, port, model) = (llm.host.clone(), llm.port, llm.model.clone());
    let path = db_path();
    std::thread::spawn(move || {
        let llm = LlmClient::new(&host, port, &model);
        let conn = match store::open(&path) {
            Ok(c) => c,
            Err(e) => return eprintln!("Waly: demonstration: base: {e}"),
        };
        match waly_core::competences::apprendre_demonstration(&llm, &conn, &None, &etapes) {
            Ok(Some(titre)) => eprintln!("Waly: tache apprise en regardant « {titre} »"),
            Ok(None) => eprintln!("Waly: demonstration : rien de neuf a apprendre"),
            Err(e) => eprintln!("Waly: demonstration: {e}"),
        }
    });
}

/// Capture l'écran (cadrage) + OCR → `CaptureEcran` pour waly-core (R5 ch. 3).
/// Aucun pixel ni texte OCR persisté ici : tout vit en mémoire le temps du tour.
fn capture_ecran_ocr(
    ecran: &Rc<std::cell::RefCell<Option<waly_sight::ocr::Ocr>>>,
    cible_ecran: &Arc<std::sync::atomic::AtomicI64>,
) -> Result<waly_core::native_tools::CaptureEcran, String> {
    let mut garde = ecran.borrow_mut();
    let ocr = garde
        .as_mut()
        .ok_or("le partage d'écran n'est pas actif — l'utilisateur doit ouvrir le mode Écran")?;
    // La cible choisie dans le sélecteur du pop-up décide : 0 = tout l'écran,
    // sinon la fenêtre (handle). Elle prime sur l'intention (Michée choisit).
    let h = cible_ecran.load(Ordering::Relaxed);
    let shot = if h == 0 {
        waly_sight::screen::capture_screen()?
    } else {
        waly_sight::screen::capture_window(h as isize)?
    };
    // Image ≤ 720p pour le VLM (GATE 3) ; OCR sur le cliché pleine résolution.
    let (jpeg, w, h) = shot.jpeg(1600)?;
    let res = ocr.read(&shot)?;
    Ok(waly_core::native_tools::CaptureEcran {
        image: waly_core::native_tools::ImageCapturee {
            data_url: format!("data:image/jpeg;base64,{}", waly_core::native_tools::base64(&jpeg)),
            largeur: w,
            hauteur: h,
        },
        texte_ocr: res.text,
        conf: res.mean_conf,
    })
}

/// Spawne waly-voice.exe (compagnon d'appel). Console masquee sous Windows ;
/// l'exe herite de l'environnement (WALY_MODEL, WALY_DB...) + recoit le port
/// du service d'appel (camera + etat de presence via le desktop).
fn demarrer_voix(
    appel_port: u16,
    ecran: bool,
    camera: bool,
    session: i64,
) -> Result<std::process::Child, String> {
    let exe = std::env::var("WALY_VOICE_EXE")
        .unwrap_or_else(|_| waly_core::chemins::exe("waly-voice"));
    if !std::path::Path::new(&exe).exists() {
        return Err(format!("{exe} introuvable"));
    }
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("talk")
        .env("WALY_APPEL_PORT", appel_port.to_string())
        // Le cerveau local choisi dans l'app vaut aussi pour la voix.
        .env("WALY_MODEL", waly_core::llm::modele_par_defaut())
        .env("WALY_SESSION", session.to_string()); // conversation ou ranger les messages
    if ecran {
        cmd.env("WALY_ECRAN_MODE", "1"); // la voix voit l'ecran (R5), pas la camera
    }
    // Honnetete (2026-09-14) : la voix ne se croit en appel video QUE si la
    // camera tourne vraiment (le port d'appel est passe a TOUTES les voix).
    if camera {
        cmd.env("WALY_CAMERA", "1");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().map_err(|e| format!("spawn {exe}: {e}"))
}

/// Spawne la VEILLE (R6b) : waly-voice en ecoute legere du wake word —
/// ~50 Mo, ~1,4 % d'un coeur (banc lab/eveil-banc). Dire « Waly » reveille
/// la presence sans clic. WALY_VEILLE=0 desactive.
fn demarrer_veille(appel_port: u16) -> Result<std::process::Child, String> {
    let exe = std::env::var("WALY_VOICE_EXE")
        .unwrap_or_else(|_| waly_core::chemins::exe("waly-voice"));
    if !std::path::Path::new(&exe).exists() {
        return Err(format!("{exe} introuvable"));
    }
    let mut cmd = std::process::Command::new(&exe);
    // Pas de WALY_SESSION : reveillee, la veille demande au desktop la
    // conversation ouverte (GET /session) et y ecrit.
    cmd.arg("veille").env("WALY_APPEL_PORT", appel_port.to_string());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().map_err(|e| format!("spawn veille {exe}: {e}"))
}

/// La veille est-elle desactivee par l'environnement ?
/// Veille « Waly » RETIREE par defaut (decision Michee 2026-09-14) : le micro
/// ne s'allume QUE quand on active la voix ou la dictee. Opt-in developpeur :
/// WALY_VEILLE=1.
fn veille_desactivee() -> bool {
    if std::env::var("WALY_VEILLE").as_deref() != Ok("1") {
        return true;
    }
    // La Garde : micro coupe = pas d'ecoute du mot d'eveil non plus. Le chien
    // de garde de la veille relit cette condition : la coupure la laisse
    // eteinte, le retablissement la relance.
    base().map(|c| !waly_core::garde::permis(&c, Ressource::Micro)).unwrap_or(false)
}

/// Etat de presence du mode appel, partage entre le forwarder d'evenements,
/// le service loopback (processus voix) et la conscience des tours tapes.
#[derive(Default, Clone)]
struct EtatAppel {
    actif: bool,
    present: bool,
    vers_ecran: Option<bool>,
}

/// La CONSCIENCE D'APPEL (retour Michee 2026-07-08 : « il ne sait pas qu'il
/// est en visio ») — forme COMPACTE pour l'en-tete frais du message (la
/// semantique « tu es capable de voir, utilise regarder » vit dans le prompt
/// systeme STABLE ; ici seulement l'etat de l'instant). None hors appel.
fn ligne_conscience(etat: &EtatAppel) -> Option<String> {
    if !etat.actif {
        return None;
    }
    Some(if etat.present {
        let mut t = String::from("en appel video : tu vois ton utilisateur");
        match etat.vers_ecran {
            Some(true) => t.push_str(", attentif a l'ecran"),
            Some(false) => t.push_str(", le regard ailleurs"),
            None => {}
        }
        t
    } else {
        "en appel video : ton utilisateur est hors du champ de la camera".to_string()
    })
}

/// Service d'appel loopback : expose au processus voix le cliche camera
/// (`/cliche`, JPEG 640) et l'etat de presence (`/etat`, JSON), et RECOIT
/// son pouls (`POST /pouls`, ch. 4 — etat de tour + niveau audio pour
/// l'eclipse). 127.0.0.1 uniquement, requetes minuscules, fermeture apres
/// reponse (pas de flux long -> piege Winsock n°7 sans objet).
fn serveur_appel(
    port: u16,
    etat: Arc<Mutex<EtatAppel>>,
    cliche: Arc<Mutex<Option<waly_sight::perception::Cliche>>>,
    pouls: Arc<Mutex<Pouls>>,
    // R5 : la voix compagnon lira l'ecran via /cliche-ecran quand le mode
    // Ecran est actif (parite avec /cliche = camera).
    ecran_actif: Arc<AtomicBool>,
    // R5 : etat du mute micro (le pop-up le bascule, la voix le lit sur /mic).
    mic_muted: Arc<AtomicBool>,
    // R5 : cible de capture (0 = tout l'ecran, sinon handle de fenetre).
    cible_ecran: Arc<std::sync::atomic::AtomicI64>,
    // R6b : la veille signale « Waly » entendu (POST /eveil) — ou retire un
    // eveil optimiste refute par le STT (POST /eveil-annule).
    eveil: Arc<Mutex<Option<std::time::Instant>>>,
    eveil_annule: Arc<Mutex<Option<std::time::Instant>>>,
    // UI 2026-09-14 : la conversation OUVERTE, ou la voix range ses messages.
    session_courante: Arc<std::sync::atomic::AtomicI64>,
) {
    use std::io::{Read, Write};
    let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("service d'appel: bind {port}: {e}");
            return;
        }
    };
    for stream in listener.incoming() {
        let Ok(mut s) = stream else { continue };
        let mut buf = [0u8; 512];
        let n = match s.read(&mut buf) {
            Ok(n) => n,
            Err(_) => continue,
        };
        let req = String::from_utf8_lossy(&buf[..n]);
        let (status, ctype, body): (&str, &str, Vec<u8>) = if req.starts_with("POST /pouls") {
            // Corps minuscule ({"etat":"parole","niveau":0.42}) : tient dans
            // la premiere lecture. Mise a jour silencieuse, reponse 204.
            if let Some(pos) = req.find("\r\n\r\n") {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&req[pos + 4..]) {
                    if let Ok(mut p) = pouls.lock() {
                        if let Some(e) = v["etat"].as_str() {
                            p.etat = e.to_string();
                        }
                        p.niveau = v["niveau"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0) as f32;
                        p.maj = std::time::Instant::now();
                    }
                }
            }
            ("204 No Content", "text/plain", Vec::new())
        } else if req.starts_with("POST /eveil-annule") {
            // ⚠ AVANT /eveil (prefixe commun). La confirmation STT a refute
            // l'eveil optimiste : l'UI referme la page voix.
            if let Ok(mut e) = eveil_annule.lock() {
                *e = Some(std::time::Instant::now());
            }
            ("204 No Content", "text/plain", Vec::new())
        } else if req.starts_with("POST /eveil") {
            // R6b : la veille a entendu « Waly ». L'UI sondera core_eveil
            // et jouera le « Souffle » — reponse immediate, aucun corps.
            if let Ok(mut e) = eveil.lock() {
                *e = Some(std::time::Instant::now());
            }
            ("204 No Content", "text/plain", Vec::new())
        } else if req.starts_with("GET /session") {
            let id = session_courante.load(Ordering::Relaxed);
            ("200 OK", "text/plain", id.to_string().into_bytes())
        } else if req.starts_with("GET /etat") {
            let e = etat.lock().map(|e| e.clone()).unwrap_or_default();
            let json = serde_json::json!({
                "actif": e.actif, "present": e.present,
                "vers_ecran": e.vers_ecran,
            });
            ("200 OK", "application/json", json.to_string().into_bytes())
        } else if req.starts_with("GET /mic") {
            let v = if mic_muted.load(Ordering::Relaxed) { "1" } else { "0" };
            ("200 OK", "text/plain", v.as_bytes().to_vec())
        } else if req.starts_with("GET /lecture-ecran") {
            // B1 (2026-09-11) : la voix lit l'ecran en TEXTE (arbre
            // d'accessibilite). Un lecteur par requete (COM sur CE thread) ;
            // rien n'est garde. `riche` = false -> la voix se replie sur l'image.
            if ecran_actif.load(Ordering::Relaxed) {
                let h = cible_ecran.load(Ordering::Relaxed);
                let lu = waly_sight::uia::Lecteur::new().and_then(|l| {
                    let hwnd = if h != 0 {
                        h as isize
                    } else {
                        waly_sight::uia::fenetre_utilisateur()
                            .map(|(h, _)| h)
                            .ok_or_else(|| "aucune fenetre a lire".to_string())?
                    };
                    l.lire(hwnd, waly_sight::uia::BUDGET_DEFAUT)
                });
                match lu {
                    Ok(i) => {
                        let j = serde_json::json!({
                            "fenetre": i.fenetre,
                            "texte": i.texte,
                            "riche": waly_sight::uia::est_riche(&i),
                        });
                        ("200 OK", "application/json", j.to_string().into_bytes())
                    }
                    Err(e) => ("503 Service Unavailable", "text/plain", e.into_bytes()),
                }
            } else {
                ("503 Service Unavailable", "text/plain", b"mode ecran inactif".to_vec())
            }
        } else if req.starts_with("GET /cliche-ecran") {
            // ⚠ AVANT /cliche (qui matcherait aussi). Cible du selecteur : 0 =
            // tout l'ecran, sinon la fenetre choisie. 720p pour le VLM.
            if ecran_actif.load(Ordering::Relaxed) {
                let h = cible_ecran.load(Ordering::Relaxed);
                let shot = if h == 0 {
                    waly_sight::screen::capture_screen()
                } else {
                    waly_sight::screen::capture_window(h as isize)
                };
                match shot.and_then(|s| s.jpeg(1600)) {
                    Ok((jpeg, _, _)) => ("200 OK", "image/jpeg", jpeg),
                    Err(e) => ("503 Service Unavailable", "text/plain", e.into_bytes()),
                }
            } else {
                ("503 Service Unavailable", "text/plain", b"mode ecran inactif".to_vec())
            }
        } else if req.starts_with("GET /cliche") {
            match cliche.lock().ok().and_then(|g| g.as_ref().map(|c| c.jpeg(640))) {
                Some(Ok((jpeg, _, _))) => ("200 OK", "image/jpeg", jpeg),
                Some(Err(e)) => ("503 Service Unavailable", "text/plain", e.into_bytes()),
                None => ("503 Service Unavailable", "text/plain", b"appel inactif".to_vec()),
            }
        } else {
            ("404 Not Found", "text/plain", b"?".to_vec())
        };
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = s.write_all(head.as_bytes());
        let _ = s.write_all(&body);
    }
}

/// Perception proactive AU REPOS du worker (R4.5 ch. 3). Politique GATE C :
/// n'est appelee que quand la file de commandes est vide ; chaque appel LLM
/// d'ici EVINCE le cache de conversation FLM (une case) — le tour de
/// dialogue suivant re-paie son prefill (~2 s), assume et gravé. Deux
/// etages :
/// - moment VLM : la scene a change (drapeau du forwarder) → decrire le
///   cliche en une phrase adressee a Michee → journal kind 'moment'
///   (la voix le dira au repos, le delta du ch. 2 le racontera au modele) ;
///   cooldown 90 s — le drapeau reste leve entre-temps.
/// - reflexion : toutes les ≥ 180 s s'il y a du NEUF au journal → UNE
///   inference d'activite → kind 'reflexion' ; la sentinelle RIEN jette.
fn proactif_au_repos(
    llm: &LlmClient,
    conn: &rusqlite::Connection,
    percepteur: &Rc<std::cell::RefCell<Option<waly_sight::perception::Percepteur>>>,
    etat_appel: &Arc<Mutex<EtatAppel>>,
    scene_a_decrire: &Arc<Mutex<Option<std::time::Instant>>>,
    dernier_moment: &mut std::time::Instant,
    reflexion_cursor: &mut i64,
    derniere_reflexion: &mut std::time::Instant,
    // R5 ch. 3b : session ecran active + drapeau de changement de scene ecran
    // (pose par la boucle de scene) + cooldown du moment ecran.
    ecran_actif: &Arc<AtomicBool>,
    scene_ecran: &Arc<Mutex<Option<std::time::Instant>>>,
    dernier_moment_ecran: &mut std::time::Instant,
) {
    // -- Moment VLM CAMERA (si appel actif) --
    let cam = percepteur.borrow();
    if let Some(p) = cam.as_ref() {
    if dernier_moment.elapsed().as_secs() >= 90 {
        let scene = scene_a_decrire.lock().ok().and_then(|mut s| s.take());
        if scene.is_some() {
            // Vision adaptative (2026-09-10) : le modele qui VOIT (cerveau, ou
            // modele vision declare) ; personne ne voit -> pas de moment.
            if let (Ok((jpeg, _, _)), Some(mut vlm)) =
                (p.jpeg(640), waly_core::vision::client_vision(llm))
            {
                let data_url = format!(
                    "data:image/jpeg;base64,{}",
                    waly_core::native_tools::base64(&jpeg)
                );
                vlm.max_tokens = 80;
                // Le moment CONNAIT la presence (vecu terrain 2026-07-08 :
                // force au « tu », le VLM inventait Michee absent — « tu
                // regardes la porte fermee » face a une piece vide).
                let present = etat_appel.lock().map(|e| e.present).unwrap_or(false);
                let consigne = if present {
                    "Tu es l'oeil de Waly pendant un appel video avec ton utilisateur. \
                     Decris ce que montre l'image en UNE seule phrase courte \
                     (20 mots max), factuelle, en t'adressant a ton utilisateur (« tu »). \
                     Pas de preambule, pas d'enumeration."
                } else {
                    "Tu es l'oeil de Waly pendant un appel video. Ton utilisateur est \
                     ABSENT du champ : decris la piece en UNE seule phrase \
                     courte (20 mots max), factuelle — n'invente PERSONNE. \
                     Pas de preambule, pas d'enumeration."
                };
                let messages = vec![
                    Msg::System(consigne.into()),
                    Msg::UserImage {
                        texte: "La scene vient de changer devant la camera.".into(),
                        data_url,
                    },
                ];
                if let Ok(waly_core::llm::Turn::Text(desc)) = vlm.chat(&messages, &[]) {
                    let desc = desc.trim();
                    if !desc.is_empty() {
                        store::visual_memory_add(
                            conn,
                            store::MAIN_SESSION,
                            "moment",
                            &waly_core::prompt::compacte(desc, 200),
                        )
                        .ok();
                        *dernier_moment = std::time::Instant::now();
                    }
                }
            }
        }
    }
    }
    drop(cam);
    // -- Moment proactif ECRAN : DÉSACTIVÉ (retour Michee 2026-07-10) : les
    // moments/inferences journalises ancraient Waly sur de vieilles scenes
    // (« terminal ») et POLLUAIENT la vision — il repondait de memoire au lieu
    // de regarder. La vision ecran est desormais FRAICHE a chaque tour (image
    // jointe par la commande Send). On draine juste le drapeau de scene.
    let _ = &dernier_moment_ecran; // conserve pour un futur moment propre
    if ecran_actif.load(Ordering::Relaxed) {
        let _ = scene_ecran.lock().ok().and_then(|mut s| s.take());
    }
    // -- Reflexion periodique (inference d'activite) — camera SEULEMENT (en
    // mode ecran elle inventait « Michee semble via un terminal » et polluait).
    // Reflexion SEULEMENT caméra allumée : sans elle, l'inference tournait sur
    // le vieux journal et nourrissait le delta de fausses observations.
    if !ecran_actif.load(Ordering::Relaxed)
        && percepteur.borrow().is_some()
        && derniere_reflexion.elapsed().as_secs() >= 180
    {
        *derniere_reflexion = std::time::Instant::now();
        let dernier_id = store::visual_memory_last_id(conn, store::MAIN_SESSION).unwrap_or(0);
        if dernier_id > *reflexion_cursor {
            *reflexion_cursor = dernier_id;
            let recent =
                store::visual_memory_recent(conn, store::MAIN_SESSION, 8).unwrap_or_default();
            let mut journal = String::new();
            for (heure, _kind, contenu) in &recent {
                journal.push_str(&format!("- [{heure}] {contenu}\n"));
            }
            let mut txt = LlmClient::new(&llm.host, llm.port, &llm.model);
            txt.max_tokens = 60;
            let messages = vec![
                Msg::System(
                    "Tu es Waly. Voici le journal recent de ce que tu as observe \
                     (camera et/ou ecran de ton utilisateur)."
                        .into(),
                ),
                Msg::User(format!(
                    "{journal}\nFormule UNE hypothese courte et utile sur ce que \
                     ton utilisateur est en train de faire (une seule phrase, commence par \
                     « Ton utilisateur semble »). Si rien de notable, reponds exactement : RIEN."
                )),
            ];
            if let Ok(waly_core::llm::Turn::Text(inf)) = txt.chat(&messages, &[]) {
                let inf = inf.trim();
                if !inf.is_empty() && !inf.to_uppercase().contains("RIEN") {
                    store::visual_memory_add(
                        conn,
                        store::MAIN_SESSION,
                        "reflexion",
                        &waly_core::prompt::compacte(inf, 200),
                    )
                    .ok();
                }
            }
        }
    }
}

/// Boucle de detection de scene ECRAN (R5 ch. 3b), thread dedie. Tant que la
/// session ecran est active, capture une vignette toutes les ~4 s, calcule un
/// dHash et signale un CHANGEMENT DE SCENE stable (SceneDetector R4.5 reutilise)
/// via `scene_ecran`. Aucun pixel garde : seul le hash 64 bits survit a la
/// frame. Le worker (au repos) transforme le drapeau en moment VLM journalise.
fn boucle_scene_ecran(actif: Arc<AtomicBool>, scene_ecran: Arc<Mutex<Option<std::time::Instant>>>) {
    let neuf = || {
        let mut d = waly_sight::perception::SceneDetector::default();
        // A ~0,25 Hz : 2 captures stables ≈ 8 s de nouvel ecran pose.
        d.frames_stables = 2;
        d.seuil_change = 18;
        d.seuil_stable = 10;
        d
    };
    let mut detecteur = neuf();
    let mut etait_actif = false;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(4));
        if !actif.load(Ordering::Relaxed) {
            if etait_actif {
                detecteur = neuf(); // repartir propre a la prochaine session
                etait_actif = false;
            }
            continue;
        }
        etait_actif = true;
        if let Ok(shot) = waly_sight::screen::capture_screen() {
            let hash =
                waly_sight::perception::dhash(&shot.rgb, shot.width as usize, shot.height as usize);
            if detecteur.pousser(hash) {
                if let Ok(mut f) = scene_ecran.lock() {
                    *f = Some(std::time::Instant::now());
                }
            }
        }
    }
}

/// Le thread worker : possede le stack waly-core, boucle sur les commandes.
fn worker(
    rx: Receiver<Cmd>,
    cancel: Arc<AtomicBool>,
    pouls: Arc<Mutex<Pouls>>,
    mic_muted: Arc<AtomicBool>,
    cible_ecran: Arc<std::sync::atomic::AtomicI64>,
    eveil: Arc<Mutex<Option<std::time::Instant>>>,
    eveil_annule: Arc<Mutex<Option<std::time::Instant>>>,
) {
    let path = db_path();
    if path != ":memory:" {
        if let Some(dir) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(dir);
        }
    }
    // Le coeur ouvre (et cree) la base EN PREMIER ; les fils de fond
    // attendent `BASE_PRETE`. Quelques essais : un verrou passager ne doit
    // pas laisser l'app sans cerveau (vecu 2026-10-01, base neuve).
    let mut ouverte = None;
    for essai in 0..10 {
        match store::open(&path) {
            Ok(c) => {
                ouverte = Some(c);
                break;
            }
            Err(e) => {
                eprintln!("Waly: ouverture base (essai {}): {e}", essai + 1);
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
    }
    BASE_PRETE.store(true, Ordering::SeqCst);
    let Some(conn) = ouverte.map(Rc::new) else {
        eprintln!("Waly: ouverture base impossible");
        return;
    };
    // Cerveau local choisi dans l'app (menu des modèles) : il prime, tant
    // qu'il est encore installé sur le moteur.
    let port = waly_core::llm::port_par_defaut();
    if let Some(choisi) = store::reglage(&conn, "modele_local") {
        let installes = LlmClient::new("127.0.0.1", port, "").modeles_installes();
        if port != waly_core::llm::PORT_MOTEUR_B || installes.is_empty() || installes.contains(&choisi) {
            waly_core::llm::choisir_modele(Some(choisi));
        } else {
            eprintln!("Waly: le modele choisi ({choisi}) n'est plus installe, retour au defaut");
        }
    }
    let mut llm = LlmClient::new("127.0.0.1", port, &waly_core::llm::modele_par_defaut());
    garder_chaud_en_fond(&llm);
    let mut registry = Registry::new();
    // Embedder non charge en v1 (memoire sur mot-cle) : plus leger, pas de
    // onnxruntime.dll requise, budget RAM tenu. Passera a l'embedder plus tard.
    register_core_tools(&mut registry, conn.clone(), None);
    waly_core::fichiers::register_fichier_tools(
        &mut registry,
        waly_core::fichiers::PolitiqueFichiers::defaut(),
    );
    // Serveurs MCP stdio de waly.toml (ADR 2026-09-10) : outils de la
    // communaute, approbation a chaque appel par defaut (hors du sceau).
    for ligne in waly_core::mcp::register_mcp_tools(&mut registry) {
        eprintln!("Waly: mcp: {ligne}");
    }
    // Outil sensible de demo (meme gate que le bin) : exerce le HITL en reel.
    if std::env::var("WALY_DEMO_SENSIBLE").as_deref() == Ok("1") {
        registry.register(Box::new(waly_core::native_tools::EnvoyerMessageDemo));
    }
    // L'oeil (R4) : le percepteur vit ici, l'outil `regarder` lit son cliche.
    // Mode appel eteint -> erreur honnete renvoyee au modele.
    let percepteur: Rc<std::cell::RefCell<Option<waly_sight::perception::Percepteur>>> =
        Rc::new(std::cell::RefCell::new(None));
    let cam = percepteur.clone();
    let cam_allumee = percepteur.clone();
    waly_core::native_tools::register_vision_tool_si(
        &mut registry,
        Box::new(move || {
            let garde = cam.borrow();
            let p = garde
                .as_ref()
                .ok_or("la caméra n'est pas active — l'utilisateur doit démarrer le mode appel")?;
            let (jpeg, w, h) = p.jpeg(640)?;
            Ok(waly_core::native_tools::ImageCapturee {
                data_url: format!(
                    "data:image/jpeg;base64,{}",
                    waly_core::native_tools::base64(&jpeg)
                ),
                largeur: w,
                hauteur: h,
            })
        }),
        Box::new(move || cam_allumee.borrow().is_some()),
    );
    let cliche_640 = |p: &waly_sight::perception::Percepteur| -> Option<String> {
        let (jpeg, _, _) = p.jpeg(640).ok()?;
        Some(format!("data:image/jpeg;base64,{}", waly_core::native_tools::base64(&jpeg)))
    };
    // L'ecran (R5 ch. 3) : l'OCR vit ici, charge quand le mode Ecran s'ouvre.
    // L'outil `regarder_ecran` lit cet Option ; mode Ecran eteint -> erreur
    // honnete au modele (meme regle que la camera).
    let ecran_ocr: Rc<std::cell::RefCell<Option<waly_sight::ocr::Ocr>>> =
        Rc::new(std::cell::RefCell::new(None));
    {
        let ecr = ecran_ocr.clone();
        let cib = cible_ecran.clone();
        let ecran_ouvert = ecran_ocr.clone();
        waly_core::native_tools::register_vision_ecran_tool_si(
            &mut registry,
            Box::new(move |_cadrage| capture_ecran_ocr(&ecr, &cib)),
            Box::new(move || ecran_ouvert.borrow().is_some()),
        );
    }
    // Session ecran active (signal cross-thread : la boucle de scene est un
    // thread dedie, ne peut pas lire l'Rc<RefCell> ci-dessus) + drapeau de
    // changement de scene ecran (R5 ch. 3b).
    let ecran_actif = Arc::new(AtomicBool::new(false));
    let scene_ecran: Arc<Mutex<Option<std::time::Instant>>> = Arc::new(Mutex::new(None));
    {
        let (a, s) = (ecran_actif.clone(), scene_ecran.clone());
        std::thread::spawn(move || boucle_scene_ecran(a, s));
    }
    // Les mains d'ecran (B, 2026-09-11) : lire_ecran / agir_ecran, au
    // catalogue SEULEMENT pendant le partage (Tool::disponible) ; chaque
    // action passe par une approbation au libelle humain. Lecture de la
    // fenetre cible par l'arbre d'accessibilite (texte exact, ~0,1 s, zero
    // pixel) — hote dans waly-sight (feature `mains`), vit dans ce worker.
    let hote_ecran = Rc::new(waly_sight::mains::HoteUia::new(
        ecran_actif.clone(),
        cible_ecran.clone(),
    ));
    waly_core::mains_ecran::register_mains_ecran(&mut registry, hote_ecran.clone());
    let mut dernier_moment_ecran = std::time::Instant::now();
    // Tours ecran : reponses breves (GATE 3 R5) -> plafond de decodage dedie.
    let mut llm_court = LlmClient::new("127.0.0.1", waly_core::llm::port_par_defaut(),&waly_core::llm::modele_par_defaut());
    llm_court.max_tokens = 120;
    // Style « détaillé » (réglage) : réponses longues -> plafond relevé.
    let mut llm_long = LlmClient::new("127.0.0.1", waly_core::llm::port_par_defaut(),&waly_core::llm::modele_par_defaut());
    llm_long.max_tokens = 1200;
    // La voix compagnon (R4 ch. 5) : waly-voice.exe spawne avec l'appel,
    // tue a l'arret. Meme base (WAL + busy_timeout), Fil principal partage.
    let mut voix: Option<std::process::Child> = None;
    // Voix compagnon du mode ECRAN (R5) : spawnee au demarrage du partage
    // d'ecran, tuee a l'arret. Slot separe de l'appel (peuvent coexister).
    let mut voix_ecran: Option<std::process::Child> = None;
    // B3 : la session « regarde-moi » en cours (hooks poses sur son propre thread).
    let mut regarde: Option<waly_sight::demo::Session> = None;
    // Veille wake word (R6b) : UN seul micro-resident — tuee pendant
    // Appel/Ecran (waly-voice y ecoute deja), relancee apres. Chien de
    // garde au repos : si le processus meurt (micro perdu...), respawn
    // avec un garde-fou de 30 s.
    let mut veille: Option<std::process::Child> = None;
    let mut veille_relance = std::time::Instant::now();
    // Mode VOIX (R6b) : conversation vocale sans camera, slot separe de
    // l'appel et de l'ecran.
    let mut voix_seule: Option<std::process::Child> = None;
    // Service d'appel loopback : etat de presence + cliche pour le processus
    // voix (conscience d'appel + outil regarder a l'oral).
    let appel_port: u16 = std::env::var("WALY_APPEL_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(52710);
    let etat_appel = Arc::new(Mutex::new(EtatAppel::default()));
    let cliche_partage: Arc<Mutex<Option<waly_sight::perception::Cliche>>> =
        Arc::new(Mutex::new(None));
    // R4.5 ch. 3 : le forwarder leve ce drapeau sur Event::Scene ; le worker
    // le consomme AU REPOS (moment VLM proactif). Il reste leve tant que le
    // cooldown n'est pas passe — la scene la plus recente sera decrite.
    let scene_a_decrire: Arc<Mutex<Option<std::time::Instant>>> = Arc::new(Mutex::new(None));
    // La conversation ouverte, publiee a la voix (GET /session) — mise a jour
    // en tete de chaque tour de boucle, donc apres chaque commande.
    let session_courante = Arc::new(std::sync::atomic::AtomicI64::new(store::MAIN_SESSION));
    {
        let (e, c, p) = (etat_appel.clone(), cliche_partage.clone(), pouls);
        let (ea, mm, ci) = (ecran_actif.clone(), mic_muted.clone(), cible_ecran.clone());
        let ev = eveil.clone();
        let eva = eveil_annule.clone();
        let sc = session_courante.clone();
        std::thread::spawn(move || serveur_appel(appel_port, e, c, p, ea, mm, ci, ev, eva, sc));
    }
    // Veille wake word (R6b) : demarre avec l'app — dire « Waly » reveille
    // la presence sans clic. Best effort (pas de micro = pas de veille).
    if !veille_desactivee() {
        match demarrer_veille(appel_port) {
            Ok(child) => veille = Some(child),
            Err(e) => eprintln!("veille indisponible: {e}"),
        }
    }

    // Reprendre la ou on etait : la session du dernier message.
    let mut current = store::last_active_session(&conn).unwrap_or(store::MAIN_SESSION);
    let is_agent =
        |conn: &rusqlite::Connection, id: i64| store::session_kind(conn, id).ok() == Some("agent".into());
    let mut messages: Vec<Msg> = Vec::new();
    // Sélection d'outils ADAPTATIVE (selection.rs) : petit modèle local =
    // outils utiles au message ; gros modèle ou inconnu (clé API) = tous.
    appliquer_mode_outils(&llm, &conn, &registry);
    rebuild_window(&mut messages, &conn, current, is_agent(&conn, current));
    // 1er tour d'une session = 23 s mesures (prefill systeme + outils) contre
    // ~3,5 s ensuite : on paie ce prefill MAINTENANT, en fond, au demarrage.
    prechauffer_prefixe_en_fond(&llm, &messages, &registry);
    // Curseur du delta visuel « depuis ton dernier tour » (R4.5 ch. 2) —
    // les evenements vivent sur le Fil principal (la camera est globale).
    let mut vu_jusqu_a =
        store::visual_memory_last_id(&conn, store::MAIN_SESSION).unwrap_or(0);
    // Perception proactive (ch. 3) : etats du repos.
    let mut dernier_moment = std::time::Instant::now();
    let mut reflexion_cursor = vu_jusqu_a;
    let mut derniere_reflexion = std::time::Instant::now();
    // Échéances (rappels, missions programmées) : balayées au repos.
    let mut derniere_echeance = std::time::Instant::now() - std::time::Duration::from_secs(60);
    // Résumé des longues conversations : au repos seulement (tour LLM court).
    let mut derniere_activite = std::time::Instant::now();
    let mut dernier_resume = std::time::Instant::now();

    // Boucle de commandes avec REPOS : quand rien n'arrive pendant 1 s, la
    // perception proactive a le droit de tourner (politique GATE C — le
    // dialogue preempte par construction, un moment n'est jamais lance
    // pendant qu'une commande attend).
    loop {
        session_courante.store(current, Ordering::Relaxed);
        let cmd = match rx.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(cmd) => {
                // Seuls les TOURS comptent comme activité : l'UI sonde le worker
                // toutes les 5 s (attentes), ce qui empêchait tout repos.
                if matches!(cmd, Cmd::Send { .. } | Cmd::AgentStart { .. } | Cmd::Approve { .. } | Cmd::Passerelle { .. }) {
                    derniere_activite = std::time::Instant::now();
                }
                cmd
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Chien de garde veille (R6b) : si le processus est mort
                // (micro perdu, erreur), respawn — garde-fou 30 s pour ne
                // pas boucler sur une panne persistante.
                if !veille_desactivee()
                    && voix.is_none()
                    && voix_ecran.is_none()
                    && voix_seule.is_none()
                {
                    let mort = match veille.as_mut() {
                        Some(c) => c.try_wait().ok().flatten().is_some(),
                        None => true,
                    };
                    if mort
                        && veille_relance.elapsed() > std::time::Duration::from_secs(30)
                    {
                        veille_relance = std::time::Instant::now();
                        if let Some(mut v) = veille.take() {
                            let _ = v.wait();
                        }
                        match demarrer_veille(appel_port) {
                            Ok(c) => veille = Some(c),
                            Err(e) => eprintln!("veille: relance impossible: {e}"),
                        }
                    }
                }
                if derniere_activite.elapsed() > std::time::Duration::from_secs(20)
                    && dernier_resume.elapsed() > std::time::Duration::from_secs(30)
                {
                    dernier_resume = std::time::Instant::now();
                    match waly_core::resume::resumer_session(&llm, &conn, current) {
                        Ok(true) => eprintln!("Waly: resume de la session {current} mis a jour"),
                        Ok(false) => {}
                        Err(e) => eprintln!("Waly: resume: {e}"),
                    }
                }
                if derniere_echeance.elapsed() > std::time::Duration::from_secs(15) {
                    derniere_echeance = std::time::Instant::now();
                    // Une voix vivante énonce elle-même les rappels (waly-voice) ;
                    // sinon c'est l'app — depuis le retrait de la veille (14/09)
                    // plus personne ne les déclenchait hors appel.
                    if voix.is_none() && voix_ecran.is_none() && voix_seule.is_none() {
                        for r in store::due_reminders(&conn).unwrap_or_default() {
                            signaler("rappel", serde_json::json!({"titre": r.title, "message": r.message}));
                        }
                    }
                    for m in store::due_missions_programmees(&conn).unwrap_or_default() {
                        lancer_mission_programmee(&llm, &registry, &conn, &cancel, &m);
                    }
                }
                proactif_au_repos(
                    &llm,
                    &conn,
                    &percepteur,
                    &etat_appel,
                    &scene_a_decrire,
                    &mut dernier_moment,
                    &mut reflexion_cursor,
                    &mut derniere_reflexion,
                    &ecran_actif,
                    &scene_ecran,
                    &mut dernier_moment_ecran,
                );
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match cmd {
            Cmd::History { reply } => {
                let h = store::recent_messages_in(&conn, current, 30).unwrap_or_default();
                let _ = reply.send((current, h));
            }
            Cmd::Sessions { kind, reply } => {
                let _ = reply.send(store::list_sessions(&conn, &kind).unwrap_or_default());
            }
            Cmd::NewSession { reply } => match store::create_session(&conn, "Conversation", "chat") {
                Ok(id) => {
                    current = id;
                    rebuild_window(&mut messages, &conn, current, false); // vierge
                    let _ = reply.send(id);
                }
                Err(_) => {
                    let _ = reply.send(current); // degrade : on reste ou on est
                }
            },
            Cmd::SelectSession { id, reply } => {
                current = id;
                rebuild_window(&mut messages, &conn, current, is_agent(&conn, current));
                let h = store::recent_messages_in(&conn, current, 30).unwrap_or_default();
                let _ = reply.send(h);
            }
            Cmd::DeleteSession { id, reply } => {
                let res = match store::delete_session(&conn, id) {
                    Ok(true) => {
                        if current == id {
                            current = store::last_active_session(&conn).unwrap_or(store::MAIN_SESSION);
                            rebuild_window(&mut messages, &conn, current, is_agent(&conn, current));
                        }
                        Ok(current)
                    }
                    Ok(false) => Err("conversation introuvable".into()),
                    Err(e) => Err(e.to_string()),
                };
                let _ = reply.send(res);
            }
            Cmd::Search { query, kind, reply } => {
                let _ = reply.send(store::search_sessions(&conn, &query, &kind).unwrap_or_default());
            }
            Cmd::Rebuild { reply } => {
                rebuild_window(&mut messages, &conn, current, is_agent(&conn, current));
                let _ = reply.send(());
            }
            Cmd::Reglage { cle, valeur, reply } => {
                let r = store::set_reglage(&conn, &cle, &valeur).map_err(|e| e.to_string());
                if r.is_ok() {
                    if cle == "outils" {
                        appliquer_mode_outils(&llm, &conn, &registry);
                    }
                    rebuild_window(&mut messages, &conn, current, is_agent(&conn, current));
                }
                let _ = reply.send(r);
            }
            Cmd::Passerelle { id, texte, reply } => {
                // Conversation dédiée, créée au premier message (recréée si
                // l'utilisateur l'a supprimée) : la conversation ouverte n'est
                // pas touchée.
                let cle = format!("passerelle_session_{id}");
                let session = store::reglage(&conn, &cle)
                    .and_then(|s| s.parse::<i64>().ok())
                    .filter(|s| store::session_kind(&conn, *s).is_ok())
                    .or_else(|| {
                        let s = store::create_session(&conn, "Téléphone · Telegram", "chat").ok()?;
                        store::set_reglage(&conn, &cle, &s.to_string()).ok();
                        Some(s)
                    });
                let Some(session) = session else {
                    let _ = reply.send(Err("conversation de la passerelle introuvable".into()));
                    continue;
                };
                let mut fenetre = Vec::new();
                rebuild_window(&mut fenetre, &conn, session, false);
                let muet: Channel<StreamMsg> = Channel::new(|_| Ok(()));
                let res = run_streamed_turn(
                    &llm, &registry, &conn, &mut fenetre, &cancel, &muet, session, &texte, false, None,
                    Some("message recu par Telegram, depuis le telephone de ton utilisateur (camera eteinte) : \
                          reponds en texte simple, sans mise en forme".into()),
                    "", None, false,
                );
                store::set_session_title(&conn, session, "Téléphone · Telegram").ok();
                if session == current {
                    rebuild_window(&mut messages, &conn, current, false);
                }
                signaler("passerelle", serde_json::json!({"session": session}));
                let _ = reply.send(res);
            }
            Cmd::Modele { nom, reply } => {
                let r = store::set_reglage(&conn, "modele_local", &nom).map_err(|e| e.to_string());
                if r.is_ok() {
                    waly_core::llm::choisir_modele(Some(nom.clone()));
                    llm = LlmClient::new("127.0.0.1", llm.port, &nom);
                    llm_court = LlmClient::new("127.0.0.1", llm.port, &nom);
                    llm_court.max_tokens = 120;
                    llm_long = LlmClient::new("127.0.0.1", llm.port, &nom);
                    llm_long.max_tokens = 1200;
                    appliquer_mode_outils(&llm, &conn, &registry);
                    rebuild_window(&mut messages, &conn, current, is_agent(&conn, current));
                    garder_chaud_en_fond_une_fois(&llm);
                }
                let _ = reply.send(r);
            }
            Cmd::Skills { reply } => {
                let s = registry
                    .specs()
                    .into_iter()
                    .map(|s| (s.name, s.description))
                    .collect();
                let _ = reply.send(s);
            }
            Cmd::Artifacts { reply } => {
                let notes = store::list_notes(&conn).unwrap_or_default();
                let taches = store::list_open_tasks(&conn).unwrap_or_default();
                let rappels = store::list_active_reminders(&conn).unwrap_or_default();
                let v = serde_json::json!({
                    "notes": notes.iter().map(|(id, titre)|
                        serde_json::json!({"id": id, "titre": titre})).collect::<Vec<_>>(),
                    "taches": taches.iter().map(|t| serde_json::json!({
                        "id": t.id, "titre": t.title, "priorite": t.priority,
                        "statut": t.status, "echeance": t.due_date})).collect::<Vec<_>>(),
                    "rappels": rappels.iter().map(|r| serde_json::json!({
                        "id": r.id, "titre": r.title, "quand": r.remind_at,
                        "recurrence": r.recurrence})).collect::<Vec<_>>(),
                });
                let _ = reply.send(v);
            }
            Cmd::Pendings { reply } => {
                let p = store::list_pending(&conn)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|p| (p.id, p.tool_name, p.tool_args))
                    .collect();
                let _ = reply.send(p);
            }
            Cmd::Send { message, image: image_jointe, fichiers, chan, reply } => {
                let agent = is_agent(&conn, current);
                // Pendant l'appel, la voix ecrit aussi dans la session : la
                // fenetre en RAM se rafraichit depuis la base avant le tour
                // (rebuild assume — les deux ecrivains s'evincent le cache
                // FLM de toute facon, une case).
                if voix.is_some() || voix_ecran.is_some() {
                    rebuild_window(&mut messages, &conn, current, agent);
                }
                // R5 (revu 2026-07-10, retour Michee : « il decrit un souvenir,
                // pas mon ecran ») : en mode Ecran, CHAQUE tour capture une
                // image FRAICHE de l'ecran et le modele repond d'apres CE QU'IL
                // VOIT MAINTENANT — jamais d'apres le journal (qui l'ancrait sur
                // de vieilles descriptions). Vision temps reel = fraiche a
                // chaque question, zero souvenir parasite.
                let mut contexte_ecran: Option<String> = None;
                let mut image_ecran: Option<String> = None;
                let mut tour_ecran = false;
                let en_ecran = !agent && ecran_ocr.borrow().is_some();
                // B1 (2026-09-11) : l'ecran se lit d'abord en TEXTE (arbre
                // d'accessibilite : ~0,1 s, texte exact, ids [n] pour
                // agir_ecran) -> tour texte ~3,5 s au lieu d'un tour visuel
                // (86 s sur la machine de reference, vision deleguee). L'image
                // + OCR (R5) ne servent plus que si la demande est VISUELLE ou
                // si l'arbre est pauvre (canvas, PDF image, jeu).
                let lecture = if en_ecran && !intention_visuelle(&message) {
                    match hote_ecran.instantane() {
                        Ok(i) if waly_sight::uia::est_riche(&i) => Some(i),
                        Ok(_) => None,
                        Err(e) => {
                            eprintln!("Waly: lecture ecran: {e}");
                            None
                        }
                    }
                } else {
                    None
                };
                if en_ecran {
                    // Une seule lecture vivante : les anciennes quittent le
                    // contexte (mutation en QUEUE d'historique, re-prefill court).
                    waly_core::mains_ecran::degrader_lectures(&mut messages);
                }
                if let Some(i) = &lecture {
                    contexte_ecran = Some(format!(
                        "{}\n{}\n",
                        waly_core::mains_ecran::bloc(&i.fenetre, &i.texte),
                        waly_core::mains_ecran::CONSIGNE
                    ));
                } else if en_ecran {
                    match capture_ecran_ocr(&ecran_ocr, &cible_ecran) {
                        Ok(cap) => {
                            let ocr = cap.texte_ocr.trim();
                            let aide = if ocr.is_empty() {
                                String::new()
                            } else {
                                format!("[Texte lu à l'écran MAINTENANT (OCR) :\n{ocr}\n]\n")
                            };
                            contexte_ecran = Some(format!(
                                "{aide}(Tu vois l'écran de ton utilisateur MAINTENANT sur l'image jointe. \
                                 Réponds d'après CETTE image (jamais un souvenir). ⚠ N'INVENTE \
                                 RIEN : pour un nom de fichier ou du texte précis, fie-toi \
                                 UNIQUEMENT au texte OCR ci-dessus ; s'il n'y figure pas, ne le \
                                 nomme pas — dis « un fichier » sans deviner, ou « je ne \
                                 distingue pas ». Ignore le pop-up et le cadre Waly. Bref.)\n"
                            ));
                            image_ecran = Some(cap.image.data_url);
                            tour_ecran = true;
                        }
                        Err(e) => eprintln!("Waly: capture ecran: {e}"),
                    }
                }
                // Raccourci du mode appel : intention visuelle + camera active
                // -> l'image part AVEC le message (un seul tour de modele).
                // Ecran prioritaire : pas de double capture.
                let image_utilisateur = image_jointe.is_some();
                let image = if tour_ecran {
                    image_ecran
                } else if image_jointe.is_some() {
                    image_jointe
                } else if !agent && intention_visuelle(&message) {
                    percepteur.borrow().as_ref().and_then(&cliche_640)
                } else {
                    None
                };
                // Conscience d'appel pour les tours TAPES aussi.
                let conscience = etat_appel.lock().ok().and_then(|e| ligne_conscience(&e));
                let camera_active = conscience.is_some();
                // Honnetete (vécu 2026-09-14) : sans camera ni ecran, l'en-tete
                // le DIT — sinon le 4B « voyait » une piece inventee.
                // Image jointe par l'utilisateur : il la VOIT (sinon « camera
                // eteinte » l'emportait — vécu 2026-09-30 : « je ne vois rien »).
                let conscience = if image_utilisateur && !tour_ecran {
                    Some("image jointe par ton utilisateur : tu la vois sur ce message, decris-la d'apres ce que tu vois".to_string())
                } else {
                    conscience.or_else(|| {
                        (!en_ecran).then(|| "camera eteinte : tu ne vois rien en ce moment".to_string())
                    })
                };
                // Delta visuel : seulement caméra ALLUMÉE (hors appel, il
                // racontait de vieux « tu reviens » comme du present), jamais
                // pour les missions ni les tours ecran.
                let (delta, curseur) = if agent || en_ecran || !camera_active {
                    (String::new(), vu_jusqu_a)
                } else {
                    waly_core::prompt::delta_visuel(&conn, store::MAIN_SESSION, vu_jusqu_a)
                };
                // Tour ecran : plafond de decodage (reponses breves, GATE 3).
                // Modèle extérieur (lot 3) : choisi par l'utilisateur, ou par le
                // routeur « Auto ». Mission, écran et image restent TOUJOURS
                // locaux — le routeur choisit un modèle, pas ce qui sort.
                let cerveau = store::reglage(&conn, "cerveau").unwrap_or_else(|| "local".into());
                let exterieur = (cerveau != "local")
                    .then(|| store::reglage(&conn, "exterieur_id"))
                    .flatten()
                    .and_then(|i| i.parse::<i64>().ok())
                    .and_then(|i| waly_core::exterieur::trouver(&conn, i));
                if let Some(cfg) = &exterieur {
                    use waly_core::exterieur::{router, Cible, Tour};
                    let reste_local = agent || en_ecran || image.is_some();
                    let (cible, raison) = if cerveau == "auto" {
                        let connecteurs: Vec<String> =
                            waly_core::mcp::actifs().into_iter().map(|s| s.nom).collect();
                        router(&Tour {
                            message: &message,
                            mission: agent,
                            ecran: en_ecran,
                            image: image.is_some(),
                            outils: !waly_core::selection::groupes_pour(&message, &connecteurs).is_empty(),
                        })
                    } else if reste_local {
                        (Cible::Local, "mission, écran ou image : reste sur la machine")
                    } else {
                        (Cible::Exterieur, "modèle choisi")
                    };
                    let sort = cible == Cible::Exterieur;
                    let _ = chan.send(StreamMsg::Route {
                        cible: if sort { "exterieur" } else { "local" }.into(),
                        modele: if sort { cfg.modele.clone() } else { llm.model.clone() },
                        hote: if sort { cfg.hote() } else { String::new() },
                        raison: raison.into(),
                    });
                    if sort {
                        let res = run_external_turn(
                            &conn, cfg, &mut messages, &cancel, &chan, current, &message, fichiers,
                        );
                        let _ = reply.send(res);
                        continue;
                    }
                }
                let detaille = store::reglage(&conn, "style").as_deref() == Some("detaille");
                // Reflexion approfondie : conversation seulement (ni mission,
                // ni partage d'ecran, ni image) ; lue a chaque tour, sans
                // rebuild — la consigne voyage dans le message.
                let reflexion = !agent
                    && !en_ecran
                    && store::reglage(&conn, "reflexion").as_deref() == Some("oui");
                let mut llm_reflexion = LlmClient::new(&llm.host, llm.port, &llm.model);
                llm_reflexion.max_tokens = if detaille { 2400 } else { 1600 };
                llm_reflexion.reflexion = true;
                let llm_tour = if tour_ecran {
                    &llm_court
                } else if reflexion {
                    &llm_reflexion
                } else if detaille {
                    &llm_long
                } else {
                    &llm
                };
                let res = run_streamed_turn(
                    llm_tour, &registry, &conn, &mut messages, &cancel, &chan, current, &message,
                    agent, image, conscience, &delta, contexte_ecran, reflexion,
                );
                // Curseur avance seulement si le delta a ete ENTENDU (tour
                // non annule avant le premier mot).
                if matches!(&res, Ok(s) if !s.is_empty()) {
                    vu_jusqu_a = curseur;
                }
                let _ = reply.send(res);
            }
            Cmd::VoixStart { reply } => {
                if voix_seule.is_some() {
                    let _ = reply.send(Ok("voix déjà active".into()));
                    continue;
                }
                // Un seul micro : la veille cede la place.
                if let Some(mut v) = veille.take() {
                    let _ = v.kill();
                    let _ = v.wait();
                }
                // La voix parle dans la conversation OUVERTE (decision Michee
                // 2026-09-14 : pas de Fil principal).
                match demarrer_voix(appel_port, false, false, current) {
                    Ok(child) => {
                        voix_seule = Some(child);
                        let _ = reply.send(Ok("voix active — parle".into()));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(format!("voix indisponible: {e}")));
                        // La veille reprend si la voix n'a pas pu partir.
                        if !veille_desactivee() && voix.is_none() && voix_ecran.is_none() {
                            veille = demarrer_veille(appel_port).ok();
                        }
                    }
                }
            }
            Cmd::VoixStop { reply } => {
                // Tue la voix du mode voix ET la veille-devenue-conversation
                // (cas eveil) : les deux chemins se terminent ici.
                if let Some(mut child) = voix_seule.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                if let Some(mut v) = veille.take() {
                    let _ = v.kill();
                    let _ = v.wait();
                }
                if !veille_desactivee() && voix.is_none() && voix_ecran.is_none() {
                    match demarrer_veille(appel_port) {
                        Ok(c) => veille = Some(c),
                        Err(e) => eprintln!("veille: {e}"),
                    }
                }
                let _ = reply.send("voix terminée".into());
            }
            Cmd::AppelStart { chan, reply } => {
                if let Some(p) = percepteur.borrow().as_ref() {
                    let _ = reply.send(Ok(("déjà actif".into(), p.cliche())));
                    continue;
                }
                let yunet = std::env::var("WALY_YUNET").unwrap_or_else(|_| {
                    waly_core::chemins::modele("yunet/face_detection_yunet_2023mar.onnx")
                });
                match waly_sight::perception::demarrer(
                    0,
                    waly_sight::perception::Config::default(),
                    &yunet,
                ) {
                    Ok((events, p)) => {
                        let cliche = p.cliche();
                        *percepteur.borrow_mut() = Some(p);
                        if let Ok(mut c) = cliche_partage.lock() {
                            *c = Some(cliche.clone());
                        }
                        if let Ok(mut e) = etat_appel.lock() {
                            *e = EtatAppel { actif: true, ..Default::default() };
                        }
                        // Les evenements partent vers l'UI depuis un forwarder
                        // dedie (le worker reste libre) qui tient AUSSI l'etat
                        // de presence a jour pour la conscience d'appel, ET
                        // ecrit le JOURNAL VISUEL (R4.5 ch. 1 : la perception
                        // s'ecrit en texte horodate — sa propre connexion,
                        // 3e ecrivain assume, WAL + busy_timeout).
                        let etat_fwd = etat_appel.clone();
                        let scene_fwd = scene_a_decrire.clone();
                        let jconn = store::open(&db_path()).ok();
                        std::thread::spawn(move || {
                            use waly_sight::perception::Event;
                            // Retour apres LONGUE absence = moment FORT
                            // (dicible a voix haute), pas un simple evenement.
                            let mut depart_a: Option<std::time::Instant> = None;
                            // Les transitions de presence ne declenchent PAS
                            // de moment scene (vecu terrain : le flou de la
                            // re-entree decrit au lieu du retour reconnu) —
                            // elles ont leurs propres evenements.
                            let mut derniere_presence = std::time::Instant::now();
                            for ev in events {
                                if let Ok(mut e) = etat_fwd.lock() {
                                    match &ev {
                                        Event::Arrivee { .. } => e.present = true,
                                        Event::Depart => {
                                            e.present = false;
                                            e.vers_ecran = None;
                                        }
                                        Event::Attention { vers_ecran } => {
                                            e.vers_ecran = Some(*vers_ecran)
                                        }
                                        Event::Scene => {}
                                    }
                                }
                                match &ev {
                                    Event::Arrivee { .. } | Event::Depart => {
                                        derniere_presence = std::time::Instant::now();
                                    }
                                    Event::Scene
                                        if derniere_presence.elapsed().as_secs() >= 10 =>
                                    {
                                        if let Ok(mut s) = scene_fwd.lock() {
                                            *s = Some(std::time::Instant::now());
                                        }
                                    }
                                    _ => {}
                                }
                                // Journal : arrivee/depart (l'attention
                                // est trop bavarde pour une memoire). Les
                                // evenements n'arrivent QUE sur changement
                                // (hysteresis) — pas de throttle a ajouter.
                                if let Some(conn) = &jconn {
                                    // Moments FORTS (kind 'moment', adresses a
                                    // Michee — la voix les dira tels quels) :
                                    // retour apres > 2 min, plusieurs visages.
                                    let moment = match &ev {
                                        Event::Arrivee { visages } if *visages > 1 => {
                                            Some(format!(
                                                "quelqu'un t'a rejoint devant la camera \
                                                 ({visages} personnes)"
                                            ))
                                        }
                                        Event::Arrivee { .. } => depart_a
                                            .take()
                                            .filter(|d| d.elapsed().as_secs() > 120)
                                            .map(|d| {
                                                format!(
                                                    "te revoila apres {} minutes d'absence",
                                                    (d.elapsed().as_secs() / 60).max(1)
                                                )
                                            }),
                                        _ => None,
                                    };
                                    if let Some(m) = moment {
                                        let _ = store::visual_memory_add(
                                            conn,
                                            store::MAIN_SESSION,
                                            "moment",
                                            &m,
                                        );
                                    }
                                    let ligne = match &ev {
                                        Event::Arrivee { visages: 1 } => {
                                            Some("Ton utilisateur apparait a la camera".to_string())
                                        }
                                        Event::Arrivee { visages } => Some(format!(
                                            "{visages} visages apparaissent a la camera"
                                        )),
                                        Event::Depart => {
                                            depart_a = Some(std::time::Instant::now());
                                            Some("Ton utilisateur sort du champ de la camera".into())
                                        }
                                        Event::Attention { .. } | Event::Scene => None,
                                    };
                                    if let Some(l) = ligne {
                                        let _ = store::visual_memory_add(
                                            conn,
                                            store::MAIN_SESSION,
                                            "evenement",
                                            &l,
                                        );
                                    }
                                }
                                if chan.send(ev).is_err() {
                                    break; // fenetre partie
                                }
                            }
                        });
                        // Veille + mode voix (R6b) : UN seul micro — l'appel
                        // prend la main.
                        if let Some(mut v) = veille.take() {
                            let _ = v.kill();
                            let _ = v.wait();
                        }
                        if let Some(mut v) = voix_seule.take() {
                            let _ = v.kill();
                            let _ = v.wait();
                        }
                        // Voix compagnon : presente -> l'appel parle ; absente
                        // -> appel vision seule, dit honnetement.
                        let etat_voix = match demarrer_voix(appel_port, false, true, current) {
                            Ok(child) => {
                                voix = Some(child);
                                "voix active"
                            }
                            Err(e) => {
                                eprintln!("voix compagnon indisponible: {e}");
                                "vision seule (voix indisponible)"
                            }
                        };
                        let _ =
                            reply.send(Ok((format!("appel démarré — {etat_voix}"), cliche)));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
            Cmd::AppelStop { reply } => {
                if let Some(mut child) = voix.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                // Le micro est rendu : la veille reprend (sauf si l'ecran
                // ecoute encore).
                if !veille_desactivee() && voix_ecran.is_none() && veille.is_none() {
                    match demarrer_veille(appel_port) {
                        Ok(c) => veille = Some(c),
                        Err(e) => eprintln!("veille: {e}"),
                    }
                }
                if let Ok(mut c) = cliche_partage.lock() {
                    *c = None;
                }
                if let Ok(mut e) = etat_appel.lock() {
                    *e = EtatAppel::default();
                }
                let stats = match percepteur.borrow_mut().take() {
                    Some(p) => {
                        let s = p.arreter();
                        format!(
                            "{} cycles à {:.1} Hz — capture {:.1} ms, visage {:.1} ms",
                            s.cycles,
                            s.cadence_effective_hz,
                            s.grab_ms_med,
                            s.detect_ms_med
                        )
                    }
                    None => "déjà arrêté".into(),
                };
                let _ = reply.send(stats);
            }
            Cmd::EcranStart { reply } => {
                if ecran_ocr.borrow().is_some() {
                    let _ = reply.send(Ok("déjà actif".into()));
                } else {
                    waly_sight::screen::rendre_conscient_dpi();
                    match waly_sight::ocr::Ocr::load_default() {
                        Ok(ocr) => {
                            *ecran_ocr.borrow_mut() = Some(ocr);
                            ecran_actif.store(true, Ordering::SeqCst);
                            dernier_moment_ecran = std::time::Instant::now(); // laisse la scene se poser
                            // L'ecran se partage DANS la conversation ouverte
                            // (UI 2026-09-14 : « Partager l'ecran » du + ) ; la
                            // voix et le pop-up y ecrivent.
                            let sid = current;
                            // Veille + mode voix (R6b) : UN seul micro.
                            if let Some(mut v) = veille.take() {
                                let _ = v.kill();
                                let _ = v.wait();
                            }
                            if let Some(mut v) = voix_seule.take() {
                                let _ = v.kill();
                                let _ = v.wait();
                            }
                            // Voix compagnon (R5) : Waly parle de l'ecran (mode
                            // voix + chat). L'ecran doit etre ACTIF avant le
                            // spawn (la voix lira /cliche-ecran).
                            let etat_voix = match demarrer_voix(appel_port, true, false, sid) {
                                Ok(child) => {
                                    voix_ecran = Some(child);
                                    "voix active"
                                }
                                Err(e) => {
                                    eprintln!("voix ecran indisponible: {e}");
                                    "chat seul (voix indisponible)"
                                }
                            };
                            let _ = reply
                                .send(Ok(format!("partage d'écran actif — {etat_voix}")));
                        }
                        Err(e) => {
                            let _ = reply.send(Err(format!("OCR indisponible: {e}")));
                        }
                    }
                }
            }
            Cmd::RegardeStart { reply } => {
                if regarde.is_some() {
                    let _ = reply.send(Err("je te regarde déjà".into()));
                    continue;
                }
                match waly_sight::demo::Session::demarrer() {
                    Ok(s) => {
                        regarde = Some(s);
                        let _ = reply.send(Ok("je te regarde faire".into()));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
            Cmd::RegardeStop { reply } => {
                let res = match regarde.take() {
                    Some(s) => s.arreter(),
                    None => Err("aucune démonstration en cours".into()),
                };
                if let Ok(etapes) = &res {
                    if !etapes.is_empty() {
                        apprendre_demo_en_fond(&llm, etapes.clone());
                    }
                }
                let _ = reply.send(res);
            }
            Cmd::EcranStop { reply } => {
                // Fin du partage = fin d'une demonstration restee ouverte
                // (jamais de hook qui survit a la session) ; elle s'apprend.
                if let Some(s) = regarde.take() {
                    if let Ok(etapes) = s.arreter() {
                        if !etapes.is_empty() {
                            apprendre_demo_en_fond(&llm, etapes);
                        }
                    }
                }
                if let Some(mut child) = voix_ecran.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                // Le micro est rendu : la veille reprend (sauf appel actif).
                if !veille_desactivee() && voix.is_none() && veille.is_none() {
                    match demarrer_veille(appel_port) {
                        Ok(c) => veille = Some(c),
                        Err(e) => eprintln!("veille: {e}"),
                    }
                }
                *ecran_ocr.borrow_mut() = None;
                ecran_actif.store(false, Ordering::SeqCst);
                cible_ecran.store(0, Ordering::SeqCst); // retour « tout l'écran »
                if let Ok(mut s) = scene_ecran.lock() {
                    *s = None;
                }
                let _ = reply.send("partage d'écran arrêté".into());
            }
            Cmd::AgentStart { goal, projet, chan, reply } => {
                match store::create_session(&conn, &title_from(&goal), "agent") {
                    Ok(id) => {
                        current = id;
                        if projet.is_some() {
                            store::set_session_projet(&conn, id, projet).ok();
                        }
                        // Système de MISSION (+ projet) — `truncate(1)` gardait celui
                        // de la conversation précédente (corrigé 2026-09-30).
                        rebuild_window(&mut messages, &conn, current, true);
                        let _ = chan.send(StreamMsg::Session { id });
                        let res = run_streamed_turn(
                            &llm, &registry, &conn, &mut messages, &cancel, &chan, current,
                            &goal, true, None, None, "", None, false,
                        );
                        let _ = reply.send(res);
                    }
                    Err(e) => {
                        let _ = reply.send(Err(format!("session agent impossible: {e}")));
                    }
                }
            }
            Cmd::Approve { id, approve, chan, reply } => {
                // Decision humaine directe : claim atomique + murs re-verifies
                // dans le core, AUCUN LLM dans la boucle de decision.
                let outcome = registry.resolve_one(&conn, id, approve);
                for chemin in fichiers_ecrits(&outcome) {
                    store::add_session_file(&conn, current, &chemin).ok();
                }
                if is_agent(&conn, current) {
                    // La session-agent reprend son travail avec la decision.
                    let decision = if approve {
                        format!(
                            "Ma decision : demande approuvee et executee ({outcome}). \
                             Poursuis ton objectif et conclus."
                        )
                    } else {
                        format!(
                            "Ma decision : demande refusee ({outcome}). Continue sans \
                             cette action et conclus."
                        )
                    };
                    let res = run_streamed_turn(
                        &llm, &registry, &conn, &mut messages, &cancel, &chan, current,
                        &decision, true, None, None, "", None, false,
                    );
                    let _ = reply.send(res);
                } else {
                    // Compagnon : la carte affiche l'issue, pas de tour LLM.
                    let _ = reply.send(Ok(outcome));
                }
            }
        }
    }
    // Fenetre fermee (canal clos) : ne pas laisser la voix parler seule.
    if let Some(mut child) = voix.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(mut child) = voix_ecran.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(mut child) = veille.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(mut child) = voix_seule.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Prévient l'UI hors commande et fait clignoter Waly dans la barre des
/// tâches (rappel échu, mission programmée lancée).
fn signaler(evenement: &str, charge: serde_json::Value) {
    if let Some(app) = APP.get() {
        let _ = app.emit(evenement, charge);
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.request_user_attention(Some(tauri::UserAttentionType::Informational));
        }
    }
}

/// Lance une mission programmée dans SA session (projet compris), avec sa
/// propre fenêtre : la conversation ouverte n'est pas touchée. Aucun front
/// n'écoute le flux ; l'UI est prévenue à la fin (et les demandes
/// d'approbation apparaissent comme pour toute mission).
fn lancer_mission_programmee(
    llm: &LlmClient,
    registry: &Registry,
    conn: &rusqlite::Connection,
    cancel: &AtomicBool,
    m: &store::MissionProgrammee,
) {
    let Ok(id) = store::create_session(conn, &title_from(&m.objectif), "agent") else { return };
    if m.projet_id.is_some() {
        store::set_session_projet(conn, id, m.projet_id).ok();
    }
    store::mission_programmee_lancee(conn, m.id, id).ok();
    let mut fenetre = Vec::new();
    rebuild_window(&mut fenetre, conn, id, true);
    let muet: Channel<StreamMsg> = Channel::new(|_| Ok(()));
    let res = run_streamed_turn(
        llm, registry, conn, &mut fenetre, cancel, &muet, id, &m.objectif, true, None, None, "", None, false,
    );
    signaler(
        "mission-programmee",
        serde_json::json!({"session": id, "objectif": m.objectif, "ok": res.is_ok()}),
    );
}

/// Levé par le cœur quand il a ouvert (donc créé et migré) la base.
static BASE_PRETE: AtomicBool = AtomicBool::new(false);

/// Laisse le cœur ouvrir la base en premier (30 s au plus).
fn attendre_base() {
    for _ in 0..300 {
        if BASE_PRETE.load(Ordering::SeqCst) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Base d'un fil de fond : au tout premier lancement, le cœur est encore en
/// train de la créer (ouverture concurrente refusée) — on réessaie au lieu
/// d'abandonner le fil en silence (vécu 2026-10-01 : sur une base neuve, la
/// relève ne démarrait jamais).
fn ouvrir_base_de_fond() -> rusqlite::Connection {
    attendre_base();
    loop {
        match store::open(&db_path()) {
            Ok(c) => return c,
            Err(e) => {
                eprintln!("Waly: fil de fond: base pas encore prete ({e}), nouvel essai");
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        }
    }
}

/// Dernière erreur de relève du partage (affichée dans Partage et contacts).
static PARTAGE_ERREUR: Mutex<Option<String>> = Mutex::new(None);

/// Fil du partage entre deux Waly (lot 3) : relève la boîte au relais par la
/// passerelle séparée (Waly reste scellé). Une conversation reçue d'un
/// contact attend l'accord de l'utilisateur ; le reste est jeté. Ne tourne
/// que si un relais est réglé.
fn boucle_partage() {
    use waly_core::partage;
    let conn = ouvrir_base_de_fond();
    loop {
        if !partage::actif(&conn) {
            std::thread::sleep(std::time::Duration::from_secs(5));
            continue;
        }
        match partage::relever(&conn, 20) {
            Ok((recus, jetes)) => {
                if let Ok(mut e) = PARTAGE_ERREUR.lock() {
                    *e = None;
                }
                for r in &recus {
                    let _ = waly_core::sceau::noter(
                        &conn,
                        "sortie",
                        &format!("partage : conversation « {} » reçue de {} (en attente de ton accord)", r.titre, r.contact),
                    );
                    signaler("partage-recu", serde_json::json!({"contact": r.contact, "titre": r.titre}));
                }
                if jetes > 0 {
                    let _ = waly_core::sceau::noter(&conn, "sortie", &format!("partage : {jetes} enveloppe(s) jetée(s) — expéditeur inconnu ou contenu falsifié"));
                }
            }
            Err(e) => {
                eprintln!("Waly: partage: {e}");
                if let Ok(mut err) = PARTAGE_ERREUR.lock() {
                    *err = Some(e);
                }
                std::thread::sleep(std::time::Duration::from_secs(15));
            }
        }
    }
}

/// Dernière erreur de relève des passerelles (affichée dans Personnaliser).
static PASSERELLE_ERREUR: Mutex<Option<String>> = Mutex::new(None);

/// Fil des passerelles de messagerie (lot 3) : relève les messages par la
/// passerelle séparée (Waly reste scellé), n'écoute QUE l'interlocuteur
/// appairé, confie le tour au worker (modèle local) et renvoie la réponse.
/// Tout est inscrit au journal du sceau. Vit tant que l'app est ouverte.
fn boucle_passerelles(tx: Sender<Cmd>) {
    use waly_core::passerelles::{self, Decision};
    let conn = ouvrir_base_de_fond();
    let noter = |detail: String| {
        let _ = waly_core::sceau::noter(&conn, "sortie", &detail);
    };
    loop {
        let liste = passerelles::lister(&conn);
        if liste.is_empty() {
            std::thread::sleep(std::time::Duration::from_secs(4));
            continue;
        }
        let attente = if liste.len() == 1 { 20 } else { 5 };
        for p in &liste {
            let recus = match passerelles::relever(&conn, p, attente) {
                Ok(r) => {
                    if let Ok(mut e) = PASSERELLE_ERREUR.lock() {
                        *e = None;
                    }
                    r
                }
                Err(e) => {
                    eprintln!("Waly: passerelle {}: {e}", p.genre);
                    if let Ok(mut err) = PASSERELLE_ERREUR.lock() {
                        *err = Some(e);
                    }
                    std::thread::sleep(std::time::Duration::from_secs(10));
                    continue;
                }
            };
            for r in recus {
                // État relu à chaque message : l'appairage peut changer en cours de lot.
                let Some(p) = passerelles::lister(&conn).into_iter().find(|x| x.id == p.id) else { break };
                match passerelles::decider(&p, &r) {
                    Decision::Appairer => {
                        if passerelles::appairer(&conn, p.id, r.chat_id, &r.prenom).is_ok() {
                            noter(format!("Telegram : téléphone de {} appairé à @{}", r.prenom, p.bot));
                            let _ = passerelles::envoyer(&conn, &p, r.chat_id, "Appairé. Tu parles maintenant à Waly, sur ta machine.");
                            signaler("passerelle", serde_json::json!({}));
                        }
                    }
                    Decision::CodeFaux => {
                        let n = passerelles::code_faux(&conn, p.id);
                        noter(format!("Telegram : code d'appairage faux, ignoré (essai {n}/{})", passerelles::ESSAIS_MAX));
                    }
                    Decision::Ignorer(raison) => noter(format!("Telegram : message ignoré — {raison}")),
                    Decision::Repondre => {
                        let texte: String = r.texte.chars().take(4000).collect();
                        noter(format!("Telegram : message reçu de {} ({} caractères)", r.prenom, texte.chars().count()));
                        let (rtx, rrx) = channel();
                        if tx.send(Cmd::Passerelle { id: p.id, texte, reply: rtx }).is_err() {
                            return; // l'app se ferme
                        }
                        let reponse = match rrx.recv() {
                            Ok(Ok(t)) if !t.trim().is_empty() => t,
                            Ok(Ok(_)) => continue,
                            Ok(Err(e)) => {
                                eprintln!("Waly: passerelle: tour: {e}");
                                "Waly n'a pas pu répondre : son moteur local ne répond pas.".to_string()
                            }
                            Err(_) => return,
                        };
                        match passerelles::envoyer(&conn, &p, r.chat_id, &reponse) {
                            Ok(()) => noter(format!("Telegram : réponse envoyée ({} caractères)", reponse.chars().count())),
                            Err(e) => eprintln!("Waly: passerelle: envoi: {e}"),
                        }
                    }
                }
            }
        }
    }
}

/// Poignée de l'app pour les événements émis HORS commande (worker au repos :
/// rappels échus, missions programmées lancées). Posée au `setup`.
static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// Envoie une commande au worker et attend sa reponse.
fn ask<T>(state: &tauri::State<'_, Core>, make: impl FnOnce(Sender<T>) -> Cmd) -> Option<T> {
    let (tx, rx) = channel();
    state.tx.lock().ok()?.send(make(tx)).ok()?;
    rx.recv().ok()
}

// (async) : execute hors du thread principal Tauri — un tour LLM dure des
// secondes, en sync il gelerait la fenetre pendant la reflexion.
#[tauri::command(async)]
fn core_send(
    state: tauri::State<'_, Core>,
    message: String,
    image: Option<String>,
    fichiers: Option<bool>,
    chan: Channel<StreamMsg>,
) -> Result<String, String> {
    let fichiers = fichiers.unwrap_or(false);
    ask(&state, |tx| Cmd::Send { message, image, fichiers, chan, reply: tx })
        .ok_or_else(|| "cerveau indisponible".to_string())?
}

// --- Lot 1 des « bientôt » (2026-09-30) ------------------------------------

/// Réglages de l'utilisateur lus pour l'UI : instructions, style.
#[tauri::command(async)]
fn core_reglages() -> serde_json::Value {
    let conn = store::open(&db_path()).ok();
    let r = |cle: &str| conn.as_ref().and_then(|c| store::reglage(c, cle));
    serde_json::json!({
        "instructions": r("instructions").unwrap_or_default(),
        "style": r("style").unwrap_or_else(|| "normal".into()),
        "reflexion": r("reflexion").as_deref() == Some("oui"),
    })
}

/// Pose un réglage (seules les clés connues) — via le worker, qui
/// reconstruit la fenêtre pour que le prochain tour en tienne compte.
#[tauri::command(async)]
fn core_reglage_set(state: tauri::State<'_, Core>, cle: String, valeur: String) -> Result<(), String> {
    match cle.as_str() {
        "instructions" if valeur.chars().count() <= 2000 => {}
        "instructions" => return Err("2 000 caractères au plus".into()),
        "style" if ["concis", "normal", "detaille"].contains(&valeur.as_str()) => {}
        "outils" if ["auto", "tous", "selection"].contains(&valeur.as_str()) => {}
        // Lue a chaque tour (consigne dans le message) : ni worker ni rebuild.
        "reflexion" if ["oui", "non"].contains(&valeur.as_str()) => {
            return store::set_reglage(&base()?, &cle, &valeur).map_err(|e| e.to_string());
        }
        _ => return Err(format!("réglage inconnu : {cle}")),
    }
    ask(&state, |tx| Cmd::Reglage { cle, valeur, reply: tx }).ok_or("cerveau indisponible")?
}

/// Sélection d'outils : réglage, mode effectif, taille du modèle.
#[tauri::command]
fn core_outils_etat() -> serde_json::Value {
    OUTILS_ETAT.lock().ok().and_then(|e| e.clone()).unwrap_or(serde_json::Value::Null)
}

/// Exporte la conversation ouverte (md | pdf | docx) dans Téléchargements,
/// puis la montre dans l'Explorateur. Rien ne sort de la machine.
#[tauri::command(async)]
fn core_exporter(state: tauri::State<'_, Core>, format: String, titre: String) -> Result<String, String> {
    if !["md", "pdf", "docx"].contains(&format.as_str()) {
        return Err(format!("format inconnu : {format}"));
    }
    let (_, h) = ask(&state, |tx| Cmd::History { reply: tx }).ok_or("cerveau indisponible")?;
    if h.is_empty() {
        return Err("conversation vide".into());
    }
    let mut md = format!("# {}\n\n", titre.trim());
    for (role, content) in &h {
        // Les tours dictés portent l'en-tête [horodatage | conscience] destiné au modèle.
        let c = if role == "user" {
            content.trim_start().strip_prefix('[').and_then(|r| r.split_once("] ")).map(|(_, t)| t).unwrap_or(content)
        } else {
            content.as_str()
        };
        let qui = if role == "user" { "Toi" } else { "Waly" };
        md.push_str(&format!("## {qui}\n\n{}\n\n", sans_joints(c).trim()));
    }
    let octets = if format == "md" {
        md.into_bytes()
    } else {
        waly_core::documents::generer(&format, &md).ok_or("génération impossible")?
    };
    let home = std::env::var("USERPROFILE").map_err(|_| "dossier personnel introuvable")?;
    let dl = std::path::Path::new(&home).join("Downloads");
    std::fs::create_dir_all(&dl).map_err(|e| e.to_string())?;
    let base: String = titre
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let base = if base.is_empty() { "Conversation Waly".to_string() } else { base.chars().take(60).collect() };
    let mut dest = dl.join(format!("{base}.{format}"));
    let mut k = 2;
    while dest.exists() {
        dest = dl.join(format!("{base} ({k}).{format}"));
        k += 1;
    }
    std::fs::write(&dest, octets).map_err(|e| format!("écriture impossible : {e}"))?;
    let d = dest.to_string_lossy().into_owned();
    let _ = core_ouvrir_fichier(d.clone());
    Ok(d)
}

/// Un fichier joint voyage EN CLAIR dans le message (le modèle le lit) ;
/// à l'export il se résume à son nom, comme à l'écran.
fn sans_joints(s: &str) -> String {
    const DEBUT: &str = "\n\n--- Contenu du fichier joint : ";
    const FIN: &str = "\n--- fin du fichier ---";
    let mut out = String::new();
    let mut reste = s;
    while let Some(i) = reste.find(DEBUT) {
        out.push_str(&reste[..i]);
        let apres = &reste[i + DEBUT.len()..];
        let nom = apres.split(" ---").next().unwrap_or("");
        out.push_str(&format!("\n\n📎 {nom}"));
        reste = match apres.find(FIN) {
            Some(j) => &apres[j + FIN.len()..],
            None => "",
        };
    }
    out.push_str(reste);
    out
}

/// Sélecteur natif pour joindre un fichier au message.
#[tauri::command(async)]
fn core_fichier_choisir() -> Option<String> {
    waly_core::sceau::choisir_fichier(
        "Joindre un fichier à ton message",
        "Documents et images\0*.txt;*.md;*.csv;*.tsv;*.json;*.html;*.docx;*.xlsx;*.pptx;*.pdf;*.png;*.jpg;*.jpeg\0Tous les fichiers\0*.*\0\0",
    )
}

/// Texte joint au message : borne en caractères (le contexte du 4B est
/// court : le système + les outils prennent ~3 000 tokens par tour, mesuré le
/// 2026-09-30 dans Utilisation ; l'alias Ollama tourne à num_ctx 8 192 depuis
/// — 8 000 caractères ≈ 2 300 tokens laissent la place à l'historique).
const JOINT_MAX: usize = 8000;

/// Lit un fichier à joindre : texte (documents, PDF par OCR) ou image
/// réduite pour la vision. Rien n'est copié ni envoyé ailleurs.
#[tauri::command(async)]
fn core_fichier_lire(chemin: String) -> Result<serde_json::Value, String> {
    let p = std::path::PathBuf::from(&chemin);
    let nom = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if ["png", "jpg", "jpeg"].contains(&ext.as_str()) {
        let octets = std::fs::read(&p).map_err(|e| format!("lecture impossible : {e}"))?;
        let shot = waly_sight::pdf::shot_depuis_octets(&octets)?;
        let (jpeg, _, _) = shot.jpeg(1280)?;
        return Ok(serde_json::json!({
            "nom": nom, "genre": "image",
            "data_url": format!("data:image/jpeg;base64,{}", waly_core::native_tools::base64(&jpeg)),
        }));
    }
    let (texte, note) = if ext == "pdf" {
        let (t, lues, total) = waly_sight::pdf::texte_ocr(&p, 5)?;
        let note = if total > lues { format!("{lues} pages lues sur {total}") } else { String::new() };
        (t, note)
    } else {
        (waly_core::apercu::texte(&p)?, String::new())
    };
    let texte = texte.trim().to_string();
    if texte.is_empty() {
        return Err("aucun texte lisible dans ce fichier".into());
    }
    let tronque = texte.chars().count() > JOINT_MAX;
    let texte: String = texte.chars().take(JOINT_MAX).collect();
    Ok(serde_json::json!({ "nom": nom, "genre": "texte", "texte": texte, "tronque": tronque, "note": note }))
}

// --- Missions programmées (lot 2) ------------------------------------------

#[tauri::command(async)]
fn core_missions_programmees() -> Result<Vec<store::MissionProgrammee>, String> {
    store::list_missions_programmees(&base()?).map_err(|e| e.to_string())
}

/// `quand` = heure LOCALE « AAAA-MM-JJ HH:MM » (champ date de l'UI).
#[tauri::command(async)]
fn core_mission_programmer(
    objectif: String,
    quand: String,
    recurrence: Option<String>,
    projet: Option<i64>,
) -> Result<i64, String> {
    if objectif.trim().is_empty() {
        return Err("donne un objectif à la mission".into());
    }
    let q = quand.trim().replace('T', " ");
    let ok = q.len() == 16
        && q.chars().enumerate().all(|(i, c)| match i {
            4 | 7 => c == '-',
            10 => c == ' ',
            13 => c == ':',
            _ => c.is_ascii_digit(),
        });
    if !ok {
        return Err("date attendue : AAAA-MM-JJ HH:MM".into());
    }
    let rec = recurrence.filter(|r| !r.is_empty());
    if let Some(r) = &rec {
        if !["quotidien", "hebdomadaire", "ouvres"].contains(&r.as_str()) {
            return Err(format!("récurrence inconnue : {r}"));
        }
    }
    store::create_mission_programmee(&base()?, &objectif, &format!("{q}:00"), rec.as_deref(), projet)
        .map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn core_mission_programmee_annuler(id: i64) -> Result<bool, String> {
    store::cancel_mission_programmee(&base()?, id).map_err(|e| e.to_string())
}

// --- Projets (lot 2, 2026-09-30) -------------------------------------------

fn base() -> Result<rusqlite::Connection, String> {
    store::open(&db_path()).map_err(|e| e.to_string())
}

/// Après tout changement qui touche le système STABLE (projet de la session,
/// instructions du projet) : le prochain tour doit le voir.
fn rebuild(state: &tauri::State<'_, Core>) {
    let _ = ask(state, |tx| Cmd::Rebuild { reply: tx });
}

#[tauri::command(async)]
fn core_projets() -> Result<Vec<store::Projet>, String> {
    store::list_projets(&base()?).map_err(|e| e.to_string())
}

/// Un projet et ses conversations/missions.
#[tauri::command(async)]
fn core_projet(id: i64) -> Result<serde_json::Value, String> {
    let conn = base()?;
    let p = store::projet(&conn, id).map_err(|e| e.to_string())?.ok_or("projet introuvable")?;
    let sessions = store::sessions_du_projet(&conn, id).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "projet": p, "sessions": sessions }))
}

/// Le projet d'une conversation (barre de titre, menu +), s'il y en a un.
#[tauri::command(async)]
fn core_projet_de_session(session: i64) -> Option<store::Projet> {
    let conn = base().ok()?;
    let id = store::session_projet(&conn, session)?;
    store::projet(&conn, id).ok().flatten()
}

#[tauri::command(async)]
fn core_projet_creer(nom: String) -> Result<i64, String> {
    if nom.trim().is_empty() {
        return Err("donne un nom au projet".into());
    }
    store::create_projet(&base()?, &nom).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn core_projet_maj(state: tauri::State<'_, Core>, id: i64, nom: String, instructions: String, archive: bool) -> Result<(), String> {
    if nom.trim().is_empty() {
        return Err("le nom ne peut pas être vide".into());
    }
    if instructions.chars().count() > 4000 {
        return Err("4 000 caractères au plus pour les instructions".into());
    }
    store::update_projet(&base()?, id, &nom, &instructions, archive).map_err(|e| e.to_string())?;
    rebuild(&state);
    Ok(())
}

#[tauri::command(async)]
fn core_projet_supprimer(state: tauri::State<'_, Core>, id: i64) -> Result<(), String> {
    store::delete_projet(&base()?, id).map_err(|e| e.to_string())?;
    rebuild(&state);
    Ok(())
}

/// Range une conversation dans un projet (ou l'en sort : `projet` absent).
#[tauri::command(async)]
fn core_session_projet(state: tauri::State<'_, Core>, session: i64, projet: Option<i64>) -> Result<(), String> {
    store::set_session_projet(&base()?, session, projet).map_err(|e| e.to_string())?;
    rebuild(&state);
    Ok(())
}

/// Ajoute un fichier de référence (sélecteur natif). Texte et Office sont
/// lus à la demande par `lire_fichier` ; un PDF est lu UNE fois par l'OCR
/// local et son texte rangé dans Documents\Waly\Projets\<projet>\ (le
/// modèle ne sait pas lire un PDF lui-même). Renvoie le chemin retenu.
#[tauri::command(async)]
fn core_projet_fichier_ajouter(state: tauri::State<'_, Core>, id: i64) -> Result<Option<String>, String> {
    let Some(chemin) = waly_core::sceau::choisir_fichier(
        "Ajouter un fichier de référence au projet",
        "Documents\0*.txt;*.md;*.csv;*.tsv;*.json;*.html;*.docx;*.xlsx;*.pptx;*.pdf\0Tous les fichiers\0*.*\0\0",
    ) else {
        return Ok(None);
    };
    let conn = base()?;
    let p = std::path::PathBuf::from(&chemin);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let retenu = if ext == "pdf" {
        let projet = store::projet(&conn, id).map_err(|e| e.to_string())?.ok_or("projet introuvable")?;
        let (texte, lues, total) = waly_sight::pdf::texte_ocr(&p, 30)?;
        let propre = |s: &str| -> String {
            s.chars().map(|c| if c.is_alphanumeric() || " -_.()".contains(c) { c } else { '_' }).collect()
        };
        let dossier = waly_core::fichiers::racine_documents().join("Projets").join(propre(&projet.nom));
        std::fs::create_dir_all(&dossier).map_err(|e| e.to_string())?;
        let nom = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dest = dossier.join(format!("{}.txt", propre(&nom)));
        let entete = format!(
            "Texte extrait de {} ({} pages lues sur {}, reconnaissance locale : accents possiblement perdus)\n\n",
            chemin, lues, total
        );
        std::fs::write(&dest, entete + &texte).map_err(|e| e.to_string())?;
        dest.to_string_lossy().into_owned()
    } else {
        chemin
    };
    store::add_projet_fichier(&conn, id, &retenu).map_err(|e| e.to_string())?;
    rebuild(&state);
    Ok(Some(retenu))
}

#[tauri::command(async)]
fn core_projet_fichier_retirer(state: tauri::State<'_, Core>, id: i64, chemin: String) -> Result<(), String> {
    store::remove_projet_fichier(&base()?, id, &chemin).map_err(|e| e.to_string())?;
    rebuild(&state);
    Ok(())
}

/// Paramètres › Utilisation : agrégats 30 jours par modèle + derniers tours.
#[tauri::command(async)]
fn core_utilisation() -> Result<serde_json::Value, String> {
    let conn = store::open(&db_path()).map_err(|e| e.to_string())?;
    let par_modele: Vec<_> = store::utilisation_par_modele(&conn, 30)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(m, n, p, c, d, f, e)| {
            serde_json::json!({"modele": m, "tours": n, "entree": p, "sortie": c, "duree_ms": d, "premier_ms": f, "estime": e})
        })
        .collect();
    let derniers: Vec<_> = store::derniers_tours(&conn, 12)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(at, t)| serde_json::json!({"at": at, "tour": t}))
        .collect();
    Ok(serde_json::json!({ "par_modele": par_modele, "derniers": derniers }))
}

/// Interrompt le tour en cours (le worker le voit a la prochaine sonde).
/// Sync et instantane : un simple store atomique.
#[tauri::command]
fn core_stop(state: tauri::State<'_, Core>) {
    state.cancel.store(true, Ordering::SeqCst);
}

// Toutes (async) : le worker est seriel, une requete derriere un Send
// attendrait la fin du tour — jamais sur le thread principal.
#[tauri::command(async)]
fn core_history(state: tauri::State<'_, Core>) -> (i64, Vec<(String, String)>) {
    ask(&state, |tx| Cmd::History { reply: tx }).unwrap_or((store::MAIN_SESSION, vec![]))
}

#[tauri::command(async)]
fn core_sessions(state: tauri::State<'_, Core>, kind: String) -> Vec<store::SessionRow> {
    ask(&state, |tx| Cmd::Sessions { kind, reply: tx }).unwrap_or_default()
}

#[tauri::command(async)]
fn core_new_session(state: tauri::State<'_, Core>) -> i64 {
    ask(&state, |tx| Cmd::NewSession { reply: tx }).unwrap_or(store::MAIN_SESSION)
}

#[tauri::command(async)]
fn core_select_session(state: tauri::State<'_, Core>, id: i64) -> Vec<(String, String)> {
    ask(&state, |tx| Cmd::SelectSession { id, reply: tx }).unwrap_or_default()
}

/// Supprime une conversation ou une mission (UI 2026-09-14).
#[tauri::command(async)]
fn core_session_supprimer(state: tauri::State<'_, Core>, id: i64) -> Result<i64, String> {
    ask(&state, |tx| Cmd::DeleteSession { id, reply: tx })
        .ok_or_else(|| "cerveau indisponible".to_string())?
}

/// Renomme une conversation ou une mission.
#[tauri::command(async)]
fn core_session_renommer(id: i64, titre: String) -> Result<(), String> {
    let t: String = titre.trim().chars().take(80).collect();
    if t.is_empty() {
        return Err("titre vide".into());
    }
    let conn = store::open(&db_path()).map_err(|e| e.to_string())?;
    store::set_session_title(&conn, id, &t).map_err(|e| e.to_string())
}

/// Aperçu DANS l'app d'un fichier créé (UI 2026-09-14) : texte, markdown,
/// page HTML, tableau, document, présentation, image, PDF (pages rendues par
/// Windows). Dans TOUTES les conversations, où que le fichier ait été créé
/// (Documents\Waly ou dossier pointé par une mission). Lecture seule, bornée.
#[tauri::command(async)]
fn core_apercu(chemin: String) -> Result<serde_json::Value, String> {
    let p = std::fs::canonicalize(&chemin).map_err(|_| "fichier introuvable".to_string())?;
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if ext == "pdf" {
        let (pages, total) = waly_sight::pdf::pages_png(&p, 20, 1000)?;
        return Ok(serde_json::json!({
            "type": "pdf",
            "total": total,
            "pages": pages
                .iter()
                .map(|b| format!("data:image/png;base64,{}", waly_core::native_tools::base64(b)))
                .collect::<Vec<_>>(),
        }));
    }
    waly_core::apercu::apercu(&p)
}

/// Cartes de fichiers d'une conversation : {total, fichiers:[{chemin, position}]}.
#[tauri::command(async)]
fn core_fichiers_session(id: i64) -> serde_json::Value {
    let Ok(conn) = store::open(&db_path()) else {
        return serde_json::json!({"total": 0, "fichiers": []});
    };
    let (total, f) = store::list_session_files(&conn, id).unwrap_or((0, Vec::new()));
    let reflexions = store::list_session_reflexions(&conn, id).unwrap_or_default();
    serde_json::json!({
        "total": total,
        "reflexions": reflexions
            .into_iter()
            .map(|(p, t)| serde_json::json!({"position": p, "texte": t}))
            .collect::<Vec<_>>(),
        "fichiers": f
            .into_iter()
            .filter(|(c, _)| std::path::Path::new(c).is_file())
            .map(|(c, p)| serde_json::json!({"chemin": c, "position": p}))
            .collect::<Vec<_>>(),
    })
}

/// « Télécharger » (comme Claude) : copie le fichier dans le dossier
/// Téléchargements (nom libre « x (2).ext » si besoin) et l'y montre.
#[tauri::command(async)]
fn core_telecharger(chemin: String) -> Result<String, String> {
    let src = std::path::PathBuf::from(&chemin);
    if !src.is_file() {
        return Err("fichier introuvable".into());
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "dossier personnel introuvable".to_string())?;
    let dl = std::path::Path::new(&home).join("Downloads");
    std::fs::create_dir_all(&dl).map_err(|e| e.to_string())?;
    let nom = src.file_name().ok_or("nom de fichier invalide")?.to_string_lossy().into_owned();
    let (base, ext) = match nom.rfind('.') {
        Some(i) if i > 0 => (nom[..i].to_string(), nom[i..].to_string()),
        _ => (nom.clone(), String::new()),
    };
    let mut dest = dl.join(&nom);
    let mut k = 2;
    while dest.exists() {
        dest = dl.join(format!("{base} ({k}){ext}"));
        k += 1;
    }
    std::fs::copy(&src, &dest).map_err(|e| format!("copie impossible : {e}"))?;
    let d = dest.to_string_lossy().into_owned();
    let _ = core_ouvrir_fichier(d.clone());
    Ok(d)
}

#[tauri::command(async)]
fn core_search(state: tauri::State<'_, Core>, query: String, kind: String) -> Vec<store::SessionRow> {
    ask(&state, |tx| Cmd::Search { query, kind, reply: tx }).unwrap_or_default()
}

#[tauri::command(async)]
fn core_skills(state: tauri::State<'_, Core>) -> Vec<(String, String)> {
    ask(&state, |tx| Cmd::Skills { reply: tx }).unwrap_or_default()
}

#[tauri::command(async)]
fn core_artifacts(state: tauri::State<'_, Core>) -> serde_json::Value {
    ask(&state, |tx| Cmd::Artifacts { reply: tx }).unwrap_or(serde_json::Value::Null)
}

#[tauri::command(async)]
fn core_agent_start(
    state: tauri::State<'_, Core>,
    goal: String,
    projet: Option<i64>,
    chan: Channel<StreamMsg>,
) -> Result<String, String> {
    ask(&state, |tx| Cmd::AgentStart { goal, projet, chan, reply: tx })
        .ok_or_else(|| "cerveau indisponible".to_string())?
}

#[tauri::command(async)]
fn core_pendings(state: tauri::State<'_, Core>) -> Vec<(i64, String, String)> {
    ask(&state, |tx| Cmd::Pendings { reply: tx }).unwrap_or_default()
}

#[tauri::command(async)]
fn core_approve(
    state: tauri::State<'_, Core>,
    id: i64,
    approve: bool,
    chan: Channel<StreamMsg>,
) -> Result<String, String> {
    ask(&state, |tx| Cmd::Approve { id, approve, chan, reply: tx })
        .ok_or_else(|| "cerveau indisponible".to_string())?
}

#[tauri::command(async)]
fn core_appel_start(
    state: tauri::State<'_, Core>,
    chan: Channel<waly_sight::perception::Event>,
) -> Result<String, String> {
    garde_passer(&[(Ressource::Camera, "a ouvert la caméra pour un appel"), (Ressource::Micro, "a écouté pendant un appel")])?;
    let (msg, cliche) = ask(&state, |tx| Cmd::AppelStart { chan, reply: tx })
        .ok_or_else(|| "cerveau indisponible".to_string())??;
    if let Ok(mut c) = state.cliche.lock() {
        *c = Some(cliche);
    }
    Ok(msg)
}

#[tauri::command(async)]
fn core_appel_stop(state: tauri::State<'_, Core>) -> String {
    if let Ok(mut c) = state.cliche.lock() {
        *c = None;
    }
    ask(&state, |tx| Cmd::AppelStop { reply: tx }).unwrap_or_default()
}

/// Aperçu auto-vue de l'écran d'appel : cliché 320 px en data-URL. Lu HORS
/// du worker (voir `Core::cliche`) — l'aperçu reste fluide pendant un tour.
#[tauri::command(async)]
fn core_appel_cliche(state: tauri::State<'_, Core>) -> Result<String, String> {
    let guard = state.cliche.lock().map_err(|_| "verrou aperçu")?;
    let cliche = guard.as_ref().ok_or("appel inactif")?;
    let (jpeg, _, _) = cliche.jpeg(320)?;
    Ok(format!("data:image/jpeg;base64,{}", waly_core::native_tools::base64(&jpeg)))
}

/// Dictee (UI 2026-09-14, le micro de la saisie comme chez Claude) : allume le
/// micro via `waly-voice dicter` et attend qu'il capte. Transcription LOCALE
/// (Parakeet) — jamais la reconnaissance vocale du navigateur, qui part en ligne.
#[tauri::command(async)]
fn core_dictee_start(state: tauri::State<'_, Core>) -> Result<(), String> {
    if state.dictee.lock().map(|d| d.is_some()).unwrap_or(false) {
        return Ok(());
    }
    garde_passer(&[(Ressource::Micro, "a écouté une dictée")])?;
    let exe = std::env::var("WALY_VOICE_EXE")
        .unwrap_or_else(|_| waly_core::chemins::exe("waly-voice"));
    if !std::path::Path::new(&exe).exists() {
        return Err(format!("{exe} introuvable"));
    }
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("dicter")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| format!("spawn {exe}: {e}"))?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take().ok_or("sortie de la dictée indisponible")?;
    let etat = Arc::new(Mutex::new(DicteeEtat::default()));
    {
        let etat = etat.clone();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for l in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                let Ok(mut e) = etat.lock() else { return };
                if l == "ECOUTE" {
                    e.ecoute = true;
                } else if let Some(j) = l.strip_prefix("TEXTE ") {
                    e.texte = Some(serde_json::from_str(j).unwrap_or_default());
                } else if let Some(j) = l.strip_prefix("ERREUR ") {
                    e.erreur = Some(serde_json::from_str(j).unwrap_or_else(|_| j.to_string()));
                }
            }
            if let Ok(mut e) = etat.lock() {
                e.fini = true;
            }
        });
    }
    let debut = std::time::Instant::now();
    loop {
        {
            let e = etat.lock().map_err(|_| "verrou dictée")?;
            if e.ecoute {
                break;
            }
            if let Some(err) = e.erreur.clone() {
                drop(e);
                let _ = child.kill();
                let _ = child.wait();
                return Err(err);
            }
            if e.fini {
                drop(e);
                let _ = child.wait();
                return Err("la dictée s'est arrêtée au démarrage".into());
            }
        }
        if debut.elapsed() > std::time::Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("le micro ne s'est pas ouvert".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    *state.dictee.lock().map_err(|_| "verrou dictée")? = Some(Dictee { child, stdin, etat });
    Ok(())
}

/// Termine la dictee : le micro s'eteint, Parakeet transcrit, le texte revient.
#[tauri::command(async)]
fn core_dictee_stop(state: tauri::State<'_, Core>) -> Result<String, String> {
    let mut d = state
        .dictee
        .lock()
        .map_err(|_| "verrou dictée")?
        .take()
        .ok_or("aucune dictée en cours")?;
    if let Some(mut s) = d.stdin.take() {
        use std::io::Write;
        let _ = s.write_all(b"stop\n");
        let _ = s.flush();
    }
    let debut = std::time::Instant::now();
    let res = loop {
        {
            let e = d.etat.lock().map_err(|_| "verrou dictée")?;
            if let Some(t) = &e.texte {
                break Ok(t.clone());
            }
            if let Some(err) = &e.erreur {
                break Err(err.clone());
            }
            if e.fini {
                break Err("la dictée s'est arrêtée sans texte".to_string());
            }
        }
        if debut.elapsed() > std::time::Duration::from_secs(120) {
            break Err("transcription trop longue".to_string());
        }
        std::thread::sleep(std::time::Duration::from_millis(80));
    };
    let _ = d.child.kill();
    let _ = d.child.wait();
    res
}

/// Annule la dictee : micro eteint, rien n'est transcrit.
#[tauri::command(async)]
fn core_dictee_annuler(state: tauri::State<'_, Core>) {
    let d = state.dictee.lock().ok().and_then(|mut g| g.take());
    if let Some(mut d) = d {
        let _ = d.child.kill();
        let _ = d.child.wait();
    }
}

/// Mode VOIX (R6b) : conversation vocale sans camera — la page d'appel sert
/// d'ecran (selfview cachee), la voix ecrit dans la conversation ouverte.
#[tauri::command(async)]
fn core_voix_start(state: tauri::State<'_, Core>) -> Result<String, String> {
    garde_passer(&[(Ressource::Micro, "a écouté pendant une conversation à voix haute")])?;
    ask(&state, |tx| Cmd::VoixStart { reply: tx })
        .ok_or_else(|| "cerveau indisponible".to_string())?
}

#[tauri::command(async)]
fn core_voix_stop(state: tauri::State<'_, Core>) -> String {
    ask(&state, |tx| Cmd::VoixStop { reply: tx }).unwrap_or_default()
}

/// R6b : millisecondes depuis le dernier « Waly » entendu par la veille
/// (-1 si jamais). L'UI sonde et joue le « Souffle » (verdict gravé 21/07)
/// sur la marque d'identité.
#[tauri::command]
fn core_eveil(state: tauri::State<'_, Core>) -> i64 {
    match state.eveil.lock().ok().and_then(|e| *e) {
        Some(t) => t.elapsed().as_millis() as i64,
        None => -1,
    }
}

/// R6b : ms depuis la derniere ANNULATION d'eveil (confirmation STT
/// negative) — l'UI referme la page voix auto-ouverte. -1 si jamais.
#[tauri::command]
fn core_eveil_annule(state: tauri::State<'_, Core>) -> i64 {
    match state.eveil_annule.lock().ok().and_then(|e| *e) {
        Some(t) => t.elapsed().as_millis() as i64,
        None => -1,
    }
}

/// Pouls de la voix pour l'eclipse (ch. 4) : etat + niveau, lu HORS worker
/// a chaque frame d'animation. Sans POST recent (voix morte, hors appel),
/// retombe au repos.
#[tauri::command(async)]
fn core_appel_pouls(state: tauri::State<'_, Core>) -> serde_json::Value {
    let p = state.pouls.lock().ok();
    match p {
        Some(p) if p.maj.elapsed().as_millis() < 1500 => {
            serde_json::json!({ "etat": p.etat, "niveau": p.niveau })
        }
        _ => serde_json::json!({ "etat": "repos", "niveau": 0.0 }),
    }
}

/// Ouvre la session de partage d'écran (R5) : démarre l'OCR + la voix
/// compagnon (worker), PUIS ouvre la présence (cadre) et le pop-up flottant.
#[tauri::command(async)]
fn core_ecran_start(app: tauri::AppHandle, state: tauri::State<'_, Core>) -> Result<String, String> {
    garde_passer(&[(Ressource::Ecran, "a regardé l'écran pendant un partage"), (Ressource::Micro, "a écouté pendant un partage d'écran")])?;
    let etat = ask(&state, |tx| Cmd::EcranStart { reply: tx })
        .ok_or_else(|| "cerveau indisponible".to_string())??;
    ouvrir_fenetres_ecran(&app)?;
    Ok(etat)
}

/// Ferme la session : referme la présence + le pop-up, arrête OCR + voix.
#[tauri::command(async)]
fn core_ecran_stop(app: tauri::AppHandle, state: tauri::State<'_, Core>) -> String {
    fermer_fenetres_ecran(&app);
    // La fenêtre principale éteint sa puce « Écran partagé » même quand
    // l'arrêt vient du pop-up (Stop).
    let _ = app.emit("ecran-arrete", ());
    // Reactiver le micro pour la prochaine session (etat propre).
    state.mic_muted.store(false, Ordering::SeqCst);
    ask(&state, |tx| Cmd::EcranStop { reply: tx }).unwrap_or_default()
}

/// Coupe / réactive le micro (R5) : la voix lit cet état sur /mic et jette
/// l'audio quand c'est coupé (Waly ne respecte pas le mute système).
#[tauri::command]
fn core_ecran_mute(state: tauri::State<'_, Core>, muted: bool) {
    state.mic_muted.store(muted, Ordering::SeqCst);
}

/// B3 « Regarde-moi » : ouvre la démonstration (bouton ◉ du pop-up).
#[tauri::command(async)]
fn core_regarde_start(state: tauri::State<'_, Core>) -> Result<String, String> {
    ask(&state, |tx| Cmd::RegardeStart { reply: tx })
        .ok_or_else(|| "cerveau indisponible".to_string())?
}

/// Ferme la démonstration : les étapes vues (texte), apprises en fond.
#[tauri::command(async)]
fn core_regarde_stop(state: tauri::State<'_, Core>) -> Result<Vec<String>, String> {
    ask(&state, |tx| Cmd::RegardeStop { reply: tx })
        .ok_or_else(|| "cerveau indisponible".to_string())?
}

/// Cible de capture (R5, sélecteur du pop-up) : 0 = tout l'écran, sinon le
/// handle d'une fenêtre. Toutes les captures (Regarder, voix, /cliche-ecran)
/// l'utilisent.
#[tauri::command]
fn core_ecran_cible(state: tauri::State<'_, Core>, hwnd: i64) {
    state.cible_ecran.store(hwnd, Ordering::SeqCst);
}

/// Liste des fenêtres ouvertes (handle, titre) pour le sélecteur « quelle
/// fenêtre regarder ? ». Énumération native, hors worker.
#[tauri::command(async)]
fn core_ecran_fenetres() -> Vec<(i64, String)> {
    waly_sight::screen::list_windows()
        .into_iter()
        .map(|(h, t)| (h as i64, t))
        .collect()
}

// --- Huis clos (R6a) : le sceau reseau par session --------------------------
//
// Les commandes parlent DIRECTEMENT au service scelleur (pipe) + ecrivent le
// journal d'audit dans la base partagee (conn courte, WAL busy_timeout). Hors
// worker : sceller/journal ne doivent pas attendre un tour LLM.

/// Etat du sceau pour l'UI : le service repond-il, a-t-il le privilege SYSTEM,
/// et quelles sessions sont scellees. Le bouton « Sceller » s'y adapte.
///
/// `prouve` : le resultat d'un ESSAI reel depuis ce processus (`sceau::sonder`,
/// vers une adresse de documentation). Le service peut repondre « scelle »
/// sans qu'aucun filtre ne vise ce programme : l'interface n'affiche « tenu »
/// que si l'essai n'a pas montre le contraire. Si l'essai passe, on se
/// redeclare au service une fois et on reessaie (course au demarrage).
#[tauri::command(async)]
fn core_sceau_etat() -> serde_json::Value {
    use waly_core::sceau::{self, Sonde};
    match sceau::etat() {
        Ok((sessions, version, privilegie)) => {
            let mut prouve = sceau::sonder();
            if prouve == Sonde::Ouverte && privilegie {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = sceau::rejoindre(&exe.to_string_lossy(), None);
                }
                prouve = sceau::sonder_maintenant();
            }
            serde_json::json!({
                "disponible": true, "privilegie": privilegie,
                "version": version, "sessions": sessions, "prouve": prouve,
                "age": sceau::age_sonde(),
            })
        }
        Err(_) => serde_json::json!({
            "disponible": false, "privilegie": false,
            "version": "", "sessions": [], "prouve": Sonde::Indeterminee,
        }),
    }
}

/// Nom du modèle actif pour la barre d'état (WALY_MODEL ou défaut) — l'UI ne
/// doit jamais afficher un nom de modèle codé en dur.
#[tauri::command]
fn core_modele() -> String {
    waly_core::llm::modele_par_defaut()
}

/// Panneau « Ta machine et ton modèle » (point 3 open source) : profil
/// matériel, recommandation, modèles installés (Ollama), configuration
/// actuelle. Hors worker (détection + requêtes courtes).
#[tauri::command(async)]
fn core_materiel() -> serde_json::Value {
    let profil = waly_core::materiel::detecter();
    let ollama = LlmClient::new("127.0.0.1", waly_core::llm::PORT_MOTEUR_B, "");
    let installes = ollama.modeles_installes();
    let reco = waly_core::materiel::recommander(&profil, &installes);
    serde_json::json!({
        "profil": profil,
        "reco": reco,
        "installes": installes,
        // Premier lancement = pas encore de waly.toml : « Appliquer » l'écrit.
        "premier": !std::path::Path::new(&waly_core::config::chemin()).exists(),
        "actuel": {
            "modele": waly_core::llm::modele_par_defaut(),
            "port": waly_core::llm::port_par_defaut(),
            "vision": waly_core::vision::etat(),
        },
    })
}

/// Écrit la recommandation dans waly.toml — SEULEMENT s'il n'existe pas
/// (jamais d'écrasement d'une config de l'utilisateur ; sinon il copie le
/// bloc à la main). Aucun réseau : les modèles se tirent par `ollama pull`.
#[tauri::command(async)]
fn core_materiel_appliquer() -> Result<String, String> {
    let chemin = waly_core::config::chemin();
    let p = std::path::Path::new(&chemin);
    if p.exists() {
        return Err("waly.toml existe déjà — copie le bloc proposé à la main".into());
    }
    let profil = waly_core::materiel::detecter();
    let installes =
        LlmClient::new("127.0.0.1", waly_core::llm::PORT_MOTEUR_B, "").modeles_installes();
    let reco = waly_core::materiel::recommander(&profil, &installes);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let contenu = format!(
        "# waly.toml — ecrit par Waly depuis la recommandation materiel.\n\
         # Modele : C:\\waly\\waly.toml.example. Relancer Waly pour appliquer.\n\n{}",
        reco.toml
    );
    std::fs::write(p, contenu).map_err(|e| e.to_string())?;
    Ok(format!("Configuration enregistrée dans {chemin} — relance Waly pour l'appliquer."))
}

// --- Lot 3 : cerveau local en route, modèles extérieurs, routeur ------------

/// Ce que le menu des modèles montre : cerveau local (et ceux installés),
/// modèles extérieurs déclarés (jamais leur clé), mode en cours.
#[tauri::command(async)]
fn core_cerveau() -> serde_json::Value {
    let conn = store::open(&db_path()).ok();
    let port = waly_core::llm::port_par_defaut();
    let locaux: Vec<serde_json::Value> = LlmClient::new("127.0.0.1", port, "")
        .modeles_detail()
        .into_iter()
        .filter(|m| !m.capacites.iter().any(|c| c == "embedding"))
        .map(|m| serde_json::json!({
            "nom": m.nom,
            "taille_go": m.taille_go,
            // Un cerveau a besoin des outils ; capacités inconnues = on laisse essayer.
            "outils": m.capacites.is_empty() || m.capacites.iter().any(|c| c == "tools"),
        }))
        .collect();
    let exterieurs = conn.as_ref().map(waly_core::exterieur::lister).unwrap_or_default();
    let r = |cle: &str| conn.as_ref().and_then(|c| store::reglage(c, cle));
    let choisi = r("exterieur_id").and_then(|i| i.parse::<i64>().ok());
    let valide = choisi.is_some_and(|i| exterieurs.iter().any(|e| e.id == i));
    serde_json::json!({
        "local": waly_core::llm::modele_par_defaut(),
        "port": port,
        "locaux": locaux,
        "mode": if valide { r("cerveau").unwrap_or_else(|| "local".into()) } else { "local".into() },
        "exterieur_id": choisi.filter(|_| valide),
        "exterieurs": exterieurs.iter().map(|e| serde_json::json!({
            "id": e.id, "fournisseur": e.fournisseur, "modele": e.modele, "hote": e.hote(),
            "nom": waly_core::exterieur::fournisseur(&e.fournisseur).map(|f| f.nom).unwrap_or("?"),
        })).collect::<Vec<_>>(),
        "fournisseurs": waly_core::exterieur::FOURNISSEURS,
    })
}

/// Choisit qui répond : `mode` = local | exterieur | auto. `local` = nom d'un
/// modèle installé (change le cerveau local en route) ; `exterieur` = id d'un
/// modèle extérieur déclaré.
#[tauri::command(async)]
fn core_cerveau_choisir(
    state: tauri::State<'_, Core>,
    mode: String,
    local: Option<String>,
    exterieur: Option<i64>,
) -> Result<(), String> {
    if !["local", "exterieur", "auto"].contains(&mode.as_str()) {
        return Err(format!("mode inconnu : {mode}"));
    }
    let conn = base()?;
    if mode != "local" {
        let id = exterieur.ok_or("aucun modèle extérieur choisi")?;
        waly_core::exterieur::trouver(&conn, id).ok_or("modèle extérieur introuvable")?;
        store::set_reglage(&conn, "exterieur_id", &id.to_string()).map_err(|e| e.to_string())?;
    }
    store::set_reglage(&conn, "cerveau", &mode).map_err(|e| e.to_string())?;
    if let Some(nom) = local.filter(|n| *n != waly_core::llm::modele_par_defaut()) {
        if !waly_core::modeles::nom_valide(&nom) {
            return Err("nom de modèle invalide".into());
        }
        if waly_core::llm::port_par_defaut() != waly_core::llm::PORT_MOTEUR_B {
            return Err("ce moteur sert un seul modèle : change-le dans waly.toml".into());
        }
        ask(&state, |tx| Cmd::Modele { nom, reply: tx }).ok_or("cerveau indisponible")??;
    }
    Ok(())
}

/// Déclare un modèle extérieur. La clé est chiffrée par Windows avant d'entrer
/// en base ; la déclaration est inscrite au journal du sceau.
#[tauri::command(async)]
fn core_exterieur_ajouter(fournisseur: String, base_url: String, modele: String, cle: String) -> Result<i64, String> {
    let conn = base()?;
    let id = waly_core::exterieur::ajouter(&conn, &fournisseur, &base_url, &modele, &cle)?;
    if let Some(c) = waly_core::exterieur::trouver(&conn, id) {
        let _ = waly_core::sceau::noter(
            &conn,
            "sortie",
            &format!("modèle extérieur déclaré : {} vers {} (rien n'est encore sorti)", c.modele, c.hote()),
        );
    }
    Ok(id)
}

/// Retire un modèle extérieur et sa clé ; s'il était en service, retour au local.
#[tauri::command(async)]
fn core_exterieur_retirer(id: i64) -> Result<(), String> {
    let conn = base()?;
    let c = waly_core::exterieur::trouver(&conn, id);
    waly_core::exterieur::retirer(&conn, id)?;
    if store::reglage(&conn, "exterieur_id").as_deref() == Some(id.to_string().as_str()) {
        store::set_reglage(&conn, "cerveau", "local").map_err(|e| e.to_string())?;
    }
    if let Some(c) = c {
        let _ = waly_core::sceau::noter(&conn, "sortie", &format!("modèle extérieur retiré : {} ({}) — clé effacée", c.modele, c.hote()));
    }
    Ok(())
}

/// Vérifie un modèle extérieur par un message minimal (« Réponds : ok ») —
/// une vraie sortie, inscrite au journal, facturée quelques tokens.
#[tauri::command(async)]
fn core_exterieur_tester(id: i64) -> Result<String, String> {
    let conn = base()?;
    let cfg = waly_core::exterieur::trouver(&conn, id).ok_or("modèle extérieur introuvable")?;
    let _ = waly_core::sceau::noter(&conn, "sortie", &format!("{} via {} — message de test", cfg.modele, cfg.hote()));
    let msgs = vec![("user".to_string(), "Réponds seulement : ok".to_string())];
    let r = waly_core::exterieur::converser(&conn, &cfg, "Réponds en un mot.", &msgs, |_| true)?;
    if r.refus {
        return Err("le fournisseur a décliné le message de test".into());
    }
    Ok(r.texte.trim().chars().take(120).collect())
}

// --- Lot 3 : partage entre deux Waly ------------------------------------------

/// État du partage : relais, nom, code à donner (si un relais est réglé),
/// contacts, conversations reçues en attente. Ne crée PAS d'identité.
#[tauri::command(async)]
fn core_partage() -> serde_json::Value {
    use waly_core::partage;
    let Ok(conn) = base() else { return serde_json::Value::Null };
    let actif = partage::actif(&conn);
    let (relais, nom) = if actif {
        partage::identite(&conn).map(|i| (i.relais, i.nom)).unwrap_or_default()
    } else {
        Default::default()
    };
    serde_json::json!({
        "relais": relais,
        "nom": nom,
        "code": if actif { partage::mon_code(&conn).ok() } else { None },
        "contacts": partage::contacts(&conn),
        "recus": partage::recus(&conn),
        "erreur": PARTAGE_ERREUR.lock().ok().and_then(|e| e.clone()),
    })
}

/// Règle le relais (où tu reçois) et le nom que tu te donnes. Crée l'identité
/// de cette installation au premier réglage.
#[tauri::command(async)]
fn core_partage_regler(relais: String, nom: String) -> Result<(), String> {
    let conn = base()?;
    waly_core::partage::regler(&conn, &relais, &nom)?;
    let hote = relais.split("//").nth(1).unwrap_or("").trim_end_matches('/').to_string();
    let _ = waly_core::sceau::noter(
        &conn,
        "sortie",
        &if hote.is_empty() { "partage coupé : plus de relais".to_string() } else { format!("partage ouvert : relève de ta boîte sur le relais {hote}") },
    );
    Ok(())
}

#[tauri::command(async)]
fn core_contact_ajouter(nom: String, code: String) -> Result<i64, String> {
    waly_core::partage::ajouter_contact(&base()?, &nom, &code)
}

#[tauri::command(async)]
fn core_contact_retirer(id: i64) -> Result<bool, String> {
    waly_core::partage::retirer_contact(&base()?, id)
}

/// Envoie la conversation OUVERTE à un contact : chiffrée ici pour lui seul,
/// déposée sur son relais. Inscrit au journal du sceau.
#[tauri::command(async)]
fn core_partage_envoyer(state: tauri::State<'_, Core>, contact: i64, titre: String) -> Result<String, String> {
    use waly_core::partage;
    let (session, _) = ask(&state, |tx| Cmd::History { reply: tx }).ok_or("cerveau indisponible")?;
    let conn = base()?;
    let messages = store::recent_messages_in(&conn, session, 400).map_err(|e| e.to_string())?;
    if messages.is_empty() {
        return Err("conversation vide".into());
    }
    let c = partage::contacts(&conn).into_iter().find(|c| c.id == contact).ok_or("contact introuvable")?;
    let octets = partage::envoyer_conversation(&conn, contact, titre.trim(), &messages)?;
    let hote = c.relais.split("//").nth(1).unwrap_or("").to_string();
    let _ = waly_core::sceau::noter(
        &conn,
        "sortie",
        &format!("partage : conversation « {} » envoyée à {} via {hote} ({octets} octets chiffrés)", titre.trim(), c.nom),
    );
    Ok(c.nom)
}

/// Accepter (la conversation entre dans l'app) ou refuser (elle est effacée).
#[tauri::command(async)]
fn core_partage_decider(id: i64, accepter: bool) -> Result<Option<i64>, String> {
    waly_core::partage::decider(&base()?, id, accepter)
}

// --- Lot 3 : passerelles de messagerie --------------------------------------

/// Passerelles connectées (jamais leur jeton) + dernière erreur de relève.
#[tauri::command(async)]
fn core_passerelles() -> serde_json::Value {
    let liste = store::open(&db_path()).map(|c| waly_core::passerelles::lister(&c)).unwrap_or_default();
    serde_json::json!({
        "passerelles": liste,
        "erreur": PASSERELLE_ERREUR.lock().ok().and_then(|e| e.clone()),
        "essais_max": waly_core::passerelles::ESSAIS_MAX,
    })
}

/// Connecte un bot Telegram : le jeton est vérifié auprès du service (une
/// vraie sortie), chiffré par Windows, et un code d'appairage est préparé.
#[tauri::command(async)]
fn core_passerelle_connecter(base_url: Option<String>, jeton: String) -> Result<waly_core::passerelles::Passerelle, String> {
    let conn = base()?;
    let p = waly_core::passerelles::connecter(&conn, base_url.as_deref().unwrap_or(""), &jeton)?;
    let hote = p.base_url.split("//").nth(1).unwrap_or("").to_string();
    let _ = waly_core::sceau::noter(&conn, "sortie", &format!("passerelle Telegram ouverte : @{} via {hote}", p.bot));
    Ok(p)
}

#[tauri::command(async)]
fn core_passerelle_retirer(id: i64) -> Result<(), String> {
    let conn = base()?;
    waly_core::passerelles::retirer(&conn, id)?;
    let _ = waly_core::sceau::noter(&conn, "sortie", "passerelle Telegram coupée — jeton effacé");
    Ok(())
}

/// Oublie le téléphone appairé et prépare un nouveau code.
#[tauri::command(async)]
fn core_passerelle_reappairer(id: i64) -> Result<String, String> {
    waly_core::passerelles::reappairer(&base()?, id)
}

// --- Lot 3 : télécharger d'autres modèles (2026-10-01) ----------------------

/// Téléchargement en cours ou dernier terminé — sondé par l'UI (il survit à
/// la fermeture du panneau : un modèle pèse des Go).
#[derive(Clone, Default, serde::Serialize)]
struct Telechargement {
    nom: String,
    statut: String,
    total: u64,
    fait: u64,
    fini: bool,
    annule: bool,
    erreur: Option<String>,
}

static TELECHARGEMENT: Mutex<Option<Telechargement>> = Mutex::new(None);
static TELECHARGEMENT_STOP: AtomicBool = AtomicBool::new(false);

/// Catalogue (verdict pour CETTE machine), modèles installés, moteur présent.
#[tauri::command(async)]
fn core_modeles() -> serde_json::Value {
    use waly_core::modeles;
    let profil = waly_core::materiel::detecter();
    let moteur = LlmClient::new("127.0.0.1", waly_core::llm::PORT_MOTEUR_B, "");
    let installes = moteur.modeles_detail();
    let est_installe = |nom: &str| {
        installes.iter().any(|i| i.nom == nom || i.nom.strip_suffix(":latest") == Some(nom))
    };
    serde_json::json!({
        "moteur": waly_core::llm::ecoute_local(waly_core::llm::PORT_MOTEUR_B),
        "actuel": waly_core::llm::modele_par_defaut(),
        "catalogue": modeles::CATALOGUE.iter().map(|e| serde_json::json!({
            "nom": e.nom, "taille_go": e.taille_go, "outils": e.outils, "vision": e.vision,
            "reflexion": e.reflexion, "note": e.note,
            "verdict": modeles::verdict(&profil, e.taille_go),
            "avertissement": modeles::avertissement(&profil, e.nom),
            "installe": est_installe(e.nom),
        })).collect::<Vec<_>>(),
        "installes": installes.iter().map(|i| serde_json::json!({
            "nom": i.nom, "taille_go": i.taille_go, "capacites": i.capacites,
            "verdict": modeles::verdict(&profil, i.taille_go),
        })).collect::<Vec<_>>(),
    })
}

/// Demande au moteur local de télécharger un modèle. Waly reste scellé :
/// c'est le moteur (hors périmètre) qui sort — inscrit au journal du sceau.
#[tauri::command]
fn core_modele_telecharger(nom: String) -> Result<(), String> {
    let nom = nom.trim().to_string();
    if !waly_core::modeles::nom_valide(&nom) {
        return Err("nom de modèle invalide".into());
    }
    if !waly_core::llm::ecoute_local(waly_core::llm::PORT_MOTEUR_B) {
        return Err("Ollama ne répond pas : ouvre-le, puis réessaie".into());
    }
    {
        let mut t = TELECHARGEMENT.lock().map_err(|e| e.to_string())?;
        if t.as_ref().is_some_and(|t| !t.fini) {
            return Err("un téléchargement est déjà en cours".into());
        }
        *t = Some(Telechargement { nom: nom.clone(), statut: "connexion au registre…".into(), ..Default::default() });
    }
    TELECHARGEMENT_STOP.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        let conn = store::open(&db_path()).ok();
        let noter = |detail: String| {
            if let Some(c) = &conn {
                let _ = waly_core::sceau::noter(c, "sortie", &detail);
            }
        };
        noter(format!("Ollama télécharge {nom} depuis son registre, à ta demande (Waly reste scellé)"));
        let moteur = LlmClient::new("127.0.0.1", waly_core::llm::PORT_MOTEUR_B, "");
        let res = waly_core::modeles::telecharger(&moteur, &nom, |p| {
            if !p.statut.is_empty() {
                if let Ok(mut t) = TELECHARGEMENT.lock() {
                    if let Some(t) = t.as_mut() {
                        t.statut = p.statut.clone();
                        (t.total, t.fait) = (p.total, p.fait);
                    }
                }
            }
            !TELECHARGEMENT_STOP.load(Ordering::Relaxed)
        });
        noter(match &res {
            Ok(true) => format!("téléchargement de {nom} terminé — la sortie du moteur est refermée"),
            Ok(false) => format!("téléchargement de {nom} annulé"),
            Err(e) => format!("téléchargement de {nom} échoué : {e}"),
        });
        if let Ok(mut t) = TELECHARGEMENT.lock() {
            if let Some(t) = t.as_mut() {
                t.fini = true;
                match res {
                    Ok(true) => t.statut = "installé".into(),
                    Ok(false) => t.annule = true,
                    Err(e) => t.erreur = Some(e),
                }
            }
        }
    });
    Ok(())
}

#[tauri::command]
fn core_modele_progression() -> Option<Telechargement> {
    TELECHARGEMENT.lock().ok().and_then(|t| t.clone())
}

/// Annule le téléchargement en cours (le moteur garde ce qui est déjà reçu).
#[tauri::command]
fn core_modele_annuler() {
    TELECHARGEMENT_STOP.store(true, Ordering::SeqCst);
}

/// Serveurs MCP branchés dans ce processus (worker) — le panneau « Vie
/// privée — la preuve » les montre : ils tournent HORS du scellé.
#[tauri::command]
fn core_mcp_actifs() -> Vec<waly_core::mcp::ServeurActif> {
    waly_core::mcp::actifs()
}

/// Compétences APPRISES (distillées en fin de mission) pour le panneau
/// Compétences : (titre, recette, usages). Connexion dédiée hors worker —
/// lecture seule, WAL.
#[tauri::command(async)]
fn core_competences() -> Vec<(String, String, i64)> {
    let Ok(conn) = store::open(&db_path()) else { return Vec::new() };
    store::list_competences(&conn)
        .map(|v| v.into_iter().map(|c| (c.titre, c.recette, c.uses)).collect())
        .unwrap_or_default()
}

// --- Plugins (lot 2) --------------------------------------------------------

#[tauri::command(async)]
fn core_plugins() -> Vec<waly_core::plugins::Plugin> {
    waly_core::plugins::installes()
}

/// Choisit le manifeste d'un plugin et le LIT sans rien installer : l'UI
/// montre ce qu'il apporte (et ce qu'il lancera) avant confirmation.
#[tauri::command(async)]
fn core_plugin_choisir() -> Result<Option<waly_core::plugins::Plugin>, String> {
    let Some(chemin) = waly_core::sceau::choisir_fichier(
        "Choisir le fichier waly-plugin.toml du plugin",
        "Manifeste de plugin Waly\0waly-plugin.toml\0Tous les fichiers\0*.*\0\0",
    ) else {
        return Ok(None);
    };
    let dossier = std::path::Path::new(&chemin).parent().ok_or("dossier introuvable")?;
    waly_core::plugins::lire(dossier).map(Some)
}

#[tauri::command(async)]
fn core_plugin_installer(dossier: String) -> Result<waly_core::plugins::Plugin, String> {
    waly_core::plugins::installer(&base()?, std::path::Path::new(&dossier))
}

#[tauri::command(async)]
fn core_plugin_activer(slug: String, actif: bool) -> Result<(), String> {
    waly_core::plugins::activer(&base()?, &slug, actif)
}

#[tauri::command(async)]
fn core_plugin_desinstaller(slug: String) -> Result<(), String> {
    waly_core::plugins::desinstaller(&base()?, &slug)
}

/// Le résumé du début d'une conversation (menu ⋯) : ce que Waly en retient.
#[tauri::command(async)]
fn core_resume(session: i64) -> Option<String> {
    store::resume_session(&base().ok()?, session).map(|(r, _)| r)
}

/// Compétences apprises avec leur version (Personnaliser, lot 2).
#[tauri::command(async)]
fn core_competences_detail() -> Result<Vec<serde_json::Value>, String> {
    let conn = base()?;
    let l = store::list_competences(&conn).map_err(|e| e.to_string())?;
    Ok(l.into_iter()
        .map(|c| {
            let v = store::version_competence(&conn, &c.key).unwrap_or(1);
            serde_json::json!({"key": c.key, "titre": c.titre, "declencheur": c.declencheur,
                               "recette": c.recette, "uses": c.uses, "version": v})
        })
        .collect())
}

#[tauri::command(async)]
fn core_competence_versions(key: String) -> Result<Vec<store::VersionCompetence>, String> {
    store::versions_competence(&base()?, &key).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn core_competence_restaurer(key: String, version: i64) -> Result<bool, String> {
    store::restaurer_competence(&base()?, &key, version).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn core_competence_supprimer(key: String) -> Result<bool, String> {
    store::delete_competence(&base()?, &key).map_err(|e| e.to_string())
}

/// Mémoire VISIBLE et MODIFIABLE (UI 2026-09-14, comme Claude/Hermes) : les
/// souvenirs actifs. Connexion dédiée hors worker (WAL) ; une modification
/// entre au prompt au prochain rebuild de fenêtre (discipline append-only).
#[tauri::command(async)]
fn core_memoires() -> Vec<serde_json::Value> {
    let Ok(conn) = store::open(&db_path()) else { return Vec::new() };
    store::active_memories(&conn)
        .map(|v| {
            v.into_iter()
                .map(|m| serde_json::json!({"cle": m.key, "categorie": m.category, "valeur": m.value}))
                .collect()
        })
        .unwrap_or_default()
}

/// Crée ou corrige un souvenir depuis l'UI (source « utilisateur »).
#[tauri::command(async)]
fn core_memoire_maj(cle: String, categorie: String, valeur: String) -> Result<(), String> {
    if !matches!(categorie.as_str(), "fact" | "preference" | "context" | "event") {
        return Err("catégorie inconnue".into());
    }
    let (cle, valeur) = (cle.trim(), valeur.trim());
    if cle.is_empty() || valeur.is_empty() {
        return Err("souvenir vide".into());
    }
    let conn = store::open(&db_path()).map_err(|e| e.to_string())?;
    // source : la table n'accepte que 'declared'/'inferred' — ce que
    // l'utilisateur écrit lui-même est déclaré.
    store::upsert_memory(&conn, &categorie, cle, valeur, "declared").map_err(|e| e.to_string())
}

/// Oublie un souvenir depuis l'UI.
#[tauri::command(async)]
fn core_memoire_oublier(cle: String) -> Result<bool, String> {
    let conn = store::open(&db_path()).map_err(|e| e.to_string())?;
    store::forget_memory(&conn, &cle).map_err(|e| e.to_string())
}

/// Annule un rappel depuis la page Programmé.
#[tauri::command(async)]
fn core_rappel_annuler(id: i64) -> Result<bool, String> {
    let conn = store::open(&db_path()).map_err(|e| e.to_string())?;
    store::cancel_reminder(&conn, id).map_err(|e| e.to_string())
}

/// Documents RÉELS du dossier Documents Waly (Mains v1) pour le Carnet :
/// fichiers (profondeur ≤ 2), les plus récents d'abord, 50 max.
#[tauri::command(async)]
fn core_documents() -> Vec<serde_json::Value> {
    fn ramasser(d: &std::path::Path, prof: usize, out: &mut Vec<(std::time::SystemTime, serde_json::Value)>) {
        if prof > 2 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let chemin = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => ramasser(&chemin, prof + 1, out),
                Ok(t) if t.is_file() => {
                    let modifie = e
                        .metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    out.push((
                        modifie,
                        serde_json::json!({
                            "nom": e.file_name().to_string_lossy(),
                            "chemin": chemin.to_string_lossy(),
                        }),
                    ));
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    ramasser(&waly_core::fichiers::racine_documents(), 0, &mut out);
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.into_iter().take(50).map(|(_, v)| v).collect()
}

/// Montre un document dans l'Explorateur (sélectionné). N'ouvre que des
/// fichiers EXISTANTS (les chemins viennent du Carnet ou des cartes de
/// livrables — jamais d'exécution, juste /select).
#[tauri::command(async)]
fn core_ouvrir_fichier(chemin: String) -> Result<(), String> {
    let p = std::path::PathBuf::from(&chemin);
    if !p.is_file() {
        return Err("fichier introuvable".into());
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", p.display()))
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err("Explorateur : Windows seulement".into())
    }
}

/// Auto-test du sceau : tente une vraie sortie reseau depuis waly.exe (process
/// scelle par defaut). Sous sceau -> bloquee + journalisee. Renvoie
/// {bloque, detail}. L'auto-verification du professionnel (« prouve que rien
/// ne sort »). Porte sur le PERIMETRE (scelle par defaut, ADR 2026-07-21).
#[tauri::command(async)]
fn core_sceau_tester() -> serde_json::Value {
    let (bloque, detail) = waly_core::sceau::tester_sortie();
    if let Ok(conn) = store::open(&db_path()) {
        let _ = waly_core::sceau::drainer_journal(waly_core::sceau::PERIMETRE, Some(&conn));
    }
    serde_json::json!({ "bloque": bloque, "detail": detail })
}

/// Draine les tentatives bloquees depuis le service (les inscrit en base) et
/// renvoie les lignes d'audit du PERIMETRE, recentes d'abord (vue du journal).
#[tauri::command(async)]
fn core_sceau_journal() -> Result<serde_json::Value, String> {
    let p = waly_core::sceau::PERIMETRE;
    let conn = store::open(&db_path()).map_err(|e| e.to_string())?;
    let _ = waly_core::sceau::drainer_journal(p, Some(&conn));
    let lignes = waly_core::sceau::lignes_audit(&conn, p, 200).map_err(|e| e.to_string())?;
    Ok(serde_json::to_value(lignes).unwrap_or_default())
}

// --- Huis clos universel (chantier C) : sceller un agent TIERS --------------
//
// L'app tourne en user ; sceller un exe hors périmètre Waly exige une élévation
// (ADR 2026-09-16). Ces commandes lancent le CLI du service en élevé (UAC) puis
// tiennent le registre app-side + le journal par agent.

/// Sélecteur de fichier natif (.exe) pour choisir l'agent à sceller.
#[tauri::command(async)]
fn core_agent_choisir() -> Option<String> {
    waly_core::sceau::choisir_exe()
}

/// Scelle un agent tiers (UAC). Enregistre au registre + draine son journal.
#[tauri::command(async)]
fn core_agent_seal(exe: String) -> serde_json::Value {
    match waly_core::sceau::sceller_agent(&exe) {
        Ok(_) => {
            if let Ok(conn) = store::open(&db_path()) {
                let _ = waly_core::sceau::enregistrer_agent(&conn, &exe);
                let s = waly_core::sceau::session_pour_exe(&exe);
                let _ = waly_core::sceau::drainer_journal(s, Some(&conn));
            }
            serde_json::json!({ "ok": true, "message": format!("{exe} scellé — sortie réseau bloquée") })
        }
        Err(e) => serde_json::json!({ "ok": false, "message": e }),
    }
}

/// Lève le sceau d'un agent tiers (UAC) et l'oublie du registre.
#[tauri::command(async)]
fn core_agent_unseal(exe: String) -> serde_json::Value {
    match waly_core::sceau::desceller_agent(&exe) {
        Ok(()) => {
            if let Ok(conn) = store::open(&db_path()) {
                let _ = waly_core::sceau::oublier_agent(&conn, &exe);
            }
            serde_json::json!({ "ok": true, "message": format!("{exe} descellé — sortie réseau restaurée") })
        }
        Err(e) => serde_json::json!({ "ok": false, "message": e }),
    }
}

/// Liste les agents tiers scellés (registre app-side, statut vivant du service).
#[tauri::command(async)]
fn core_agents() -> Result<serde_json::Value, String> {
    let conn = store::open(&db_path()).map_err(|e| e.to_string())?;
    let a = waly_core::sceau::agents(&conn).map_err(|e| e.to_string())?;
    Ok(serde_json::to_value(a).unwrap_or_default())
}

use waly_core::garde::Ressource;

/// Agents figes par la Garde (« Tout couper ») : cle `nom|exe` -> processus
/// figes. Tenu en memoire : a la fermeture de Waly, tout est relance (un
/// agent ne doit pas rester fige sans personne pour le relancer).
fn figes() -> &'static Mutex<std::collections::HashMap<String, Vec<u32>>> {
    static F: std::sync::OnceLock<Mutex<std::collections::HashMap<String, Vec<u32>>>> = std::sync::OnceLock::new();
    F.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn relancer_tous_les_figes() {
    if let Ok(mut f) = figes().lock() {
        for (_, pids) in f.drain() {
            waly_core::agents_machine::relancer(&pids);
        }
    }
}

/// Fige un agent : tous SES processus (pas le programme, donc precis meme sur
/// un moteur partage). Reversible par `core_agent_relancer`.
#[tauri::command(async)]
fn core_agent_figer(nom: String, exe: String) -> serde_json::Value {
    let trouve = waly_core::agents_machine::trouver()
        .into_iter()
        .find(|a| a.nom == nom && a.exe.eq_ignore_ascii_case(&exe));
    let Some(a) = trouve else {
        return serde_json::json!({ "ok": false, "message": format!("{nom} ne tourne plus") });
    };
    let n = waly_core::agents_machine::figer(&a.pids);
    if n == 0 {
        return serde_json::json!({ "ok": false, "message": format!("Windows a refusé de figer {nom} (processus protégé ?)") });
    }
    if let Ok(mut f) = figes().lock() {
        let liste = f.entry(format!("{nom}|{}", exe.to_lowercase())).or_default();
        for p in &a.pids {
            if !liste.contains(p) {
                liste.push(*p);
            }
        }
    }
    if let Ok(conn) = base() {
        let _ = waly_core::sceau::noter(&conn, "pose", &format!("{nom} figé par la Garde ({n} processus)"));
    }
    serde_json::json!({ "ok": true, "message": format!("{n} processus figé(s)") })
}

#[tauri::command(async)]
fn core_agent_relancer(nom: String, exe: String) -> serde_json::Value {
    let pids = figes().lock().ok().and_then(|mut f| f.remove(&format!("{nom}|{}", exe.to_lowercase()))).unwrap_or_default();
    let n = waly_core::agents_machine::relancer(&pids);
    if let Ok(conn) = base() {
        let _ = waly_core::sceau::noter(&conn, "levee", &format!("{nom} relancé ({n} processus)"));
    }
    serde_json::json!({ "ok": true, "message": format!("{n} processus relancé(s)") })
}

// --- La Garde, etape 4 : l'enclos (compte Windows a part) -------------------

/// L'enclos tel que la Garde le montre : le compte, l'essai du profil, les
/// dossiers regles avec le resultat de leur sonde. Lecture seule, sans essai
/// (les essais se font au reglage et sur demande).
fn enclos_etat(conn: &rusqlite::Connection) -> serde_json::Value {
    use waly_core::enclos;
    let essai = enclos::essai_profil(conn);
    let dossiers: Vec<serde_json::Value> = enclos::dossiers(conn)
        .iter()
        .map(|d| serde_json::json!({
            "chemin": d.chemin, "droit": d.droit.cle(), "pourquoi": d.pourquoi,
            "lit": d.lit, "ecrit": d.ecrit, "quand": d.essaye_at, "conforme": d.conforme(),
        }))
        .collect();
    serde_json::json!({
        "compte": enclos::COMPTE, "existe": enclos::existe(), "pret": enclos::pret(conn), "profil": enclos::profil(),
        "profil_invisible": essai.as_ref().map(|e| e.0), "profil_essaye": essai.map(|e| e.1),
        "dossiers": dossiers,
    })
}

/// Cree le compte de l'enclos (Windows demande l'accord, une fois).
#[tauri::command(async)]
fn core_enclos_creer() -> serde_json::Value {
    let conn = match base() {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "ok": false, "message": e }),
    };
    match waly_core::enclos::creer(&conn) {
        Ok(()) => serde_json::json!({ "ok": true }),
        Err(e) => serde_json::json!({ "ok": false, "message": e }),
    }
}

/// Met un agent qui tourne dans l'enclos : on lui donne en lecture ce qu'il
/// lui faut pour demarrer, on ferme l'instance en cours, on le relance sous
/// le compte a part. Si la preparation echoue, rien n'est ferme.
#[tauri::command(async)]
fn core_enclos_mettre(nom: String, exe: String) -> serde_json::Value {
    use waly_core::{agents_machine, enclos};
    let non = |m: String| serde_json::json!({ "ok": false, "message": m });
    let conn = match base() {
        Ok(c) => c,
        Err(e) => return non(e),
    };
    if !enclos::pret(&conn) {
        if let Err(e) = enclos::creer(&conn) {
            return non(e);
        }
    }
    let Some(a) = agents_machine::trouver().into_iter().find(|a| a.nom == nom && a.exe.eq_ignore_ascii_case(&exe)) else {
        return non(format!("{nom} ne tourne plus : lance-le, puis recommence"));
    };
    let donnes = match enclos::preparer(&conn, &nom, &exe, &a.ligne) {
        Ok(d) => d,
        Err(e) => return non(format!("{nom} n'a pas été touché. {e}")),
    };
    // S'il etait fige, on ne garde pas des identifiants qui vont disparaitre.
    if let Ok(mut f) = figes().lock() {
        if let Some(pids) = f.remove(&format!("{nom}|{}", exe.to_lowercase())) {
            agents_machine::relancer(&pids);
        }
    }
    let fermes = agents_machine::fermer(&a.pids);
    std::thread::sleep(std::time::Duration::from_millis(900));
    match enclos::lancer(&conn, &nom, &exe, &a.ligne) {
        Ok(_) => serde_json::json!({ "ok": true, "donnes": donnes, "fermes": fermes }),
        Err(e) => non(format!("{nom} a été fermé ({fermes} processus) mais n'a pas redémarré dans l'enclos : {e}")),
    }
}

/// Relance un agent de l'enclos qui s'est arrete.
#[tauri::command(async)]
fn core_enclos_relancer(exe: String) -> serde_json::Value {
    match base().and_then(|c| waly_core::enclos::relancer(&c, &exe)) {
        Ok(_) => serde_json::json!({ "ok": true }),
        Err(e) => serde_json::json!({ "ok": false, "message": e }),
    }
}

/// Arrete un agent de l'enclos (lui et ce qu'il a lance). `oublier` : il
/// quitte l'enclos ; l'utilisateur le relancera comme d'habitude.
#[tauri::command(async)]
fn core_enclos_arreter(exe: String, oublier: bool) -> serde_json::Value {
    match base() {
        Ok(conn) => {
            let n = waly_core::enclos::arreter(&conn, &exe, oublier);
            if oublier {
                let _ = waly_core::sceau::noter(&conn, "levee", &format!("{exe} sorti de l'enclos"));
            }
            serde_json::json!({ "ok": true, "arretes": n })
        }
        Err(e) => serde_json::json!({ "ok": false, "message": e }),
    }
}

/// Regle un dossier pour l'enclos. `droit` : « lecture », « ecriture »,
/// « coupe », ou « reprendre » (retire le reglage). Sans `chemin`, Windows
/// ouvre le choix d'un dossier. Chaque reglage est suivi de son essai.
#[tauri::command(async)]
fn core_enclos_dossier(chemin: Option<String>, droit: String) -> serde_json::Value {
    use waly_core::enclos::{self, Droit};
    let non = |m: String| serde_json::json!({ "ok": false, "message": m });
    let voulu = match droit.as_str() {
        "reprendre" => None,
        d => match Droit::depuis(d) {
            Some(d) => Some(d),
            None => return non(format!("réglage inconnu : {d}")),
        },
    };
    let chemin = match chemin.filter(|c| !c.trim().is_empty()) {
        Some(c) => c,
        None => {
            let titre = match voulu {
                Some(Droit::Coupe) => "Choisis le dossier à couper à l'enclos",
                Some(Droit::Ecriture) => "Choisis le dossier que l'enclos pourra lire et modifier",
                _ => "Choisis le dossier que l'enclos pourra lire",
            };
            match enclos::choisir_dossier(titre) {
                Some(c) => c,
                None => return serde_json::json!({ "ok": false, "annule": true }),
            }
        }
    };
    let conn = match base() {
        Ok(c) => c,
        Err(e) => return non(e),
    };
    match enclos::regler(&conn, &chemin, voulu, "") {
        Ok(d) => serde_json::json!({
            "ok": true, "chemin": chemin,
            "lit": d.as_ref().and_then(|d| d.lit), "ecrit": d.as_ref().and_then(|d| d.ecrit),
            "conforme": d.as_ref().and_then(|d| d.conforme()),
        }),
        Err(e) => non(e),
    }
}

/// Refait tous les essais de l'enclos : chaque dossier regle, et le profil
/// (qui doit rester invisible).
#[tauri::command(async)]
fn core_enclos_essai() -> serde_json::Value {
    match base().and_then(|c| waly_core::enclos::tout_essayer(&c)) {
        Ok(invisible) => serde_json::json!({ "ok": true, "profil_invisible": invisible }),
        Err(e) => serde_json::json!({ "ok": false, "message": e }),
    }
}

/// La Garde, pour les sessions ouvertes par l'interface (appel, voix, ecran,
/// dictee) : TOUTES les ressources doivent etre permises, sinon rien ne
/// s'ouvre. Refus et touches sont notes au registre.
fn garde_passer(quoi: &[(Ressource, &str)]) -> Result<(), String> {
    let conn = base()?;
    if let Some((r, detail)) = quoi.iter().find(|(r, _)| !waly_core::garde::permis(&conn, *r)) {
        waly_core::garde::noter(&conn, *r, &format!("{detail} : refusé, accès coupé"), true);
        return Err(format!("L'accès « {} » est coupé dans la Garde. Rétablis-le pour continuer.", r.nom()));
    }
    for (r, detail) in quoi {
        waly_core::garde::noter(&conn, *r, detail, false);
    }
    Ok(())
}

/// Releve ce que le service a observe depuis la derniere fois et l'inscrit
/// au registre, au nom de l'agent. Rend (mode, programmes surveilles, erreur).
/// Les programmes de Waly sont ecartes : son propre registre est exact.
fn regard_relever(conn: &rusqlite::Connection, noms: &[(String, String)]) -> (String, Vec<String>, Option<String>) {
    static VU: Mutex<u64> = Mutex::new(0);
    let Ok(mut vu) = VU.lock() else { return ("rien".into(), Vec::new(), None) };
    match waly_core::sceau::regard_lire(*vu) {
        Ok((mode, exes, obs, dernier)) => {
            if dernier < *vu {
                *vu = 0; // le service a redemarre : son carnet repart de zero
                return (mode, exes, None);
            }
            for o in &obs {
                let base = o.exe.rsplit(['\\', '/']).next().unwrap_or(&o.exe).to_string();
                if base.to_lowercase().starts_with("waly") {
                    continue;
                }
                let agent = noms
                    .iter()
                    .find(|(exe, _)| exe.eq_ignore_ascii_case(&o.exe))
                    .map(|(_, n)| n.clone())
                    .or_else(|| waly_core::agents_machine::reconnaitre(&o.exe, "").map(String::from))
                    .unwrap_or(base);
                let (ressource, detail) = waly_core::garde::dire_observation(&o.genre, &o.objet);
                waly_core::garde::noter_agent(conn, &agent, ressource, &detail);
            }
            *vu = dernier;
            (mode, exes, None)
        }
        Err(e) => ("rien".into(), Vec::new(), Some(e)),
    }
}

/// Regle la surveillance : « rien », « agents » (les programmes donnes et ce
/// qu'ils lancent) ou « tout » (toute la machine). Allumer demande l'accord
/// de Windows.
#[tauri::command(async)]
fn core_regard(mode: String, exes: Vec<String>) -> serde_json::Value {
    match waly_core::sceau::regard_regler(&mode, &exes) {
        Ok(()) => {
            if let Ok(conn) = base() {
                let quoi = match mode.as_str() {
                    "tout" => "surveillance de toute la machine allumée".to_string(),
                    "agents" => format!("surveillance allumée pour {} programme(s)", exes.len()),
                    _ => "surveillance éteinte".to_string(),
                };
                let _ = waly_core::sceau::noter(&conn, if mode == "rien" { "levee" } else { "pose" }, &quoi);
            }
            serde_json::json!({ "ok": true })
        }
        Err(e) => serde_json::json!({ "ok": false, "message": e }),
    }
}

/// Tout ce que la page « Garde » montre, en une lecture : l'etat prouve du
/// scelle, les acces de Waly (permis ou coupes, dernier geste), les sorties
/// ouvertes, les autres agents, les chiffres du jour et le fil.
#[tauri::command(async)]
fn core_garde() -> Result<serde_json::Value, String> {
    use waly_core::{garde, sceau};
    let conn = base()?;
    let etat = core_sceau_etat();
    let _ = sceau::drainer_journal(sceau::PERIMETRE, Some(&conn));
    let acces: Vec<serde_json::Value> = Ressource::TOUTES
        .into_iter()
        .map(|r| {
            let d = garde::dernier_par_ressource(&conn, r);
            serde_json::json!({
                "cle": r.cle(), "nom": r.nom(), "permis": garde::permis(&conn, r),
                "dernier": d.as_ref().map(|l| l.detail.clone()), "quand": d.map(|l| l.at),
            })
        })
        .collect();
    // Ce que le service a observe des AUTRES programmes entre au registre
    // avant de lire le fil.
    let mut agents = agents_trouves(&conn);
    let noms: Vec<(String, String)> = agents
        .iter()
        .filter_map(|a| Some((a["exe"].as_str()?.to_string(), a["nom"].as_str()?.to_string())))
        .collect();
    let (mode_regard, exes_regard, erreur_regard) = regard_relever(&conn, &noms);
    for a in agents.iter_mut() {
        let (nom, exe) = (a["nom"].as_str().unwrap_or("").to_string(), a["exe"].as_str().unwrap_or("").to_lowercase());
        a["surveille"] = serde_json::json!(mode_regard == "tout" || exes_regard.contains(&exe));
        a["vu_fichiers"] = serde_json::json!(garde::vus_par(&conn, &nom, "fichiers"));
        a["vu_internet"] = serde_json::json!(garde::vus_par(&conn, &nom, "internet"));
        a["vu_programmes"] = serde_json::json!(garde::vus_par(&conn, &nom, "programmes"));
    }
    let (touches, refus) = garde::comptes_du_jour(&conn);
    let audit = sceau::lignes_audit(&conn, sceau::PERIMETRE, 60).unwrap_or_default();
    let aujourdhui: String = conn
        .query_row("SELECT date('now','localtime')", [], |r| r.get(0))
        .unwrap_or_default();
    let bloquees = audit.iter().filter(|l| l.genre == "bloque" && l.at.starts_with(&aujourdhui)).count() as i64;
    // Le fil : le registre de Waly et le journal du scelle, fondus par date.
    let mut fil: Vec<serde_json::Value> = garde::lignes(&conn, 300)
        .unwrap_or_default()
        .into_iter()
        .map(|l| serde_json::json!({
            "at": l.at, "agent": l.agent, "detail": l.detail, "ressource": l.ressource,
            "genre": if l.agent == "Toi" { "toi" } else if l.refuse { "refus" } else { "touche" },
        }))
        .collect();
    for l in &audit {
        let (agent, genre, detail) = match l.genre.as_str() {
            "bloque" => ("Waly", "refus", format!("sortie bloquée — {}", l.detail)),
            "sortie" => ("Waly", "porte", format!("sortie ouverte — {}", l.detail)),
            _ => continue,
        };
        fil.push(serde_json::json!({ "at": l.at, "agent": agent, "detail": detail, "genre": genre, "ressource": "internet" }));
    }
    fil.sort_by(|a, b| b["at"].as_str().cmp(&a["at"].as_str()));
    fil.truncate(320);
    Ok(serde_json::json!({
        "sceau": etat, "acces": acces, "agents": agents,
        "touches": touches, "refus": refus + bloquees, "fil": fil,
        "regard": { "mode": mode_regard, "exes": exes_regard, "erreur": erreur_regard },
        "enclos": enclos_etat(&conn),
    }))
}

/// Coupe ou rend un acces de Waly. La memoire touche au prompt : on
/// reconstruit la fenetre pour que la coupure vaille tout de suite.
#[tauri::command(async)]
fn core_garde_acces(state: tauri::State<'_, Core>, cle: String, permis: bool) -> Result<(), String> {
    let r = Ressource::depuis(&cle).ok_or_else(|| format!("ressource inconnue : {cle}"))?;
    waly_core::garde::regler(&base()?, r, permis).map_err(|e| e.to_string())?;
    if r == Ressource::Memoire {
        let _ = ask(&state, |tx| Cmd::Rebuild { reply: tx });
    }
    Ok(())
}

/// Les autres agents de la machine : ceux qui TOURNENT et que Waly reconnait
/// (`agents_machine`), plus ceux deja scelles. Pour chacun : le programme sur
/// lequel porte le scelle, s'il est partage avec d'autres logiciels, s'il est
/// scelle, et ce qu'il a tente (sorties bloquees). Lecture seule.
#[tauri::command(async)]
fn core_agents_trouves() -> Result<serde_json::Value, String> {
    let conn = store::open(&db_path()).map_err(|e| e.to_string())?;
    Ok(serde_json::Value::Array(agents_trouves(&conn)))
}

fn agents_trouves(conn: &rusqlite::Connection) -> Vec<serde_json::Value> {
    use waly_core::sceau;
    let scelles = sceau::agents(conn).unwrap_or_default();
    let mut lignes: Vec<serde_json::Value> = Vec::new();
    let mut vus: Vec<String> = Vec::new();
    let fig: Vec<String> = figes().lock().map(|f| f.keys().cloned().collect()).unwrap_or_default();
    // L'enclos : les agents qu'on y a mis, et leurs processus en cours.
    let dans_enclos = waly_core::enclos::agents(conn);
    let pids_enclos: Vec<u32> = dans_enclos.iter().flat_map(|a| a.pids.iter().copied()).collect();
    let mut decrire = |nom: String, exe: String, partage: bool, autres: usize, en_cours: bool, pids: &[u32]| {
        let fige = fig.contains(&format!("{nom}|{}", exe.to_lowercase()));
        // Dans l'enclos : TOUS ses processus y sont (ou il y est inscrit et
        // arrete). Une instance lancee a la main, hors de l'enclos, se compte.
        let dedans = pids.iter().filter(|p| pids_enclos.contains(p)).count();
        let enclos = if pids.is_empty() { dans_enclos.iter().any(|a| a.exe.eq_ignore_ascii_case(&exe)) } else { dedans == pids.len() };
        let hors_enclos = if dedans > 0 { pids.len() - dedans } else { 0 };
        let scelle = scelles.iter().find(|a| a.exe.eq_ignore_ascii_case(&exe)).map(|a| a.actif).unwrap_or(false);
        let (mut bloquees, mut derniere) = (0usize, String::new());
        if scelle {
            let s = sceau::session_pour_exe(&exe);
            let _ = sceau::drainer_journal(s, Some(conn));
            if let Ok(l) = sceau::lignes_audit(conn, s, 200) {
                let b: Vec<_> = l.iter().filter(|x| x.genre == "bloque").collect();
                bloquees = b.len();
                derniere = b.first().map(|x| format!("{} · {}", x.at, x.detail)).unwrap_or_default();
            }
        }
        lignes.push(serde_json::json!({
            "origine": waly_core::agents_machine::origine(&exe),
            "nom": nom, "exe": exe, "partage": partage, "autres": autres,
            "en_cours": en_cours, "scelle": scelle, "bloquees": bloquees, "derniere": derniere, "fige": fige,
            "enclos": enclos, "hors_enclos": hors_enclos,
        }));
    };
    for a in waly_core::agents_machine::trouver() {
        vus.push(a.exe.to_lowercase());
        decrire(a.nom, a.exe, a.partage, a.autres, true, &a.pids);
    }
    for a in &dans_enclos {
        if !vus.contains(&a.exe.to_lowercase()) {
            vus.push(a.exe.to_lowercase());
            decrire(a.nom.clone(), a.exe.clone(), false, 0, false, &[]);
        }
    }
    for a in &scelles {
        if !vus.contains(&a.exe.to_lowercase()) {
            decrire(a.nom.clone(), a.exe.clone(), false, 0, false, &[]);
        }
    }
    lignes
}

/// Journal d'audit d'un agent tiers (draine le service puis renvoie les lignes).
#[tauri::command(async)]
fn core_agent_journal(exe: String) -> Result<serde_json::Value, String> {
    let s = waly_core::sceau::session_pour_exe(&exe);
    let conn = store::open(&db_path()).map_err(|e| e.to_string())?;
    let _ = waly_core::sceau::drainer_journal(s, Some(&conn));
    let lignes = waly_core::sceau::lignes_audit(&conn, s, 200).map_err(|e| e.to_string())?;
    Ok(serde_json::to_value(lignes).unwrap_or_default())
}

/// Cadre de présence (plein écran, transparent, click-through, toujours au-
/// dessus) + pop-up voix flottant (petit, bas-centre, déplaçable). Overlay
/// natif : Waly VIT autour de l'écran pendant qu'on navigue ailleurs.
fn ouvrir_fenetres_ecran(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    // Taille de l'écran principal (logique) pour cadrer et poser le pop-up.
    let (sw, sh) = app
        .get_webview_window("main")
        .and_then(|w| w.primary_monitor().ok().flatten())
        .map(|m| {
            let s = m.size();
            let sf = m.scale_factor();
            (s.width as f64 / sf, s.height as f64 / sf)
        })
        .unwrap_or((1280.0, 800.0));
    // Mêmes arguments navigateur que la fenêtre principale (durcis R6a) :
    // WebView2 refuse un 2e environnement aux options différentes sur le même
    // dossier de données -> sans ça, pop-up et cadre ne s'ouvraient JAMAIS,
    // en silence (vécu 2026-09-29, cassé depuis le durcissement du 21/07).
    let args = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .and_then(|w| w.additional_browser_args.clone());
    if app.get_webview_window("border").is_none() {
        let mut b = WebviewWindowBuilder::new(app, "border", WebviewUrl::App("border.html".into()));
        if let Some(a) = &args {
            b = b.additional_browser_args(a);
        }
        let border = b
            .title("Waly présence")
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .focused(false)
            .shadow(false)
            .inner_size(sw, sh)
            .position(0.0, 0.0)
            .build()
            .map_err(|e| format!("cadre présence: {e}"))?;
        let _ = border.set_ignore_cursor_events(true); // click-through
        // Exclure de la capture : Waly ne doit PAS se voir lui-même.
        #[cfg(windows)]
        if let Ok(h) = border.hwnd() {
            waly_sight::screen::exclude_from_capture(h.0 as isize);
        }
        #[cfg(not(windows))]
        let _ = &border;
    }
    if app.get_webview_window("popup").is_none() {
        let (pw, ph) = (340.0, 98.0);
        let mut b = WebviewWindowBuilder::new(app, "popup", WebviewUrl::App("popup.html".into()));
        if let Some(a) = &args {
            b = b.additional_browser_args(a);
        }
        let popup = b
            .title("Waly")
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .inner_size(pw, ph)
            .position((sw - pw) / 2.0, sh - ph - 52.0)
            .build()
            .map_err(|e| format!("pop-up voix: {e}"))?;
        #[cfg(windows)]
        if let Ok(h) = popup.hwnd() {
            waly_sight::screen::exclude_from_capture(h.0 as isize);
        }
        #[cfg(not(windows))]
        let _ = &popup;
    }
    // La fenêtre PRINCIPALE aussi : pendant le partage, Waly ne doit pas se
    // voir (son propre chat) — il voit le travail de Michée. Ré-incluse à l'arrêt.
    #[cfg(windows)]
    if let Some(main) = app.get_webview_window("main") {
        if let Ok(h) = main.hwnd() {
            waly_sight::screen::exclude_from_capture(h.0 as isize);
        }
    }
    Ok(())
}

fn fermer_fenetres_ecran(app: &tauri::AppHandle) {
    for label in ["popup", "border"] {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.close();
        }
    }
    // Ré-inclure la fenêtre principale dans les captures (fin du partage).
    #[cfg(windows)]
    if let Some(main) = app.get_webview_window("main") {
        if let Ok(h) = main.hwnd() {
            waly_sight::screen::include_in_capture(h.0 as isize);
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (tx, rx) = channel::<Cmd>();
    {
        let tx = tx.clone();
        std::thread::spawn(move || boucle_passerelles(tx));
    }
    std::thread::spawn(boucle_partage);
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let pouls = Arc::new(Mutex::new(Pouls::default()));
    let worker_pouls = pouls.clone();
    let mic_muted = Arc::new(AtomicBool::new(false));
    let worker_mic = mic_muted.clone();
    let cible_ecran = Arc::new(std::sync::atomic::AtomicI64::new(0));
    let worker_cible = cible_ecran.clone();
    let eveil = Arc::new(Mutex::new(None));
    let worker_eveil = eveil.clone();
    let eveil_annule = Arc::new(Mutex::new(None));
    let worker_eveil_annule = eveil_annule.clone();
    std::thread::spawn(move || {
        worker(
            rx,
            worker_cancel,
            worker_pouls,
            worker_mic,
            worker_cible,
            worker_eveil,
            worker_eveil_annule,
        )
    });

    // Huis clos (ADR 2026-07-21) : scelle PAR DEFAUT. Le service scelle le
    // perimetre au demarrage ; l'app REJOINT en declarant son propre chemin
    // d'exe (et celui de la voix qu'elle spawne) — car le chemin par-
    // utilisateur (%LOCALAPPDATA%) est inconnu du service SYSTEM au boot.
    // Best-effort, non bloquant, silencieux (l'etat est lisible via le journal).
    std::thread::spawn(|| {
        // Le scellé d'abord (il n'attend pas la base) ; le journal ensuite.
        let exe = std::env::current_exe().ok();
        if let Some(exe) = &exe {
            let _ = waly_core::sceau::rejoindre(&exe.to_string_lossy(), None);
        }
        attendre_base();
        let conn = store::open(&db_path()).ok();
        if let Some(exe) = exe {
            let _ = waly_core::sceau::rejoindre(&exe.to_string_lossy(), conn.as_ref());
        }
        let voix = std::env::var("WALY_VOICE_EXE")
            .unwrap_or_else(|_| waly_core::chemins::exe("waly-voice"));
        let _ = waly_core::sceau::rejoindre(&voix, conn.as_ref());
    });

    tauri::Builder::default()
        .manage(Core {
            tx: Mutex::new(tx),
            cancel,
            cliche: Mutex::new(None),
            pouls,
            mic_muted,
            cible_ecran,
            eveil,
            eveil_annule,
            dictee: Mutex::new(None),
        })
        .setup(|app| {
            let _ = APP.set(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core_send,
            core_reglages,
            core_reglage_set,
            core_exporter,
            core_fichier_choisir,
            core_fichier_lire,
            core_utilisation,
            core_projets,
            core_outils_etat,
            core_plugins,
            core_plugin_choisir,
            core_plugin_installer,
            core_plugin_activer,
            core_plugin_desinstaller,
            core_resume,
            core_competences_detail,
            core_competence_versions,
            core_competence_restaurer,
            core_competence_supprimer,
            core_missions_programmees,
            core_mission_programmer,
            core_mission_programmee_annuler,
            core_projet_de_session,
            core_projet,
            core_projet_creer,
            core_projet_maj,
            core_projet_supprimer,
            core_session_projet,
            core_projet_fichier_ajouter,
            core_projet_fichier_retirer,
            core_stop,
            core_history,
            core_sessions,
            core_new_session,
            core_select_session,
            core_session_supprimer,
            core_session_renommer,
            core_apercu,
            core_dictee_start,
            core_dictee_stop,
            core_dictee_annuler,
            core_fichiers_session,
            core_telecharger,
            core_search,
            core_skills,
            core_artifacts,
            core_agent_start,
            core_pendings,
            core_approve,
            core_appel_start,
            core_appel_stop,
            core_appel_cliche,
            core_appel_pouls,
            core_eveil,
            core_eveil_annule,
            core_voix_start,
            core_voix_stop,
            core_ecran_start,
            core_ecran_stop,
            core_ecran_mute,
            core_ecran_cible,
            core_ecran_fenetres,
            core_regarde_start,
            core_regarde_stop,
            core_modele,
            core_competences,
            core_mcp_actifs,
            core_materiel,
            core_materiel_appliquer,
            core_modeles,
            core_partage,
            core_partage_regler,
            core_contact_ajouter,
            core_contact_retirer,
            core_partage_envoyer,
            core_partage_decider,
            core_passerelles,
            core_passerelle_connecter,
            core_passerelle_retirer,
            core_passerelle_reappairer,
            core_cerveau,
            core_cerveau_choisir,
            core_exterieur_ajouter,
            core_exterieur_retirer,
            core_exterieur_tester,
            core_modele_telecharger,
            core_modele_progression,
            core_modele_annuler,
            core_documents,
            core_memoires,
            core_memoire_maj,
            core_memoire_oublier,
            core_rappel_annuler,
            core_ouvrir_fichier,
            core_sceau_etat,
            core_sceau_tester,
            core_sceau_journal,
            core_agent_choisir,
            core_agent_seal,
            core_agent_unseal,
            core_agents,
            core_agents_trouves,
            core_garde,
            core_garde_acces,
            core_regard,
            core_agent_figer,
            core_agent_relancer,
            core_enclos_creer,
            core_enclos_mettre,
            core_enclos_relancer,
            core_enclos_arreter,
            core_enclos_dossier,
            core_enclos_essai,
            core_agent_journal
        ])
        .build(tauri::generate_context!())
        .expect("Erreur au lancement de Waly")
        .run(|_app, evenement| {
            // Waly se ferme : aucun agent ne reste fige derriere lui.
            if let tauri::RunEvent::Exit = evenement {
                relancer_tous_les_figes();
            }
        });
}
