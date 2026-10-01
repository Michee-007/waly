//! Outils cœur natifs (R2) — descriptions TERSES : le budget prompt total
//! est < 1k tokens (critère RFC). Chaque outil déclare ses capacités
//! (writes_memory/irreversible) dans sa définition : UNE seule table.

use std::cell::RefCell;
use std::rc::Rc;

use rusqlite::Connection;

use crate::clock;
use crate::embed::{self, Embedder};
use crate::llm::ToolSpec;
use crate::safety::Capabilities;
use crate::store;
use crate::tools::{Registry, Tool};

/// Embedder partagé entre outils — optionnel : sans lui, la mémoire retombe
/// sur le mot-clé + fallback tous-souvenirs (aucune fonctionnalité perdue,
/// juste le rappel sémantique en moins).
pub type SharedEmbedder = Option<Rc<RefCell<Embedder>>>;

/// Enregistre les outils cœur sur une base ouverte.
pub fn register_core_tools(registry: &mut Registry, conn: Rc<Connection>, emb: SharedEmbedder) {
    registry.register(Box::new(Heure));
    registry.register(Box::new(Memoriser { conn: conn.clone(), emb: emb.clone() }));
    registry.register(Box::new(ChercherMemoire { conn: conn.clone(), emb: emb.clone() }));
    registry.register(Box::new(Oublier { conn: conn.clone() }));
    registry.register(Box::new(CreerNote { conn: conn.clone(), emb }));
    registry.register(Box::new(ChercherNotes { conn: conn.clone() }));
    registry.register(Box::new(ListerNotes { conn: conn.clone() }));
    registry.register(Box::new(CreerTache { conn: conn.clone() }));
    registry.register(Box::new(ListerTaches { conn: conn.clone() }));
    registry.register(Box::new(MajTache { conn: conn.clone() }));
    registry.register(Box::new(PoserRappel { conn: conn.clone() }));
    registry.register(Box::new(ListerRappels { conn: conn.clone() }));
    registry.register(Box::new(AnnulerRappel { conn }));
}

/// Image capturée fournie par l'hôte de l'outil `regarder` (desktop : la
/// dernière frame de la boucle de perception ; bin : fichier de banc).
pub struct ImageCapturee {
    pub data_url: String,
    pub largeur: u32,
    pub hauteur: u32,
}

pub type FournisseurImage = Box<dyn Fn() -> Result<ImageCapturee, String>>;

/// Enregistre l'outil vision `regarder` (R4 ch. 4). À n'appeler QUE si une
/// caméra/source d'image existe réellement (règle d'honnêteté : pas d'outil
/// fantôme au catalogue).
pub fn register_vision_tool(registry: &mut Registry, fournisseur: FournisseurImage) {
    register_vision_tool_si(registry, fournisseur, Box::new(|| true));
}

/// Variante honnête (vécu 2026-09-14 : caméra éteinte, Waly « voyait » une
/// pièce inventée) : l'outil n'est au catalogue QUE si `disponible()` dit
/// qu'une caméra tourne vraiment.
pub fn register_vision_tool_si(
    registry: &mut Registry,
    fournisseur: FournisseurImage,
    disponible: Box<dyn Fn() -> bool>,
) {
    registry.register(Box::new(Regarder { fournisseur, disponible, en_attente: RefCell::new(None) }));
}

/// Message d'échec d'une capture : sans image, le petit modèle invente une
/// scène si l'erreur n'est pas explicite.
fn echec_vue(source: &str, e: &str) -> String {
    format!(
        "ECHEC — aucune image ({source} : {e}). Tu ne vois RIEN : dis-le simplement a \
         l'utilisateur, ne decris aucune scene et ne pretends pas avoir regarde."
    )
}

/// Outil vision : capture une image et la donne à voir au cerveau (VLM
/// unique depuis R4). L'image est INJECTÉE comme message utilisateur après
/// le résultat du round (`take_injection`) — jamais écrite sur disque.
struct Regarder {
    fournisseur: FournisseurImage,
    disponible: Box<dyn Fn() -> bool>,
    en_attente: RefCell<Option<crate::llm::Msg>>,
}

impl Tool for Regarder {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "regarder".into(),
            description: "Regarde par la camera : capture une image de ce que \
                          l'utilisateur montre ou de la scene (l'image arrive \
                          au message suivant)."
                .into(),
            parameters: obj_schema(serde_json::json!({}), &[]),
        }
    }
    fn disponible(&self) -> bool {
        (self.disponible)()
    }
    fn run(&self, _args: &serde_json::Value) -> Result<String, String> {
        if !(self.disponible)() {
            return Err(echec_vue("camera", "la camera est eteinte"));
        }
        let img = (self.fournisseur)().map_err(|e| echec_vue("camera", &e))?;
        *self.en_attente.borrow_mut() = Some(crate::llm::Msg::UserImage {
            texte: "(capture caméra de l'outil regarder — réponds d'après cette image)"
                .into(),
            data_url: img.data_url,
        });
        Ok(format!(
            "image capturee ({}x{}), fournie dans le message suivant",
            img.largeur, img.hauteur
        ))
    }
    fn take_injection(&self) -> Option<crate::llm::Msg> {
        self.en_attente.borrow_mut().take()
    }
}

// ---------------------------------------------------------------------------
// Vision ÉCRAN (R5 chantier 2) — le pendant de `regarder` pour l'écran.
// L'hôte (mode « Écran » desktop, ch. 3) fournit la capture + l'OCR ; waly-core
// reste découplé de waly-sight (mêmes règles que la caméra).
// ---------------------------------------------------------------------------

/// Périmètre de capture d'écran (cadrage adaptatif du plan R5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CadrageEcran {
    /// Fenêtre au premier plan (défaut, plus privé, VLM moins cher).
    Fenetre,
    /// Écran principal entier (« regarde tout mon écran »).
    Plein,
}

/// Résultat d'une capture d'écran : l'image (jamais persistée) + le texte lu
/// par l'OCR local + sa confiance moyenne (0..1). L'OCR-first du chantier 1 :
/// le texte va au prompt, l'image seulement si la compréhension l'exige.
pub struct CaptureEcran {
    pub image: ImageCapturee,
    pub texte_ocr: String,
    pub conf: f32,
}

/// Fournisseur de capture d'écran branché par l'hôte (closure sur waly-sight).
pub type FournisseurEcran = Box<dyn Fn(CadrageEcran) -> Result<CaptureEcran, String>>;

/// Enregistre l'outil `regarder_ecran` (R5). À n'appeler QUE si une session de
/// partage d'écran est active (règle d'honnêteté : pas d'outil fantôme).
pub fn register_vision_ecran_tool(registry: &mut Registry, fournisseur: FournisseurEcran) {
    register_vision_ecran_tool_si(registry, fournisseur, Box::new(|| true));
}

/// Variante honnête : au catalogue seulement pendant un partage d'écran.
pub fn register_vision_ecran_tool_si(
    registry: &mut Registry,
    fournisseur: FournisseurEcran,
    disponible: Box<dyn Fn() -> bool>,
) {
    registry.register(Box::new(RegarderEcran { fournisseur, disponible, en_attente: RefCell::new(None) }));
}

/// Outil vision ÉCRAN : capture ce qui est affiché, joint l'image (message
/// suivant) et REND le texte OCR dans le résultat — le modèle a le texte exact
/// tout de suite, l'image pour le visuel. Jamais écrit sur disque.
struct RegarderEcran {
    fournisseur: FournisseurEcran,
    disponible: Box<dyn Fn() -> bool>,
    en_attente: RefCell<Option<crate::llm::Msg>>,
}

impl Tool for RegarderEcran {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "regarder_ecran".into(),
            description: "Regarde l'ecran de l'utilisateur : capture ce qui est \
                          affiche (tout=true pour tout l'ecran, defaut = fenetre \
                          active). Le texte lu et l'image arrivent ensuite."
                .into(),
            parameters: obj_schema(
                serde_json::json!({
                    "tout": {"type": "boolean", "description": "true = tout l'ecran, defaut = fenetre active"}
                }),
                &[],
            ),
        }
    }
    fn disponible(&self) -> bool {
        (self.disponible)()
    }
    fn run(&self, args: &serde_json::Value) -> Result<String, String> {
        if !(self.disponible)() {
            return Err(echec_vue("ecran", "aucun partage d'ecran en cours"));
        }
        let cadrage = if args.get("tout").and_then(|v| v.as_bool()).unwrap_or(false) {
            CadrageEcran::Plein
        } else {
            CadrageEcran::Fenetre
        };
        let cap = (self.fournisseur)(cadrage).map_err(|e| echec_vue("ecran", &e))?;
        *self.en_attente.borrow_mut() = Some(crate::llm::Msg::UserImage {
            texte: "(capture de l'ecran par l'outil regarder_ecran — reponds d'apres cette image et le texte lu)"
                .into(),
            data_url: cap.image.data_url,
        });
        let ocr = cap.texte_ocr.trim();
        Ok(if ocr.is_empty() {
            format!(
                "ecran capture ({}x{}) — peu ou pas de texte lisible, reponds d'apres l'image.",
                cap.image.largeur, cap.image.hauteur
            )
        } else {
            format!(
                "ecran capture ({}x{}). Texte lu a l'ecran (OCR) :\n{ocr}",
                cap.image.largeur, cap.image.hauteur
            )
        })
    }
    fn take_injection(&self) -> Option<crate::llm::Msg> {
        self.en_attente.borrow_mut().take()
    }
}

/// Charge une image PNG/JPEG du disque en `ImageCapturee` (fournisseur de
/// banc : env WALY_CAM_FAKE dans le bin ; le desktop branchera la vraie
/// caméra au chantier 5).
pub fn image_depuis_fichier(path: &str) -> Result<ImageCapturee, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("lecture {path}: {e}"))?;
    let mime = match bytes.get(..4) {
        Some([0x89, b'P', b'N', b'G']) => "image/png",
        Some([0xFF, 0xD8, ..]) => "image/jpeg",
        _ => return Err(format!("{path}: ni PNG ni JPEG")),
    };
    Ok(ImageCapturee {
        data_url: format!("data:{mime};base64,{}", base64(&bytes)),
        largeur: 0,
        hauteur: 0,
    })
}

/// Base64 standard avec padding (RFC 4648) — trivial, évite une dépendance.
pub fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Outil SENSIBLE de démo : exerce le flux d'approbation (HITL) en réel sans
/// rien envoyer nulle part. À enregistrer UNIQUEMENT si l'appelant le décide
/// (bin et desktop : env WALY_DEMO_SENSIBLE=1). Sera remplacé par la vraie
/// intégration messages. Monté du bin (2026-07-07) pour servir aussi le
/// desktop (cartes HITL du ch. 3).
pub struct EnvoyerMessageDemo;
impl Tool for EnvoyerMessageDemo {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "envoyer_message".into(),
            description: "Envoie un message a quelqu'un (destinataire, texte).".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "destinataire": {"type": "string"},
                    "texte": {"type": "string"},
                },
                "required": ["destinataire", "texte"],
            }),
        }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { irreversible: true, ..Default::default() }
    }
    fn run(&self, args: &serde_json::Value) -> Result<String, String> {
        Ok(format!(
            "SIMULATION - message envoye a {}: {}",
            args["destinataire"].as_str().unwrap_or("?"),
            args["texte"].as_str().unwrap_or("")
        ))
    }
}

/// Indexe une entité si l'embedder est là ; l'échec d'indexation ne fait
/// JAMAIS échouer l'écriture du souvenir (dégradation gracieuse).
fn try_index(emb: &SharedEmbedder, conn: &Connection, ty: &str, key: &str, text: &str) {
    let Some(e) = emb else { return };
    match e.borrow_mut().embed_passage(text) {
        Ok(v) => {
            if let Err(err) = store::index_entity(conn, ty, key, &embed::to_vec_json(&v)) {
                tracing::warn!("indexation {ty}/{key}: {err}");
            }
        }
        Err(err) => tracing::warn!("embedding {ty}/{key}: {err}"),
    }
}

fn obj_schema(props: serde_json::Value, required: &[&str]) -> serde_json::Value {
    serde_json::json!({"type": "object", "properties": props, "required": required})
}

fn format_memories(mems: &[store::Memory]) -> String {
    mems.iter()
        .map(|m| format!("[{}] {}: {}", m.category, m.key, m.value))
        .collect::<Vec<_>>()
        .join("\n")
}

fn s(v: &serde_json::Value, key: &str) -> Result<String, String> {
    match v.get(key) {
        Some(serde_json::Value::String(x)) if !x.trim().is_empty() => Ok(x.trim().to_owned()),
        _ => Err(format!("il manque {key}")),
    }
}

struct Heure;
impl Tool for Heure {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "heure".into(),
            description: "Date et heure locales actuelles.".into(),
            parameters: obj_schema(serde_json::json!({}), &[]),
        }
    }
    fn run(&self, _args: &serde_json::Value) -> Result<String, String> {
        Ok(clock::french_timestamp())
    }
}

struct Memoriser {
    conn: Rc<Connection>,
    emb: SharedEmbedder,
}
impl Tool for Memoriser {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "memoriser".into(),
            description: "Retient une info durable sur ton utilisateur (cle courte, valeur).".into(),
            parameters: obj_schema(
                serde_json::json!({
                    "categorie": {"type": "string", "enum": ["fact", "preference", "context", "event"]},
                    "cle": {"type": "string"},
                    "valeur": {"type": "string"},
                }),
                &["cle", "valeur"],
            ),
        }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { writes_memory: true, ..Default::default() }
    }
    fn run(&self, args: &serde_json::Value) -> Result<String, String> {
        let cle = s(args, "cle")?;
        let valeur = s(args, "valeur")?;
        let categorie = s(args, "categorie").unwrap_or_else(|_| "fact".into());
        store::upsert_memory(&self.conn, &categorie, &cle, &valeur, "declared")
            .map_err(|e| e.to_string())?;
        // Le texte indexé est « clé: valeur », comme l'ancien monde.
        try_index(&self.emb, &self.conn, "memory", &cle, &format!("{cle}: {valeur}"));
        Ok(format!("retenu ({categorie}): {cle}"))
    }
}

struct ChercherMemoire {
    conn: Rc<Connection>,
    emb: SharedEmbedder,
}
impl Tool for ChercherMemoire {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "chercher_memoire".into(),
            description: "Cherche dans les souvenirs retenus.".into(),
            parameters: obj_schema(
                serde_json::json!({"requete": {"type": "string"}}),
                &["requete"],
            ),
        }
    }
    fn run(&self, args: &serde_json::Value) -> Result<String, String> {
        let requete = s(args, "requete")?;
        // Rappel sémantique d'abord (hybride 0,7·cos + 0,3·mot-clé) : c'est
        // lui qui retrouve « ndolé » depuis « plat préféré ».
        if let Some(e) = &self.emb {
            if let Ok(qv) = e.borrow_mut().embed_query(&requete) {
                let found =
                    store::hybrid_search_memories(&self.conn, &requete, &embed::to_vec_json(&qv))
                        .map_err(|e| e.to_string())?;
                if !found.is_empty() {
                    // Top classé, pas filtré : le modèle ignore le hors-sujet.
                    return Ok(format!(
                        "souvenirs les plus proches (ignore ceux qui ne repondent pas) :\n{}",
                        format_memories(&found)
                    ));
                }
            }
        }
        let found = store::search_memories(&self.conn, &requete).map_err(|e| e.to_string())?;
        if !found.is_empty() {
            return Ok(format_memories(&found));
        }
        // Dernier recours porté de l'ancien monde (search_all_memories) : le
        // mot-clé rate souvent (accents, synonymes) et les souvenirs sont peu
        // nombreux → on rend TOUT et le modèle trie. La vraie réponse est la
        // recherche sémantique (chantier 4).
        let all = store::active_memories(&self.conn).map_err(|e| e.to_string())?;
        if all.is_empty() {
            return Ok("aucun souvenir retenu pour l'instant".into());
        }
        Ok(format!("rien ne correspond exactement ; tous les souvenirs :\n{}", format_memories(&all)))
    }
}

struct Oublier {
    conn: Rc<Connection>,
}
impl Tool for Oublier {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "oublier".into(),
            description: "Efface un souvenir par sa cle.".into(),
            parameters: obj_schema(serde_json::json!({"cle": {"type": "string"}}), &["cle"]),
        }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { writes_memory: true, ..Default::default() }
    }
    fn run(&self, args: &serde_json::Value) -> Result<String, String> {
        let cle = s(args, "cle")?;
        store::unindex_entity(&self.conn, "memory", &cle).map_err(|e| e.to_string())?;
        match store::forget_memory(&self.conn, &cle).map_err(|e| e.to_string())? {
            true => Ok(format!("oublie: {cle}")),
            false => Ok(format!("aucun souvenir nomme {cle}")),
        }
    }
}

struct CreerNote {
    conn: Rc<Connection>,
    emb: SharedEmbedder,
}
impl Tool for CreerNote {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "creer_note".into(),
            description: "Cree une note (titre, contenu).".into(),
            parameters: obj_schema(
                serde_json::json!({
                    "titre": {"type": "string"},
                    "contenu": {"type": "string"},
                }),
                &["titre", "contenu"],
            ),
        }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { writes_memory: true, ..Default::default() }
    }
    fn run(&self, args: &serde_json::Value) -> Result<String, String> {
        let titre = s(args, "titre")?;
        let contenu = s(args, "contenu")?;
        let id = store::create_note(&self.conn, &titre, &contenu, "[]")
            .map_err(|e| e.to_string())?;
        try_index(&self.emb, &self.conn, "note", &id.to_string(), &format!("{titre}: {contenu}"));
        Ok(format!("note {id} creee: {titre}"))
    }
}

struct ChercherNotes {
    conn: Rc<Connection>,
}
impl Tool for ChercherNotes {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "chercher_notes".into(),
            description: "Cherche dans les notes.".into(),
            parameters: obj_schema(
                serde_json::json!({"requete": {"type": "string"}}),
                &["requete"],
            ),
        }
    }
    fn run(&self, args: &serde_json::Value) -> Result<String, String> {
        let requete = s(args, "requete")?;
        let found = store::search_notes(&self.conn, &requete).map_err(|e| e.to_string())?;
        if found.is_empty() {
            return Ok("aucune note ne correspond".into());
        }
        Ok(found
            .iter()
            .map(|(id, titre, contenu)| format!("note {id} \u{ab} {titre} \u{bb}: {contenu}"))
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

struct ListerNotes {
    conn: Rc<Connection>,
}
impl Tool for ListerNotes {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "lister_notes".into(),
            description: "Liste les titres des notes.".into(),
            parameters: obj_schema(serde_json::json!({}), &[]),
        }
    }
    fn run(&self, _args: &serde_json::Value) -> Result<String, String> {
        let notes = store::list_notes(&self.conn).map_err(|e| e.to_string())?;
        if notes.is_empty() {
            return Ok("aucune note".into());
        }
        Ok(notes.iter().map(|(id, t)| format!("{id}. {t}")).collect::<Vec<_>>().join("\n"))
    }
}

/// Lit un id entier (le modèle peut l'émettre en nombre ou en chaîne).
fn id_arg(v: &serde_json::Value, key: &str) -> Result<i64, String> {
    match v.get(key) {
        Some(x) if x.is_i64() || x.is_u64() => Ok(x.as_i64().unwrap()),
        Some(serde_json::Value::String(s)) => {
            s.trim().parse().map_err(|_| format!("{key} n'est pas un nombre"))
        }
        _ => Err(format!("il manque {key}")),
    }
}

// ── Taches ──────────────────────────────────────────────────────────────────

struct CreerTache {
    conn: Rc<Connection>,
}
impl Tool for CreerTache {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "creer_tache".into(),
            description: "Ajoute une tache a faire. echeance=AAAA-MM-JJ (optionnelle).".into(),
            parameters: obj_schema(
                serde_json::json!({
                    "titre": {"type": "string"},
                    "priorite": {"type": "string", "enum": ["basse", "normale", "haute"]},
                    "echeance": {"type": "string"},
                }),
                &["titre"],
            ),
        }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { writes_memory: true, ..Default::default() }
    }
    fn run(&self, args: &serde_json::Value) -> Result<String, String> {
        let titre = s(args, "titre")?;
        let priorite = s(args, "priorite").unwrap_or_else(|_| "normale".into());
        let echeance = args.get("echeance").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty());
        let id = store::create_task(&self.conn, &titre, None, &priorite, echeance)
            .map_err(|e| e.to_string())?;
        Ok(format!("tache {id} creee: {titre}"))
    }
}

struct ListerTaches {
    conn: Rc<Connection>,
}
impl Tool for ListerTaches {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "lister_taches".into(),
            description: "Liste les taches en cours (urgentes d'abord).".into(),
            parameters: obj_schema(serde_json::json!({}), &[]),
        }
    }
    fn run(&self, _args: &serde_json::Value) -> Result<String, String> {
        let tasks = store::list_open_tasks(&self.conn).map_err(|e| e.to_string())?;
        if tasks.is_empty() {
            return Ok("aucune tache en cours".into());
        }
        Ok(tasks
            .iter()
            .map(|t| {
                let quand = t.due_date.as_deref().map(|d| format!(", pour le {d}")).unwrap_or_default();
                format!("{}. {} ({}{quand}) [{}]", t.id, t.title, t.priority, t.status)
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

struct MajTache {
    conn: Rc<Connection>,
}
impl Tool for MajTache {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "maj_tache".into(),
            description: "Change statut/priorite d'une tache (par numero).".into(),
            parameters: obj_schema(
                serde_json::json!({
                    "id": {"type": "integer"},
                    "statut": {"type": "string", "enum": ["a_faire", "en_cours", "faite", "annulee"]},
                    "priorite": {"type": "string", "enum": ["basse", "normale", "haute"]},
                }),
                &["id"],
            ),
        }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { writes_memory: true, ..Default::default() }
    }
    fn run(&self, args: &serde_json::Value) -> Result<String, String> {
        let id = id_arg(args, "id")?;
        let statut = args.get("statut").and_then(|v| v.as_str());
        let priorite = args.get("priorite").and_then(|v| v.as_str());
        if statut.is_none() && priorite.is_none() {
            return Err("indique un statut ou une priorite a changer".into());
        }
        match store::update_task(&self.conn, id, statut, priorite).map_err(|e| e.to_string())? {
            true => Ok(format!("tache {id} mise a jour")),
            false => Ok(format!("aucune tache numero {id}")),
        }
    }
}

// ── Rappels ─────────────────────────────────────────────────────────────────

struct PoserRappel {
    conn: Rc<Connection>,
}
impl Tool for PoserRappel {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "poser_rappel".into(),
            description: "Programme un rappel. quand='AAAA-MM-JJ HH:MM' (calcule la \
                          date depuis l'heure que tu connais)."
                .into(),
            parameters: obj_schema(
                serde_json::json!({
                    "titre": {"type": "string"},
                    "quand": {"type": "string"},
                    "recurrence": {"type": "string", "enum": ["quotidien", "hebdomadaire"]},
                }),
                &["titre", "quand"],
            ),
        }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { writes_memory: true, ..Default::default() }
    }
    fn run(&self, args: &serde_json::Value) -> Result<String, String> {
        let titre = s(args, "titre")?;
        let quand = s(args, "quand")?;
        let recurrence = args.get("recurrence").and_then(|v| v.as_str()).filter(|s| !s.is_empty());
        let id = store::create_reminder(&self.conn, &titre, None, &quand, recurrence)
            .map_err(|e| e.to_string())?;
        Ok(format!("rappel {id} pose pour {quand}: {titre}"))
    }
}

struct ListerRappels {
    conn: Rc<Connection>,
}
impl Tool for ListerRappels {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "lister_rappels".into(),
            description: "Liste les rappels a venir.".into(),
            parameters: obj_schema(serde_json::json!({}), &[]),
        }
    }
    fn run(&self, _args: &serde_json::Value) -> Result<String, String> {
        let rems = store::list_active_reminders(&self.conn).map_err(|e| e.to_string())?;
        if rems.is_empty() {
            return Ok("aucun rappel programme".into());
        }
        Ok(rems
            .iter()
            .map(|r| {
                let rec = r.recurrence.as_deref().map(|x| format!(" ({x})")).unwrap_or_default();
                format!("{}. {} le {}{rec}", r.id, r.title, r.remind_at)
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

struct AnnulerRappel {
    conn: Rc<Connection>,
}
impl Tool for AnnulerRappel {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "annuler_rappel".into(),
            description: "Annule un rappel par son numero.".into(),
            parameters: obj_schema(serde_json::json!({"id": {"type": "integer"}}), &["id"]),
        }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { writes_memory: true, ..Default::default() }
    }
    fn run(&self, args: &serde_json::Value) -> Result<String, String> {
        let id = id_arg(args, "id")?;
        match store::cancel_reminder(&self.conn, id).map_err(|e| e.to_string())? {
            true => Ok(format!("rappel {id} annule")),
            false => Ok(format!("aucun rappel actif numero {id}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ToolCall;
    use crate::safety::LoopGuard;
    use crate::tools::{Dispatched, TurnCtx};

    fn setup() -> (Registry, Rc<Connection>) {
        let conn = Rc::new(store::open(":memory:").unwrap());
        let mut r = Registry::new();
        register_core_tools(&mut r, conn.clone(), None);
        (r, conn)
    }

    fn dispatch(r: &Registry, name: &str, args: &str) -> Dispatched {
        let mut g = LoopGuard::new();
        let mut ctx = TurnCtx { user_message: "test", loop_guard: &mut g, approvals: None };
        r.dispatch(
            &ToolCall { id: "c1".into(), name: name.into(), arguments: args.into() },
            &mut ctx,
        )
    }

    #[test]
    fn cycle_memoire_via_dispatch() {
        let (r, _conn) = setup();
        assert!(matches!(
            dispatch(&r, "memoriser", r#"{"categorie":"preference","cle":"cafe","valeur":"noir sans sucre"}"#),
            Dispatched::Done(m) if m.contains("retenu")
        ));
        assert!(matches!(
            dispatch(&r, "chercher_memoire", r#"{"requete":"cafe"}"#),
            Dispatched::Done(m) if m.contains("noir sans sucre")
        ));
        assert!(matches!(
            dispatch(&r, "oublier", r#"{"cle":"cafe"}"#),
            Dispatched::Done(m) if m.contains("oublie")
        ));
        assert!(matches!(
            dispatch(&r, "chercher_memoire", r#"{"requete":"cafe"}"#),
            Dispatched::Done(m) if m.contains("aucun souvenir")
        ));
    }

    #[test]
    fn cycle_notes_via_dispatch() {
        let (r, _conn) = setup();
        assert!(matches!(
            dispatch(&r, "creer_note", r#"{"titre":"Courses","contenu":"pain, lait"}"#),
            Dispatched::Done(m) if m.contains("creee")
        ));
        assert!(matches!(
            dispatch(&r, "chercher_notes", r#"{"requete":"lait"}"#),
            Dispatched::Done(m) if m.contains("Courses")
        ));
        assert!(matches!(
            dispatch(&r, "lister_notes", "{}"),
            Dispatched::Done(m) if m.contains("Courses")
        ));
    }

    #[test]
    fn categorie_hallucinee_rejetee_par_l_enum() {
        let (r, _conn) = setup();
        assert!(matches!(
            dispatch(&r, "memoriser", r#"{"categorie":"important","cle":"x","valeur":"y"}"#),
            Dispatched::Rejected(_)
        ));
    }

    #[test]
    fn cycle_taches_via_dispatch() {
        let (r, _conn) = setup();
        assert!(matches!(
            dispatch(&r, "creer_tache", r#"{"titre":"Appeler le dentiste","priorite":"haute"}"#),
            Dispatched::Done(m) if m.contains("creee")
        ));
        assert!(matches!(
            dispatch(&r, "lister_taches", "{}"),
            Dispatched::Done(m) if m.contains("dentiste")
        ));
        assert!(matches!(
            dispatch(&r, "maj_tache", r#"{"id":1,"statut":"faite"}"#),
            Dispatched::Done(m) if m.contains("mise a jour")
        ));
        assert!(matches!(
            dispatch(&r, "lister_taches", "{}"),
            Dispatched::Done(m) if m.contains("aucune tache")
        ));
        // Statut halluciné rejeté par l'enum ; id en chaîne toléré.
        assert!(matches!(
            dispatch(&r, "maj_tache", r#"{"id":"1","statut":"urgent"}"#),
            Dispatched::Rejected(_)
        ));
    }

    #[test]
    fn cycle_rappels_via_dispatch() {
        let (r, _conn) = setup();
        assert!(matches!(
            dispatch(&r, "poser_rappel", r#"{"titre":"Sortir avec Paul","quand":"2027-01-01 18:00"}"#),
            Dispatched::Done(m) if m.contains("pose pour")
        ));
        assert!(matches!(
            dispatch(&r, "lister_rappels", "{}"),
            Dispatched::Done(m) if m.contains("Paul")
        ));
        assert!(matches!(
            dispatch(&r, "annuler_rappel", r#"{"id":1}"#),
            Dispatched::Done(m) if m.contains("annule")
        ));
        assert!(matches!(
            dispatch(&r, "lister_rappels", "{}"),
            Dispatched::Done(m) if m.contains("aucun rappel")
        ));
    }

    #[test]
    fn budget_prompt_des_outils_coeur() {
        let (r, _conn) = setup();
        assert_eq!(r.specs().len(), 13, "catalogue R2 = 13 outils natifs");
        let json = serde_json::to_string(
            &r.specs().iter().map(crate::llm::tool_to_json).collect::<Vec<_>>(),
        )
        .unwrap();
        // ~4 octets/token. 13 outils descriptifs restent sous ~1100 tokens de
        // bloc outils ; avec ~90 tokens de prompt système de base, le total
        // reste dans le budget R2 (< 1k avec souvenirs raisonnables, mesuré au
        // tokenizer FLM par `waly prompt-tokens`).
        assert!(json.len() < 4400, "bloc outils trop lourd: {} octets", json.len());
    }
}
