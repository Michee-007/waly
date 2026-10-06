//! Persistance SQLite (rusqlite bundled) + recherche vectorielle (sqlite-vec).
//!
//! Tout est lié statiquement dans notre binaire : sqlite3.c et sqlite-vec sont
//! du C compilé par le build-script (ELF côté WSL, hors de portée de SAC) —
//! aucune DLL tierce, aucun exe tiers.
//!
//! Mémoire portée de l'ancien monde (user_memory PGlite) :
//! - TTL par catégorie : fact=permanent, preference=90 j, context=30 j,
//!   event=7 j ;
//! - refresh-on-read : toute lecture remet le TTL et confidence=1.0 ;
//! - clé UNIQUE (UPSERT) ; source declared|inferred.
//! La recherche vectorielle (embeddings locaux, hybride 0,7/0,3) arrive au
//! chantier 4 — le schéma vec est posé, la recherche v1 est mot-clé.

use rusqlite::{params, Connection};

/// Ouvre une connexion avec sqlite-vec enregistré et le schéma migré.
/// Chemin de la base par défaut : `WALY_DB` › waly.toml `[stockage] base` ›
/// `C:\waly\data\waly.db`. Une seule vérité pour desktop, CLI et voix.
pub fn chemin_par_defaut() -> String {
    std::env::var("WALY_DB")
        .ok()
        .or_else(|| crate::config::valeur("stockage", "base"))
        .unwrap_or_else(|| crate::chemins::data_fichier("waly.db"))
}

pub fn open(path: &str) -> rusqlite::Result<Connection> {
    // Enregistrement AVANT l'ouverture : vec0 doit exister pour les CREATE
    // VIRTUAL TABLE des migrations.
    unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    }
    let conn = if path == ":memory:" {
        Connection::open_in_memory()?
    } else {
        Connection::open(path)?
    };
    // Deux processus ecrivent la meme base pendant le mode appel (desktop +
    // waly-voice compagnon, R4 ch. 5) : attendre le verrou plutot que
    // d'echouer SQLITE_BUSY sec. AVANT le passage en WAL : sur une base
    // neuve, deux ouvertures simultanees s'y heurtaient (vecu 2026-10-01).
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    // La Garde : registre de ce que Waly touche (voir garde.rs).
    crate::garde::migrer(conn)?;
    crate::enclos::migrer(conn)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS user_memory (
           key        TEXT PRIMARY KEY,
           category   TEXT NOT NULL CHECK(category IN ('fact','preference','context','event')),
           value      TEXT NOT NULL,
           confidence REAL NOT NULL DEFAULT 1.0,
           source     TEXT NOT NULL DEFAULT 'declared' CHECK(source IN ('declared','inferred')),
           expires_at TEXT,
           created_at TEXT NOT NULL DEFAULT (datetime('now')),
           updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE IF NOT EXISTS notes (
           note_id    INTEGER PRIMARY KEY,
           title      TEXT NOT NULL,
           content    TEXT NOT NULL,
           tags       TEXT NOT NULL DEFAULT '[]',
           created_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE IF NOT EXISTS conversations (
           id         INTEGER PRIMARY KEY,
           role       TEXT NOT NULL CHECK(role IN ('user','assistant')),
           content    TEXT NOT NULL,
           created_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         -- Sessions de conversation (R3 desktop). La session 1 est le
         -- « Fil principal » : la voix et le bin y ecrivent toujours (un
         -- seul compagnon, un seul fil par defaut) ; le desktop peut en
         -- creer d'autres. kind='agent' = session-agent (espace Agentique,
         -- ch. 3) avec un etat de travail affichable.
         CREATE TABLE IF NOT EXISTS chat_sessions (
           session_id   INTEGER PRIMARY KEY,
           title        TEXT NOT NULL DEFAULT 'Conversation',
           kind         TEXT NOT NULL DEFAULT 'chat' CHECK(kind IN ('chat','agent')),
           agent_status TEXT CHECK(agent_status IN ('travail','attente','fini','echec')),
           created_at   TEXT NOT NULL DEFAULT (datetime('now'))
         );
         -- Index sémantique : embedding_map porte l'identité (et le taint,
         -- exclu de la recherche — spec ancien monde), embedding_vec le
         -- vecteur (rowid partagé). e5-small : 384 dims, L2-normalisé.
         CREATE TABLE IF NOT EXISTS embedding_map (
           rowid       INTEGER PRIMARY KEY,
           entity_type TEXT NOT NULL CHECK(entity_type IN ('memory','note')),
           entity_key  TEXT NOT NULL,
           tainted     INTEGER NOT NULL DEFAULT 0,
           UNIQUE(entity_type, entity_key)
         );
         CREATE VIRTUAL TABLE IF NOT EXISTS embedding_vec USING vec0(
           embedding float[384]
         );
         -- Approbations humaines (HITL) portées de l'ancien monde :
         -- machine d'états pending -> resolving -> resolved|failed,
         -- pending -> expired|rejected. Claim ATOMIQUE (exactly-once),
         -- expiration 24 h balayée paresseusement.
         CREATE TABLE IF NOT EXISTS pending_approvals (
           id          INTEGER PRIMARY KEY,
           tool_name   TEXT NOT NULL,
           tool_args   TEXT NOT NULL,
           status      TEXT NOT NULL DEFAULT 'pending'
                       CHECK(status IN ('pending','resolving','resolved','failed','rejected','expired')),
           created_at  TEXT NOT NULL DEFAULT (datetime('now')),
           expires_at  TEXT NOT NULL DEFAULT (datetime('now', '+24 hours')),
           resolved_at TEXT
         );
         -- Taches (portage life-commitments/tasks de l'ancien monde).
         CREATE TABLE IF NOT EXISTS tasks (
           task_id    INTEGER PRIMARY KEY,
           title      TEXT NOT NULL,
           notes      TEXT,
           priority   TEXT NOT NULL DEFAULT 'normale'
                      CHECK(priority IN ('basse','normale','haute')),
           status     TEXT NOT NULL DEFAULT 'a_faire'
                      CHECK(status IN ('a_faire','en_cours','faite','annulee')),
           due_date   TEXT,
           created_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         -- Rappels : remind_at est un datetime SQLite ('YYYY-MM-DD HH:MM').
         -- 'declenche' est pose par due_reminders (balayage du service).
         -- Heure LOCALE (le modele calcule depuis l'horodatage local) :
         -- comparer a datetime('now','localtime') — l'UTC retardait de 2 h
         -- en ete (corrige 2026-09-30).
         CREATE TABLE IF NOT EXISTS reminders (
           reminder_id INTEGER PRIMARY KEY,
           title       TEXT NOT NULL,
           message     TEXT,
           remind_at   TEXT NOT NULL,
           recurrence  TEXT CHECK(recurrence IN ('quotidien','hebdomadaire')),
           status      TEXT NOT NULL DEFAULT 'actif'
                       CHECK(status IN ('actif','declenche','annule')),
           created_at  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         -- Journal visuel de session (R4.5 ch. 1) : la perception s'ecrit
         -- en TEXTE horodate — JAMAIS de pixels (regle vie privee R4).
         -- kind : 'evenement' (presence repliee par l'hysteresis),
         -- 'vu' (description d'un tour vision), 'moment' (description
         -- PROACTIVE — fort, dicible a voix haute), 'reflexion' (inference
         -- d'activite, ch. 3). created_at en HEURE LOCALE.
         CREATE TABLE IF NOT EXISTS visual_memory (
           id         INTEGER PRIMARY KEY,
           session_id INTEGER NOT NULL,
           kind       TEXT NOT NULL CHECK(kind IN ('evenement','vu','moment','reflexion')),
           content    TEXT NOT NULL,
           created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
         );
         CREATE INDEX IF NOT EXISTS idx_visual_memory_session
           ON visual_memory(session_id, id);
         -- Journal d'audit du sceau « Huis clos » (R6a) : pose/levee du sceau
         -- et chaque tentative de sortie reseau BLOQUEE (net event WFP). C'est
         -- la PREUVE consultable (one-pager conformite). Local, jamais exporte
         -- sans action explicite. 'cle' deduplique les tentatives (le service
         -- renvoie le ring buffer complet a chaque drain). at en HEURE LOCALE.
         CREATE TABLE IF NOT EXISTS audit_sceau (
           id         INTEGER PRIMARY KEY,
           session_id INTEGER NOT NULL,
           genre      TEXT NOT NULL CHECK(genre IN ('pose','levee','bloque','sortie')),
           detail     TEXT NOT NULL DEFAULT '',
           cle        TEXT UNIQUE,
           at         TEXT NOT NULL DEFAULT (datetime('now','localtime'))
         );
         CREATE INDEX IF NOT EXISTS idx_audit_sceau_session
           ON audit_sceau(session_id, id);
         -- Agents TIERS scelles par l'utilisateur via l'UI « Sceller un autre
         -- agent » (chantier C, Huis clos universel). Registre app-side : le
         -- service ne rend que des id de session (hash negatif), pas les
         -- chemins -> on garde ici {exe, session} pour lister/lever. 'session'
         -- = waly_seal::ipc::session_pour_exe(exe). Statut vivant recoupe avec
         -- l'etat du service. Aucune donnee sensible (juste un chemin d'exe).
         CREATE TABLE IF NOT EXISTS agents_scelles (
           exe        TEXT PRIMARY KEY,
           session    INTEGER NOT NULL,
           at         TEXT NOT NULL DEFAULT (datetime('now','localtime'))
         );",
    )?;
    // Migration additive : rattacher les conversations aux sessions.
    // ALTER TABLE n'est pas idempotent -> garde sur pragma_table_info.
    let has_session: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('conversations') WHERE name='session_id'")?
        .exists([])?;
    if !has_session {
        conn.execute_batch("ALTER TABLE conversations ADD COLUMN session_id INTEGER")?;
    }
    // kind/agent_status sont arrivés APRES chat_sessions (ch. 3) : les bases
    // créées entre les deux reçoivent les colonnes par ALTER.
    let has_kind: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('chat_sessions') WHERE name='kind'")?
        .exists([])?;
    if !has_kind {
        conn.execute_batch(
            "ALTER TABLE chat_sessions ADD COLUMN kind TEXT NOT NULL DEFAULT 'chat';
             ALTER TABLE chat_sessions ADD COLUMN agent_status TEXT",
        )?;
    }
    // UI 2026-09-14 (décision Michée) : plus de « Fil principal » imposé — la
    // voix écrit dans la conversation OUVERTE. La session 1 n'est plus créée
    // d'office ; une ancienne base la garde comme conversation ordinaire.
    conn.execute(
        "UPDATE chat_sessions SET title='Conversation vocale' WHERE session_id=1 AND title='Fil principal'",
        [],
    )?;
    // R4.5 ch. 3 : kinds 'moment'/'reflexion' — le CHECK de la v1 (ch. 1,
    // meme journee) ne les connait pas et SQLite ne modifie pas un CHECK :
    // rebatir la table si l'ancien schema est la (donnees copiees).
    let vm_sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='visual_memory'",
            [],
            |r| r.get(0),
        )
        .ok();
    if let Some(sql) = vm_sql {
        if !sql.contains("moment") {
            conn.execute_batch(
                "CREATE TABLE visual_memory_v2 (
                   id         INTEGER PRIMARY KEY,
                   session_id INTEGER NOT NULL,
                   kind       TEXT NOT NULL CHECK(kind IN ('evenement','vu','moment','reflexion')),
                   content    TEXT NOT NULL,
                   created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
                 );
                 INSERT INTO visual_memory_v2 SELECT * FROM visual_memory;
                 DROP TABLE visual_memory;
                 ALTER TABLE visual_memory_v2 RENAME TO visual_memory;
                 CREATE INDEX IF NOT EXISTS idx_visual_memory_session
                   ON visual_memory(session_id, id);",
            )?;
        }
    }
    // Lot 3 (2026-10-01) : genre 'sortie' = une sortie OUVERTE par
    // l'utilisateur (modèle extérieur, téléchargement) entre au journal du
    // sceau — même remède que ci-dessus pour l'ancien CHECK.
    let audit_sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='audit_sceau'",
            [],
            |r| r.get(0),
        )
        .ok();
    if audit_sql.is_some_and(|sql| !sql.contains("sortie")) {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE audit_sceau_v2 (
               id         INTEGER PRIMARY KEY,
               session_id INTEGER NOT NULL,
               genre      TEXT NOT NULL CHECK(genre IN ('pose','levee','bloque','sortie')),
               detail     TEXT NOT NULL DEFAULT '',
               cle        TEXT UNIQUE,
               at         TEXT NOT NULL DEFAULT (datetime('now','localtime'))
             );
             INSERT INTO audit_sceau_v2 SELECT id, session_id, genre, detail, cle, at FROM audit_sceau;
             DROP TABLE audit_sceau;
             ALTER TABLE audit_sceau_v2 RENAME TO audit_sceau;
             CREATE INDEX IF NOT EXISTS idx_audit_sceau_session
               ON audit_sceau(session_id, id);
             COMMIT;",
        )?;
    }
    // Les messages d'avant les sessions rejoignent le fil principal.
    conn.execute("UPDATE conversations SET session_id=1 WHERE session_id IS NULL", [])?;
    // Compétences apprises (2026-09-10) : la boucle d'auto-amélioration —
    // distillées après une mission réussie, réinjectées quand une mission
    // semblable arrive (competences.rs).
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS competences (
           key         TEXT PRIMARY KEY,
           titre       TEXT NOT NULL,
           declencheur TEXT NOT NULL,
           recette     TEXT NOT NULL,
           uses        INTEGER NOT NULL DEFAULT 0,
           created_at  TEXT NOT NULL DEFAULT (datetime('now')),
           last_used   TEXT
         );
         -- Fichiers créés dans une conversation (UI 2026-09-14) : la carte
         -- d'aperçu reste à sa place quand on rouvre la conversation.
         -- position = nombre de messages qui la précèdent.
         CREATE TABLE IF NOT EXISTS session_fichiers (
           id         INTEGER PRIMARY KEY,
           session_id INTEGER NOT NULL,
           chemin     TEXT NOT NULL,
           position   INTEGER NOT NULL,
           created_at TEXT NOT NULL DEFAULT (datetime('now')),
           UNIQUE(session_id, chemin)
         );
         -- Réglages de l'utilisateur (2026-09-30, lot 1 des « bientôt ») :
         -- personnalité/instructions, style de réponse. Clé -> valeur texte.
         CREATE TABLE IF NOT EXISTS reglages (
           cle        TEXT PRIMARY KEY,
           valeur     TEXT NOT NULL,
           updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         -- Journal des tours (Paramètres › Utilisation) : jamais le CONTENU,
         -- seulement des mesures. `estime` = 1 si le serveur n'a pas renvoyé
         -- `usage` (tokens de sortie estimés à ~4 caractères/token).
         CREATE TABLE IF NOT EXISTS tours (
           id                INTEGER PRIMARY KEY,
           session_id        INTEGER NOT NULL,
           modele            TEXT NOT NULL,
           prompt_tokens     INTEGER NOT NULL,
           completion_tokens INTEGER NOT NULL,
           duree_ms          INTEGER NOT NULL,
           premier_ms        INTEGER,
           estime            INTEGER NOT NULL DEFAULT 0,
           created_at        TEXT NOT NULL DEFAULT (datetime('now','localtime'))
         );
         -- Projets (lot 2, 2026-09-30) : un dossier de travail que Waly
         -- connaît — conversations/missions rattachées, instructions et
         -- fichiers de référence propres.
         CREATE TABLE IF NOT EXISTS projets (
           id           INTEGER PRIMARY KEY,
           nom          TEXT NOT NULL,
           instructions TEXT NOT NULL DEFAULT '',
           archive      INTEGER NOT NULL DEFAULT 0,
           created_at   TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE IF NOT EXISTS projet_fichiers (
           projet_id  INTEGER NOT NULL,
           chemin     TEXT NOT NULL,
           created_at TEXT NOT NULL DEFAULT (datetime('now')),
           UNIQUE(projet_id, chemin)
         );",
    )?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_resumes (
           session_id INTEGER PRIMARY KEY,
           resume     TEXT NOT NULL,
           jusqu_a    INTEGER NOT NULL,
           updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );",
    )?;
    // Réflexion approfondie (lot 3) : le raisonnement AFFICHÉ d'une réponse,
    // gardé pour la réouverture. position = rang du message de Waly.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_reflexions (
           id         INTEGER PRIMARY KEY,
           session_id INTEGER NOT NULL,
           position   INTEGER NOT NULL,
           texte      TEXT NOT NULL
         );",
    )?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS competence_versions (
           key        TEXT NOT NULL,
           version    INTEGER NOT NULL,
           recette    TEXT NOT NULL,
           raison     TEXT NOT NULL DEFAULT '',
           created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
           PRIMARY KEY(key, version)
         );",
    )?;
    // Missions programmées (lot 2) : un objectif lancé en MISSION à l'heure
    // dite, par l'app ouverte. Heure LOCALE comme les rappels.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS missions_programmees (
           id          INTEGER PRIMARY KEY,
           objectif    TEXT NOT NULL,
           prochain    TEXT NOT NULL,
           recurrence  TEXT CHECK(recurrence IN ('quotidien','hebdomadaire','ouvres')),
           projet_id   INTEGER,
           actif       INTEGER NOT NULL DEFAULT 1,
           derniere_session INTEGER,
           derniere_fois    TEXT,
           created_at  TEXT NOT NULL DEFAULT (datetime('now','localtime'))
         );",
    )?;
    let has_projet: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('chat_sessions') WHERE name='projet_id'")?
        .exists([])?;
    if !has_projet {
        conn.execute_batch("ALTER TABLE chat_sessions ADD COLUMN projet_id INTEGER")?;
    }
    Ok(())
}

// ── Missions programmées ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MissionProgrammee {
    pub id: i64,
    pub objectif: String,
    pub prochain: String,
    pub recurrence: Option<String>,
    pub projet_id: Option<i64>,
    pub derniere_session: Option<i64>,
    pub derniere_fois: Option<String>,
}

pub fn create_mission_programmee(
    conn: &Connection,
    objectif: &str,
    prochain: &str,
    recurrence: Option<&str>,
    projet_id: Option<i64>,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO missions_programmees(objectif, prochain, recurrence, projet_id) VALUES (?1, ?2, ?3, ?4)",
        params![objectif.trim(), prochain, recurrence, projet_id],
    )?;
    Ok(conn.last_insert_rowid())
}

fn mission_de(r: &rusqlite::Row) -> rusqlite::Result<MissionProgrammee> {
    Ok(MissionProgrammee {
        id: r.get(0)?,
        objectif: r.get(1)?,
        prochain: r.get(2)?,
        recurrence: r.get(3)?,
        projet_id: r.get(4)?,
        derniere_session: r.get(5)?,
        derniere_fois: r.get(6)?,
    })
}

const MISSION_COLS: &str = "id, objectif, prochain, recurrence, projet_id, derniere_session, derniere_fois";

/// Missions programmées actives, la plus proche d'abord.
pub fn list_missions_programmees(conn: &Connection) -> rusqlite::Result<Vec<MissionProgrammee>> {
    conn.prepare(&format!(
        "SELECT {MISSION_COLS} FROM missions_programmees WHERE actif=1 ORDER BY prochain"
    ))?
    .query_map([], mission_de)?
    .collect()
}

pub fn cancel_mission_programmee(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    Ok(conn.execute("UPDATE missions_programmees SET actif=0 WHERE id=?1 AND actif=1", params![id])? > 0)
}

/// Note la session où une mission programmée a tourné.
pub fn mission_programmee_lancee(conn: &Connection, id: i64, session: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE missions_programmees SET derniere_session=?2, derniere_fois=datetime('now','localtime') WHERE id=?1",
        params![id, session],
    )?;
    Ok(())
}

/// Missions ÉCHUES à lancer (heure locale). Même contrat que
/// [`due_reminders`] : chacune n'est rendue qu'UNE fois par échéance —
/// ponctuelle désactivée, récurrente avancée au prochain créneau FUTUR
/// (une app fermée trois jours ne rejoue pas trois fois la mission).
/// `ouvres` = du lundi au vendredi.
pub fn due_missions_programmees(conn: &Connection) -> rusqlite::Result<Vec<MissionProgrammee>> {
    let due: Vec<MissionProgrammee> = conn
        .prepare(&format!(
            "SELECT {MISSION_COLS} FROM missions_programmees
             WHERE actif=1 AND prochain <= datetime('now','localtime') ORDER BY prochain"
        ))?
        .query_map([], mission_de)?
        .collect::<Result<_, _>>()?;
    for m in &due {
        let step = match m.recurrence.as_deref() {
            Some("quotidien") | Some("ouvres") => "+1 day",
            Some("hebdomadaire") => "+7 days",
            _ => {
                conn.execute("UPDATE missions_programmees SET actif=0 WHERE id=?1", params![m.id])?;
                continue;
            }
        };
        for _ in 0..4000 {
            conn.execute(
                &format!("UPDATE missions_programmees SET prochain=datetime(prochain,'{step}') WHERE id=?1"),
                params![m.id],
            )?;
            // strftime('%w') : 0 = dimanche, 6 = samedi.
            let (futur, weekend): (bool, bool) = conn.query_row(
                "SELECT prochain > datetime('now','localtime'), strftime('%w', prochain) IN ('0','6')
                 FROM missions_programmees WHERE id=?1",
                params![m.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if futur && !(m.recurrence.as_deref() == Some("ouvres") && weekend) {
                break;
            }
        }
    }
    Ok(due)
}

// ── Projets ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Projet {
    pub id: i64,
    pub nom: String,
    pub instructions: String,
    pub archive: bool,
    pub sessions: i64,
    pub fichiers: Vec<String>,
}

pub fn create_projet(conn: &Connection, nom: &str) -> rusqlite::Result<i64> {
    conn.execute("INSERT INTO projets(nom) VALUES (?1)", params![nom.trim()])?;
    Ok(conn.last_insert_rowid())
}

pub fn projet(conn: &Connection, id: i64) -> rusqlite::Result<Option<Projet>> {
    let row = conn.query_row(
        "SELECT p.id, p.nom, p.instructions, p.archive,
                (SELECT COUNT(*) FROM chat_sessions s WHERE s.projet_id = p.id)
         FROM projets p WHERE p.id = ?1",
        params![id],
        |r| {
            Ok(Projet {
                id: r.get(0)?,
                nom: r.get(1)?,
                instructions: r.get(2)?,
                archive: r.get::<_, i64>(3)? != 0,
                sessions: r.get(4)?,
                fichiers: vec![],
            })
        },
    );
    match row {
        Ok(mut p) => {
            p.fichiers = projet_fichiers(conn, id)?;
            Ok(Some(p))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Tous les projets, actifs d'abord, le plus récemment actif en tête.
pub fn list_projets(conn: &Connection) -> rusqlite::Result<Vec<Projet>> {
    let ids: Vec<i64> = conn
        .prepare(
            "SELECT p.id FROM projets p ORDER BY p.archive,
               COALESCE((SELECT MAX(c.id) FROM conversations c JOIN chat_sessions s
                         ON c.session_id = s.session_id WHERE s.projet_id = p.id), 0) DESC,
               p.id DESC",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for id in ids {
        if let Some(p) = projet(conn, id)? {
            out.push(p);
        }
    }
    Ok(out)
}

pub fn update_projet(conn: &Connection, id: i64, nom: &str, instructions: &str, archive: bool) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE projets SET nom=?2, instructions=?3, archive=?4 WHERE id=?1",
        params![id, nom.trim(), instructions.trim(), archive as i64],
    )?;
    Ok(())
}

/// Supprime le projet ; ses conversations RESTENT (détachées) — on ne perd
/// jamais une conversation en rangeant.
pub fn delete_projet(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE chat_sessions SET projet_id=NULL WHERE projet_id=?1", params![id])?;
    tx.execute("DELETE FROM projet_fichiers WHERE projet_id=?1", params![id])?;
    tx.execute("DELETE FROM projets WHERE id=?1", params![id])?;
    tx.commit()
}

pub fn set_session_projet(conn: &Connection, session: i64, projet: Option<i64>) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE chat_sessions SET projet_id=?2 WHERE session_id=?1",
        params![session, projet],
    )?;
    Ok(())
}

pub fn session_projet(conn: &Connection, session: i64) -> Option<i64> {
    conn.query_row(
        "SELECT projet_id FROM chat_sessions WHERE session_id=?1",
        params![session],
        |r| r.get::<_, Option<i64>>(0),
    )
    .ok()
    .flatten()
}

pub fn sessions_du_projet(conn: &Connection, projet: i64) -> rusqlite::Result<Vec<SessionRow>> {
    let mut stmt = conn.prepare(
        "SELECT s.session_id, s.title, s.agent_status, s.kind FROM chat_sessions s
         WHERE s.projet_id = ?1
         ORDER BY COALESCE(
           (SELECT MAX(c.id) FROM conversations c WHERE c.session_id = s.session_id),
           s.session_id) DESC",
    )?;
    let rows = stmt
        .query_map(params![projet], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<Vec<_>, _>>();
    rows
}

pub fn add_projet_fichier(conn: &Connection, projet: i64, chemin: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO projet_fichiers(projet_id, chemin) VALUES (?1, ?2)",
        params![projet, chemin],
    )?;
    Ok(())
}

pub fn remove_projet_fichier(conn: &Connection, projet: i64, chemin: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM projet_fichiers WHERE projet_id=?1 AND chemin=?2",
        params![projet, chemin],
    )?;
    Ok(())
}

pub fn projet_fichiers(conn: &Connection, projet: i64) -> rusqlite::Result<Vec<String>> {
    conn.prepare("SELECT chemin FROM projet_fichiers WHERE projet_id=?1 ORDER BY created_at")?
        .query_map(params![projet], |r| r.get(0))?
        .collect()
}

// ── Réglages de l'utilisateur ───────────────────────────────────────────────

pub fn reglage(conn: &Connection, cle: &str) -> Option<String> {
    conn.query_row("SELECT valeur FROM reglages WHERE cle=?1", params![cle], |r| r.get(0))
        .ok()
}

/// Pose un réglage ; une valeur vide (après trim) l'efface.
pub fn set_reglage(conn: &Connection, cle: &str, valeur: &str) -> rusqlite::Result<()> {
    if valeur.trim().is_empty() {
        conn.execute("DELETE FROM reglages WHERE cle=?1", params![cle])?;
    } else {
        conn.execute(
            "INSERT INTO reglages(cle, valeur) VALUES (?1, ?2)
             ON CONFLICT(cle) DO UPDATE SET valeur=?2, updated_at=datetime('now')",
            params![cle, valeur.trim()],
        )?;
    }
    Ok(())
}

// ── Journal des tours (Utilisation) ─────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Tour {
    pub session_id: i64,
    pub modele: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub duree_ms: i64,
    pub premier_ms: Option<i64>,
    pub estime: bool,
}

pub fn add_tour(conn: &Connection, t: &Tour) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO tours(session_id, modele, prompt_tokens, completion_tokens, duree_ms, premier_ms, estime)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            t.session_id,
            t.modele,
            t.prompt_tokens,
            t.completion_tokens,
            t.duree_ms,
            t.premier_ms,
            t.estime as i64
        ],
    )?;
    Ok(())
}

/// Agrégats par modèle sur les `jours` derniers jours : (modèle, tours,
/// tokens d'entrée, tokens de sortie, durée moyenne ms, premier mot moyen ms,
/// au moins une valeur estimée).
pub fn utilisation_par_modele(
    conn: &Connection,
    jours: i64,
) -> rusqlite::Result<Vec<(String, i64, i64, i64, i64, Option<i64>, bool)>> {
    let mut st = conn.prepare(
        "SELECT modele, COUNT(*), SUM(prompt_tokens), SUM(completion_tokens),
                CAST(AVG(duree_ms) AS INTEGER), CAST(AVG(premier_ms) AS INTEGER), MAX(estime)
         FROM tours WHERE created_at >= datetime('now','localtime', ?1)
         GROUP BY modele ORDER BY COUNT(*) DESC",
    )?;
    let rows = st.query_map(params![format!("-{jours} days")], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get::<_, i64>(6)? != 0,
        ))
    })?;
    rows.collect()
}

/// Les `n` derniers tours, le plus récent d'abord : (heure, tour).
pub fn derniers_tours(conn: &Connection, n: i64) -> rusqlite::Result<Vec<(String, Tour)>> {
    let mut st = conn.prepare(
        "SELECT created_at, session_id, modele, prompt_tokens, completion_tokens, duree_ms, premier_ms, estime
         FROM tours ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = st.query_map(params![n], |r| {
        Ok((
            r.get(0)?,
            Tour {
                session_id: r.get(1)?,
                modele: r.get(2)?,
                prompt_tokens: r.get(3)?,
                completion_tokens: r.get(4)?,
                duree_ms: r.get(5)?,
                premier_ms: r.get(6)?,
                estime: r.get::<_, i64>(7)? != 0,
            },
        ))
    })?;
    rows.collect()
}

// ── Compétences apprises ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct CompetenceRow {
    pub key: String,
    pub titre: String,
    pub declencheur: String,
    pub recette: String,
    pub uses: i64,
}

pub fn upsert_competence(
    conn: &Connection,
    key: &str,
    titre: &str,
    declencheur: &str,
    recette: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO competences(key, titre, declencheur, recette) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(key) DO UPDATE SET titre=?2, declencheur=?3, recette=?4",
        params![key, titre, declencheur, recette],
    )?;
    Ok(())
}

/// Toutes les compétences, la plus utilisée d'abord (le catalogue reste
/// petit : des dizaines, pas des milliers — un tri suffit).
pub fn list_competences(conn: &Connection) -> rusqlite::Result<Vec<CompetenceRow>> {
    let mut stmt = conn.prepare(
        "SELECT key, titre, declencheur, recette, uses FROM competences
         ORDER BY uses DESC, created_at DESC LIMIT 200",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(CompetenceRow {
                key: r.get(0)?,
                titre: r.get(1)?,
                declencheur: r.get(2)?,
                recette: r.get(3)?,
                uses: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>();
    rows
}

pub fn get_competence(conn: &Connection, key: &str) -> rusqlite::Result<Option<CompetenceRow>> {
    conn.query_row(
        "SELECT key, titre, declencheur, recette, uses FROM competences WHERE key=?1",
        params![key],
        |r| {
            Ok(CompetenceRow {
                key: r.get(0)?,
                titre: r.get(1)?,
                declencheur: r.get(2)?,
                recette: r.get(3)?,
                uses: r.get(4)?,
            })
        },
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

/// Marque un usage (compteur + horodatage) — la vie d'une compétence.
pub fn touch_competence(conn: &Connection, key: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE competences SET uses = uses + 1, last_used = datetime('now') WHERE key=?1",
        params![key],
    )?;
    Ok(())
}

/// Une version passée d'une compétence (lot 2 : « compétences qui
/// s'améliorent » — l'historique garde chaque version).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct VersionCompetence {
    pub version: i64,
    pub recette: String,
    pub raison: String,
    pub created_at: String,
}

/// Numéro de la version COURANTE (= versions archivées + 1).
pub fn version_competence(conn: &Connection, key: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) + 1 FROM competence_versions WHERE key=?1",
        params![key],
        |r| r.get(0),
    )
}

/// Remplace la recette en ARCHIVANT l'actuelle (jamais de perte). `raison`
/// dit pourquoi l'ancienne a été remplacée.
pub fn reviser_competence(conn: &Connection, key: &str, recette: &str, raison: &str) -> rusqlite::Result<bool> {
    let Some(c) = get_competence(conn, key)? else { return Ok(false) };
    let tx = conn.unchecked_transaction()?;
    let v: i64 = tx.query_row(
        "SELECT COUNT(*) + 1 FROM competence_versions WHERE key=?1",
        params![key],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT INTO competence_versions(key, version, recette, raison) VALUES (?1, ?2, ?3, ?4)",
        params![key, v, c.recette, raison],
    )?;
    tx.execute("UPDATE competences SET recette=?2 WHERE key=?1", params![key, recette.trim()])?;
    tx.commit()?;
    Ok(true)
}

/// Versions archivées, la plus récente d'abord.
pub fn versions_competence(conn: &Connection, key: &str) -> rusqlite::Result<Vec<VersionCompetence>> {
    conn.prepare(
        "SELECT version, recette, raison, created_at FROM competence_versions
         WHERE key=?1 ORDER BY version DESC",
    )?
    .query_map(params![key], |r| {
        Ok(VersionCompetence { version: r.get(0)?, recette: r.get(1)?, raison: r.get(2)?, created_at: r.get(3)? })
    })?
    .collect()
}

/// Revient à une version passée — l'actuelle est archivée à son tour.
pub fn restaurer_competence(conn: &Connection, key: &str, version: i64) -> rusqlite::Result<bool> {
    let recette: Option<String> = conn
        .query_row(
            "SELECT recette FROM competence_versions WHERE key=?1 AND version=?2",
            params![key, version],
            |r| r.get(0),
        )
        .ok();
    match recette {
        Some(r) => reviser_competence(conn, key, &r, &format!("retour à la version {version}")),
        None => Ok(false),
    }
}

pub fn delete_competence(conn: &Connection, key: &str) -> rusqlite::Result<bool> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM competence_versions WHERE key=?1", params![key])?;
    let n = tx.execute("DELETE FROM competences WHERE key=?1", params![key])?;
    tx.commit()?;
    Ok(n > 0)
}

// ── Taches ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub priority: String,
    pub status: String,
    pub due_date: Option<String>,
}

pub fn create_task(
    conn: &Connection,
    title: &str,
    notes: Option<&str>,
    priority: &str,
    due_date: Option<&str>,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO tasks(title, notes, priority, due_date) VALUES (?1, ?2, ?3, ?4)",
        params![title, notes, priority, due_date],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Taches non terminees (a_faire / en_cours), les plus urgentes d'abord
/// (echeance NULL en dernier, puis priorite).
pub fn list_open_tasks(conn: &Connection) -> rusqlite::Result<Vec<Task>> {
    let mut stmt = conn.prepare(
        "SELECT task_id, title, priority, status, due_date FROM tasks
         WHERE status IN ('a_faire','en_cours')
         ORDER BY due_date IS NULL, due_date,
           CASE priority WHEN 'haute' THEN 0 WHEN 'normale' THEN 1 ELSE 2 END
         LIMIT 50",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Task {
                id: r.get(0)?,
                title: r.get(1)?,
                priority: r.get(2)?,
                status: r.get(3)?,
                due_date: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>();
    rows
}

/// Met a jour statut et/ou priorite d'une tache. Retourne false si l'id
/// n'existe pas.
pub fn update_task(
    conn: &Connection,
    id: i64,
    status: Option<&str>,
    priority: Option<&str>,
) -> rusqlite::Result<bool> {
    let mut changed = false;
    if let Some(s) = status {
        changed |= conn.execute("UPDATE tasks SET status=?2 WHERE task_id=?1", params![id, s])? > 0;
    }
    if let Some(p) = priority {
        changed |=
            conn.execute("UPDATE tasks SET priority=?2 WHERE task_id=?1", params![id, p])? > 0;
    }
    Ok(changed)
}

// ── Rappels ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Reminder {
    pub id: i64,
    pub title: String,
    pub message: Option<String>,
    pub remind_at: String,
    pub recurrence: Option<String>,
}

pub fn create_reminder(
    conn: &Connection,
    title: &str,
    message: Option<&str>,
    remind_at: &str,
    recurrence: Option<&str>,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO reminders(title, message, remind_at, recurrence) VALUES (?1, ?2, ?3, ?4)",
        params![title, message, remind_at, recurrence],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Rappels actifs (a venir), du plus proche au plus lointain.
pub fn list_active_reminders(conn: &Connection) -> rusqlite::Result<Vec<Reminder>> {
    let mut stmt = conn.prepare(
        "SELECT reminder_id, title, message, remind_at, recurrence FROM reminders
         WHERE status='actif' ORDER BY remind_at LIMIT 50",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Reminder {
                id: r.get(0)?,
                title: r.get(1)?,
                message: r.get(2)?,
                remind_at: r.get(3)?,
                recurrence: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>();
    rows
}

pub fn cancel_reminder(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "UPDATE reminders SET status='annule' WHERE reminder_id=?1 AND status='actif'",
        params![id],
    )? > 0)
}

/// Rappels ÉCHUS à consommer (le service les énonce). Effet de bord : chaque
/// rappel rendu est soit reprogrammé (récurrent : +1 jour / +7 jours) soit
/// marqué 'declenche' — un rappel n'est donc énoncé qu'UNE fois par échéance.
pub fn due_reminders(conn: &Connection) -> rusqlite::Result<Vec<Reminder>> {
    let mut stmt = conn.prepare(
        "SELECT reminder_id, title, message, remind_at, recurrence FROM reminders
         WHERE status='actif' AND remind_at <= datetime('now','localtime') ORDER BY remind_at",
    )?;
    let due: Vec<Reminder> = stmt
        .query_map([], |r| {
            Ok(Reminder {
                id: r.get(0)?,
                title: r.get(1)?,
                message: r.get(2)?,
                remind_at: r.get(3)?,
                recurrence: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for rem in &due {
        // Récurrent : avancer au PROCHAIN créneau FUTUR (un rappel manqué
        // depuis des jours ne doit pas se déclencher une fois par jour d'un
        // coup — il rattrape en un saut). Ponctuel : marqué déclenché.
        let step = match rem.recurrence.as_deref() {
            Some("quotidien") => "+1 day",
            Some("hebdomadaire") => "+7 days",
            _ => {
                conn.execute(
                    "UPDATE reminders SET status='declenche' WHERE reminder_id=?1",
                    params![rem.id],
                )?;
                continue;
            }
        };
        // Le prochain multiple de l'intervalle strictement au-dessus de now.
        // (Boucle bornée : SQLite n'a pas de boucle native ; les rappels sont
        // peu nombreux et l'écart en périodes reste modeste.)
        for _ in 0..4000 {
            conn.execute(
                &format!("UPDATE reminders SET remind_at=datetime(remind_at,'{step}') WHERE reminder_id=?1"),
                params![rem.id],
            )?;
            let future: bool = conn.query_row(
                "SELECT remind_at > datetime('now','localtime') FROM reminders WHERE reminder_id=?1",
                params![rem.id],
                |r| r.get(0),
            )?;
            if future {
                break;
            }
        }
    }
    Ok(due)
}

// ── Approbations ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Pending {
    pub id: i64,
    pub tool_name: String,
    pub tool_args: String,
}

/// Crée une demande d'approbation ; retourne son id (montré à l'utilisateur).
pub fn create_pending(conn: &Connection, tool_name: &str, tool_args: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO pending_approvals(tool_name, tool_args) VALUES (?1, ?2)",
        params![tool_name, tool_args],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Balaye les expirations (paresseux — appelé avant toute lecture/claim).
fn sweep_expired(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE pending_approvals SET status='expired'
         WHERE status='pending' AND expires_at <= datetime('now')",
        [],
    )?;
    Ok(())
}

/// Demandes encore en attente (injectées dans le prompt du tour).
pub fn list_pending(conn: &Connection) -> rusqlite::Result<Vec<Pending>> {
    sweep_expired(conn)?;
    let mut stmt = conn.prepare(
        "SELECT id, tool_name, tool_args FROM pending_approvals
         WHERE status='pending' ORDER BY id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Pending { id: r.get(0)?, tool_name: r.get(1)?, tool_args: r.get(2)? })
        })?
        .collect::<Result<Vec<_>, _>>();
    rows
}

/// Claim ATOMIQUE : pending -> resolving. Retourne l'action à exécuter, ou
/// None si l'id n'est pas claimable (inconnu, expiré, déjà résolu — un
/// double « oui » ne ré-exécute JAMAIS).
pub fn claim_pending(conn: &Connection, id: i64) -> rusqlite::Result<Option<Pending>> {
    sweep_expired(conn)?;
    let n = conn.execute(
        "UPDATE pending_approvals SET status='resolving'
         WHERE id=?1 AND status='pending' AND expires_at > datetime('now')",
        params![id],
    )?;
    if n == 0 {
        return Ok(None);
    }
    conn.query_row(
        "SELECT id, tool_name, tool_args FROM pending_approvals WHERE id=?1",
        params![id],
        |r| Ok(Pending { id: r.get(0)?, tool_name: r.get(1)?, tool_args: r.get(2)? }),
    )
    .map(Some)
}

/// Issue d'une exécution claimée : resolved (succès) ou failed.
pub fn finish_pending(conn: &Connection, id: i64, success: bool) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE pending_approvals
         SET status=?2, resolved_at=datetime('now') WHERE id=?1 AND status='resolving'",
        params![id, if success { "resolved" } else { "failed" }],
    )?;
    Ok(())
}

/// Refus explicite de l'utilisateur : pending -> rejected.
pub fn reject_pending(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    sweep_expired(conn)?;
    Ok(conn.execute(
        "UPDATE pending_approvals SET status='rejected', resolved_at=datetime('now')
         WHERE id=?1 AND status='pending'",
        params![id],
    )? > 0)
}

// ── Index sémantique ────────────────────────────────────────────────────────

/// (Ré)indexe une entité : remplace son vecteur si elle en avait un.
pub fn index_entity(
    conn: &Connection,
    entity_type: &str,
    entity_key: &str,
    vec_json: &str,
) -> rusqlite::Result<()> {
    unindex_entity(conn, entity_type, entity_key)?;
    conn.execute(
        "INSERT INTO embedding_map(entity_type, entity_key) VALUES (?1, ?2)",
        params![entity_type, entity_key],
    )?;
    let rowid = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO embedding_vec(rowid, embedding) VALUES (?1, ?2)",
        params![rowid, vec_json],
    )?;
    Ok(())
}

pub fn unindex_entity(
    conn: &Connection,
    entity_type: &str,
    entity_key: &str,
) -> rusqlite::Result<()> {
    let rowid: Option<i64> = conn
        .query_row(
            "SELECT rowid FROM embedding_map WHERE entity_type=?1 AND entity_key=?2",
            params![entity_type, entity_key],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    if let Some(id) = rowid {
        conn.execute("DELETE FROM embedding_vec WHERE rowid=?1", params![id])?;
        conn.execute("DELETE FROM embedding_map WHERE rowid=?1", params![id])?;
    }
    Ok(())
}

/// Plus proches voisins d'une requête (cosinus, vecteurs L2-normalisés :
/// cos = 1 - d²/2 depuis la distance L2 de vec0). Taintés exclus.
pub fn semantic_neighbors(
    conn: &Connection,
    entity_type: &str,
    query_vec_json: &str,
    k: usize,
) -> rusqlite::Result<Vec<(String, f32)>> {
    let mut stmt = conn.prepare(
        "SELECT m.entity_key, v.distance FROM embedding_vec v
         JOIN embedding_map m ON m.rowid = v.rowid
         WHERE v.embedding MATCH ?1 AND v.k = ?2
           AND m.entity_type = ?3 AND m.tainted = 0
         ORDER BY v.distance",
    )?;
    let rows = stmt
        .query_map(params![query_vec_json, k as i64, entity_type], |r| {
            let key: String = r.get(0)?;
            let d: f64 = r.get(1)?;
            Ok((key, 1.0 - (d * d / 2.0) as f32))
        })?
        .collect::<Result<Vec<_>, _>>();
    rows
}

/// Plancher de cohérence (cosinus) — PAS un filtre de pertinence : mesuré au
/// banc e5-small le 2026-07-05, les cosinus FR sont compressés (0,78-0,86,
/// même pour une requête sans réponse) ; un seuil absolu ne sépare rien mais
/// le CLASSEMENT est fiable (5/5 bonnes paires en tête). Design retenu :
/// plancher bas + top 3 classé, le modèle trie. (Le 0,75 de l'ancien monde
/// était calibré voyage-3, non transposable.)
pub const SEMANTIC_MIN_COS: f32 = 0.78;

/// Recherche hybride portée : candidats vectoriels ≥ plancher, boost
/// mot-clé, score final 0,7·cos + 0,3·(mot-clé ? 1 : 0), top 3 classé,
/// refresh-on-read.
pub fn hybrid_search_memories(
    conn: &Connection,
    query: &str,
    query_vec_json: &str,
) -> rusqlite::Result<Vec<Memory>> {
    let neighbors = semantic_neighbors(conn, "memory", query_vec_json, 20)?;
    let pattern = query.to_lowercase();
    let mut scored: Vec<(f32, Memory)> = Vec::new();
    for (key, cos) in neighbors {
        if cos < SEMANTIC_MIN_COS {
            continue;
        }
        let found: Option<Memory> = conn
            .query_row(
                "SELECT key, category, value, confidence FROM user_memory
                 WHERE key = ?1 AND (expires_at IS NULL OR expires_at > datetime('now'))",
                params![key],
                |r| {
                    Ok(Memory {
                        key: r.get(0)?,
                        category: r.get(1)?,
                        value: r.get(2)?,
                        confidence: r.get(3)?,
                    })
                },
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        let Some(m) = found else { continue };
        let keyword_hit = m.key.to_lowercase().contains(&pattern)
            || m.value.to_lowercase().contains(&pattern);
        let score = 0.7 * cos + 0.3 * if keyword_hit { 1.0 } else { 0.0 };
        scored.push((score, m));
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(3);
    let found: Vec<Memory> = scored.into_iter().map(|(_, m)| m).collect();
    for m in &found {
        refresh_on_read(conn, &m.key, &m.category)?;
    }
    Ok(found)
}

/// TTL en jours par catégorie de souvenir (None = permanent).
pub fn ttl_days(category: &str) -> Option<u32> {
    match category {
        "preference" => Some(90),
        "context" => Some(30),
        "event" => Some(7),
        _ => None, // fact
    }
}

fn expires_clause(category: &str) -> String {
    match ttl_days(category) {
        Some(d) => format!("datetime('now', '+{d} days')"),
        None => "NULL".into(),
    }
}

/// Écrit ou met à jour un souvenir (UPSERT sur la clé).
pub fn upsert_memory(
    conn: &Connection,
    category: &str,
    key: &str,
    value: &str,
    source: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO user_memory(key, category, value, source, expires_at)
             VALUES (?1, ?2, ?3, ?4, {e})
             ON CONFLICT(key) DO UPDATE SET
               category=?2, value=?3, source=?4, confidence=1.0,
               expires_at={e}, updated_at=datetime('now')",
            e = expires_clause(category)
        ),
        params![key, category, value, source],
    )?;
    Ok(())
}

pub fn forget_memory(conn: &Connection, key: &str) -> rusqlite::Result<bool> {
    Ok(conn.execute("DELETE FROM user_memory WHERE key = ?1", params![key])? > 0)
}

#[derive(Debug, Clone)]
pub struct Memory {
    pub key: String,
    pub category: String,
    pub value: String,
    pub confidence: f64,
}

/// Recherche mot-clé dans les souvenirs ACTIFS (non expirés), puis
/// refresh-on-read : les souvenirs lus repartent pour un TTL complet.
pub fn search_memories(conn: &Connection, query: &str) -> rusqlite::Result<Vec<Memory>> {
    let pattern = format!("%{}%", query.to_lowercase());
    let mut stmt = conn.prepare(
        "SELECT key, category, value, confidence FROM user_memory
         WHERE (expires_at IS NULL OR expires_at > datetime('now'))
           AND (lower(key) LIKE ?1 OR lower(value) LIKE ?1)
         ORDER BY updated_at DESC LIMIT 10",
    )?;
    let found: Vec<Memory> = stmt
        .query_map(params![pattern], |r| {
            Ok(Memory {
                key: r.get(0)?,
                category: r.get(1)?,
                value: r.get(2)?,
                confidence: r.get(3)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    for m in &found {
        refresh_on_read(conn, &m.key, &m.category)?;
    }
    Ok(found)
}

/// Tous les souvenirs actifs (injection dans le prompt, chantier 6).
pub fn active_memories(conn: &Connection) -> rusqlite::Result<Vec<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT key, category, value, confidence FROM user_memory
         WHERE expires_at IS NULL OR expires_at > datetime('now')
         ORDER BY category, key",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Memory {
                key: r.get(0)?,
                category: r.get(1)?,
                value: r.get(2)?,
                confidence: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>();
    rows
}

fn refresh_on_read(conn: &Connection, key: &str, category: &str) -> rusqlite::Result<()> {
    conn.execute(
        &format!(
            "UPDATE user_memory SET expires_at = {}, confidence = 1.0 WHERE key = ?1",
            expires_clause(category)
        ),
        params![key],
    )?;
    Ok(())
}

// ── Notes ───────────────────────────────────────────────────────────────────

pub fn create_note(conn: &Connection, title: &str, content: &str, tags: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO notes(title, content, tags) VALUES (?1, ?2, ?3)",
        params![title, content, tags],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn search_notes(conn: &Connection, query: &str) -> rusqlite::Result<Vec<(i64, String, String)>> {
    let pattern = format!("%{}%", query.to_lowercase());
    let mut stmt = conn.prepare(
        "SELECT note_id, title, content FROM notes
         WHERE lower(title) LIKE ?1 OR lower(content) LIKE ?1 OR lower(tags) LIKE ?1
         ORDER BY created_at DESC LIMIT 10",
    )?;
    let rows =
        stmt.query_map(params![pattern], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>();
    rows
}

pub fn list_notes(conn: &Connection) -> rusqlite::Result<Vec<(i64, String)>> {
    let mut stmt =
        conn.prepare("SELECT note_id, title FROM notes ORDER BY created_at DESC LIMIT 50")?;
    let rows =
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>();
    rows
}

// ── Historique de conversation ──────────────────────────────────────────────

/// Session par défaut des écrivains SANS desktop (bin, voix autonome) et clé
/// du journal visuel global. Ce n'est plus une conversation imposée : dans
/// l'app, la voix écrit dans la conversation ouverte.
pub const MAIN_SESSION: i64 = 1;

pub fn append_message(conn: &Connection, role: &str, content: &str) -> rusqlite::Result<()> {
    append_message_in(conn, MAIN_SESSION, role, content)
}

pub fn append_message_in(
    conn: &Connection,
    session: i64,
    role: &str,
    content: &str,
) -> rusqlite::Result<()> {
    // Jamais de messages orphelins : la conversation existe dès qu'on y écrit
    // (voix autonome, conversation supprimée pendant qu'on lui parle…).
    conn.execute(
        "INSERT OR IGNORE INTO chat_sessions(session_id, title) VALUES (?1, 'Conversation')",
        params![session],
    )?;
    conn.execute(
        "INSERT INTO conversations(role, content, session_id) VALUES (?1, ?2, ?3)",
        params![role, content, session],
    )?;
    Ok(())
}

/// Note un fichier créé pendant un tour de la conversation (après les
/// messages déjà persistés + le message utilisateur du tour en cours).
pub fn add_session_file(conn: &Connection, session: i64, chemin: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO session_fichiers(session_id, chemin, position)
         VALUES (?1, ?2, (SELECT COUNT(*) FROM conversations WHERE session_id = ?1) + 1)",
        params![session, chemin],
    )?;
    Ok(())
}

/// Garde la réflexion du DERNIER message de la conversation (à appeler juste
/// après avoir persisté la réponse de Waly).
pub fn add_session_reflexion(conn: &Connection, session: i64, texte: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO session_reflexions(session_id, position, texte)
         VALUES (?1, (SELECT COUNT(*) FROM conversations WHERE session_id = ?1) - 1, ?2)",
        params![session, texte.trim()],
    )?;
    Ok(())
}

/// Réflexions d'une conversation : (position du message de Waly, texte).
pub fn list_session_reflexions(conn: &Connection, session: i64) -> rusqlite::Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT position, texte FROM session_reflexions WHERE session_id = ?1 ORDER BY position, id",
    )?;
    let rows = stmt
        .query_map(params![session], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<Vec<_>, _>>();
    rows
}

/// (nombre total de messages, fichiers (chemin, position)) d'une conversation.
pub fn list_session_files(conn: &Connection, session: i64) -> rusqlite::Result<(i64, Vec<(String, i64)>)> {
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM conversations WHERE session_id = ?1",
        params![session],
        |r| r.get(0),
    )?;
    let mut stmt = conn.prepare(
        "SELECT chemin, position FROM session_fichiers WHERE session_id = ?1 ORDER BY position, id",
    )?;
    let rows = stmt
        .query_map(params![session], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok((total, rows))
}

/// Les `n` derniers messages du fil principal, du plus ancien au plus récent.
pub fn recent_messages(conn: &Connection, n: u32) -> rusqlite::Result<Vec<(String, String)>> {
    recent_messages_in(conn, MAIN_SESSION, n)
}

/// Les `n` derniers messages d'une session, du plus ancien au plus récent.
pub fn recent_messages_in(
    conn: &Connection,
    session: i64,
    n: u32,
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT role, content FROM (
           SELECT id, role, content FROM conversations
           WHERE session_id = ?2 ORDER BY id DESC LIMIT ?1
         ) ORDER BY id ASC",
    )?;
    let rows =
        stmt.query_map(params![n, session], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>();
    rows
}

// ── Résumé des longues conversations (lot 2) ────────────────────────────────

/// Résumé courant d'une session : (texte, id du dernier message couvert).
pub fn resume_session(conn: &Connection, session: i64) -> Option<(String, i64)> {
    conn.query_row(
        "SELECT resume, jusqu_a FROM session_resumes WHERE session_id=?1",
        params![session],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .ok()
}

pub fn set_resume_session(conn: &Connection, session: i64, resume: &str, jusqu_a: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO session_resumes(session_id, resume, jusqu_a) VALUES (?1, ?2, ?3)
         ON CONFLICT(session_id) DO UPDATE SET resume=?2, jusqu_a=?3, updated_at=datetime('now')",
        params![session, resume.trim(), jusqu_a],
    )?;
    Ok(())
}

/// Messages SORTIS de la fenêtre (tous sauf les `fenetre` derniers) et pas
/// encore couverts par le résumé : (id, rôle, contenu), du plus ancien au
/// plus récent. Vide s'il y en a moins de `seuil` (ne pas résumer pour 2
/// messages).
pub fn messages_a_resumer(
    conn: &Connection,
    session: i64,
    fenetre: u32,
    seuil: usize,
) -> rusqlite::Result<Vec<(i64, String, String)>> {
    let deja = resume_session(conn, session).map(|(_, j)| j).unwrap_or(0);
    let mut stmt = conn.prepare(
        "SELECT id, role, content FROM conversations
         WHERE session_id=?1 AND id > ?2 AND id < COALESCE(
           (SELECT MIN(id) FROM (SELECT id FROM conversations WHERE session_id=?1 ORDER BY id DESC LIMIT ?3)),
           0)
         ORDER BY id",
    )?;
    let v: Vec<(i64, String, String)> = stmt
        .query_map(params![session, deja, fenetre], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    Ok(if v.len() >= seuil { v } else { Vec::new() })
}

// ── Journal visuel de session (R4.5 ch. 1) ──────────────────────────────────

/// Ajoute une observation au journal visuel (texte seulement — jamais de
/// pixels). Purge au passage les entrées de plus de 7 jours (le journal est
/// une mémoire de session, pas une archive).
pub fn visual_memory_add(
    conn: &Connection,
    session: i64,
    kind: &str,
    content: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM visual_memory
         WHERE created_at < datetime('now','localtime','-7 days')",
        [],
    )?;
    conn.execute(
        "INSERT INTO visual_memory(session_id, kind, content) VALUES (?1, ?2, ?3)",
        params![session, kind, content],
    )?;
    Ok(())
}

/// Les `n` dernières observations RÉCENTES (< 12 h) d'une session, du plus
/// ancien au plus récent : (heure « HH:MM », kind, texte). 12 h = la portée
/// d'une « session » de vie ; au-delà, ce n'est plus « tout à l'heure ».
pub fn visual_memory_recent(
    conn: &Connection,
    session: i64,
    n: u32,
) -> rusqlite::Result<Vec<(String, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT heure, kind, content FROM (
           SELECT id, strftime('%H:%M', created_at) AS heure, kind, content
           FROM visual_memory
           WHERE session_id = ?2
             AND created_at >= datetime('now','localtime','-12 hours')
           ORDER BY id DESC LIMIT ?1
         ) ORDER BY id ASC",
    )?;
    let rows = stmt
        .query_map(params![n, session], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<Vec<_>, _>>();
    rows
}

/// Le dernier id du journal visuel d'une session (0 si vide) — le curseur
/// initial du delta « depuis ton dernier tour » (R4.5 ch. 2).
pub fn visual_memory_last_id(conn: &Connection, session: i64) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(id), 0) FROM visual_memory WHERE session_id = ?1",
        params![session],
        |r| r.get(0),
    )
}

/// Les entrées du journal postérieures à `apres_id` (delta d'un tour),
/// récentes (< 12 h), chrono : (id, heure « HH:MM », kind, content). Les
/// 'vu' d'un tour sont exclus (déjà dans la fenêtre comme réponse) ;
/// 'moment' et 'reflexion' (ch. 3) entrent — et le kind exposé permet à la
/// voix de ne DIRE que les 'moment'.
pub fn visual_memory_after(
    conn: &Connection,
    session: i64,
    apres_id: i64,
    max: u32,
) -> rusqlite::Result<Vec<(i64, String, String, String)>> {
    // Les `max` PLUS RÉCENTS (le delta reflète l'état actuel ; s'il s'est
    // passé plus de choses, le milieu est sauté, pas différé au tour
    // suivant), rendus en ordre chrono.
    let mut stmt = conn.prepare(
        "SELECT id, heure, kind, content FROM (
           SELECT id, strftime('%H:%M', created_at) AS heure, kind, content
           FROM visual_memory
           WHERE session_id = ?1 AND id > ?2 AND kind != 'vu'
             AND created_at >= datetime('now','localtime','-12 hours')
           ORDER BY id DESC LIMIT ?3
         ) ORDER BY id ASC",
    )?;
    let rows = stmt
        .query_map(params![session, apres_id, max], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<Result<Vec<_>, _>>();
    rows
}

// ── Sessions de conversation (desktop R3) ───────────────────────────────────

/// Une session vue de l'UI : id, titre, état de travail si c'est une
/// session-mission (None pour un chat), et genre ('chat'/'agent') — l'espace
/// UNIQUE de l'UI mélange les deux (fusion 2026-09-03).
pub type SessionRow = (i64, String, Option<String>, String);

pub fn create_session(conn: &Connection, title: &str, kind: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO chat_sessions(title, kind) VALUES (?1, ?2)",
        params![title, kind],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Les sessions d'un genre ('chat', 'agent', ou 'all' = tous mélangés), la
/// plus récemment ACTIVE d'abord (dernier message, à défaut la création).
pub fn list_sessions(conn: &Connection, kind: &str) -> rusqlite::Result<Vec<SessionRow>> {
    let mut stmt = conn.prepare(
        "SELECT s.session_id, s.title, s.agent_status, s.kind FROM chat_sessions s
         WHERE (?1 = 'all' OR s.kind = ?1)
         ORDER BY COALESCE(
           (SELECT MAX(c.id) FROM conversations c WHERE c.session_id = s.session_id),
           s.session_id) DESC",
    )?;
    let rows = stmt
        .query_map(params![kind], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<Vec<_>, _>>();
    rows
}

pub fn session_kind(conn: &Connection, session: i64) -> rusqlite::Result<String> {
    conn.query_row(
        "SELECT kind FROM chat_sessions WHERE session_id = ?1",
        params![session],
        |r| r.get(0),
    )
}

/// Pose l'état de travail d'une session-agent (travail/attente/fini/echec).
pub fn set_agent_status(conn: &Connection, session: i64, status: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE chat_sessions SET agent_status=?2 WHERE session_id=?1",
        params![session, status],
    )?;
    Ok(())
}

/// Supprime une conversation ou une mission et ce qui lui appartient
/// (messages, cartes de fichiers, journal visuel). Les fichiers sur disque
/// restent. Le journal visuel GLOBAL (clé MAIN_SESSION) et le journal du
/// sceau (la preuve) ne s'effacent pas avec une conversation.
pub fn delete_session(conn: &Connection, session: i64) -> rusqlite::Result<bool> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM conversations WHERE session_id = ?1", params![session])?;
    tx.execute("DELETE FROM session_fichiers WHERE session_id = ?1", params![session])?;
    tx.execute("DELETE FROM session_resumes WHERE session_id = ?1", params![session])?;
    tx.execute("DELETE FROM session_reflexions WHERE session_id = ?1", params![session])?;
    if session != MAIN_SESSION {
        tx.execute("DELETE FROM visual_memory WHERE session_id = ?1", params![session])?;
    }
    let n = tx.execute("DELETE FROM chat_sessions WHERE session_id = ?1", params![session])?;
    tx.commit()?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests_reglages_tours {
    use super::*;

    #[test]
    fn reglage_pose_remplace_et_vide_efface() {
        let conn = open(":memory:").unwrap();
        assert_eq!(reglage(&conn, "style"), None);
        set_reglage(&conn, "style", "concis").unwrap();
        set_reglage(&conn, "style", "  detaille ").unwrap();
        assert_eq!(reglage(&conn, "style").as_deref(), Some("detaille"));
        set_reglage(&conn, "style", "   ").unwrap();
        assert_eq!(reglage(&conn, "style"), None);
    }

    #[test]
    fn projet_range_sans_jamais_perdre_de_conversation() {
        let conn = open(":memory:").unwrap();
        let p = create_projet(&conn, " Thèse ").unwrap();
        let s = create_session(&conn, "Plan", "chat").unwrap();
        set_session_projet(&conn, s, Some(p)).unwrap();
        add_projet_fichier(&conn, p, r"C:\docs\biblio.docx").unwrap();
        add_projet_fichier(&conn, p, r"C:\docs\biblio.docx").unwrap(); // idempotent
        let pr = projet(&conn, p).unwrap().unwrap();
        assert_eq!((pr.nom.as_str(), pr.sessions, pr.fichiers.len()), ("Thèse", 1, 1));
        assert_eq!(session_projet(&conn, s), Some(p));
        assert_eq!(sessions_du_projet(&conn, p).unwrap().len(), 1);
        update_projet(&conn, p, "Thèse v2", "Cite en APA.", true).unwrap();
        assert!(list_projets(&conn).unwrap()[0].archive);
        delete_projet(&conn, p).unwrap();
        assert!(projet(&conn, p).unwrap().is_none());
        assert_eq!(session_projet(&conn, s), None);
        assert_eq!(list_sessions(&conn, "all").unwrap().len(), 1); // la conversation reste
    }

    #[test]
    fn missions_programmees_echues_une_fois_par_echeance() {
        let conn = open(":memory:").unwrap();
        let p = create_mission_programmee(&conn, "Ponctuelle", "2020-01-01 08:00:00", None, None).unwrap();
        let q = create_mission_programmee(&conn, "Chaque jour", "2020-01-01 08:00:00", Some("quotidien"), None).unwrap();
        let o = create_mission_programmee(&conn, "Semaine", "2020-01-03 08:00:00", Some("ouvres"), None).unwrap();
        create_mission_programmee(&conn, "Plus tard", "2999-01-01 08:00:00", None, None).unwrap();
        let due: Vec<i64> = due_missions_programmees(&conn).unwrap().iter().map(|m| m.id).collect();
        assert_eq!(due, vec![p, q, o]);
        assert!(due_missions_programmees(&conn).unwrap().is_empty()); // une seule fois
        let l = list_missions_programmees(&conn).unwrap();
        assert_eq!(l.len(), 3); // la ponctuelle est désactivée
        let ouvres = l.iter().find(|m| m.id == o).unwrap();
        let jour: String = conn
            .query_row("SELECT strftime('%w', ?1)", params![ouvres.prochain], |r| r.get(0))
            .unwrap();
        assert!(!["0", "6"].contains(&jour.as_str()), "jour ouvré attendu, eu {jour}");
        mission_programmee_lancee(&conn, q, 42).unwrap();
        assert!(cancel_mission_programmee(&conn, q).unwrap());
        assert_eq!(list_missions_programmees(&conn).unwrap().len(), 2);
    }

    #[test]
    fn competence_versionnee_sans_perte() {
        let conn = open(":memory:").unwrap();
        upsert_competence(&conn, "rapport", "Rapport", "quand on demande un rapport", "v1").unwrap();
        assert_eq!(version_competence(&conn, "rapport").unwrap(), 1);
        assert!(reviser_competence(&conn, "rapport", "v2", "amélioré après usage").unwrap());
        assert!(reviser_competence(&conn, "rapport", "v3", "encore").unwrap());
        assert_eq!(version_competence(&conn, "rapport").unwrap(), 3);
        assert_eq!(get_competence(&conn, "rapport").unwrap().unwrap().recette, "v3");
        let h = versions_competence(&conn, "rapport").unwrap();
        assert_eq!((h[0].version, h[0].recette.as_str()), (2, "v2"));
        assert!(restaurer_competence(&conn, "rapport", 1).unwrap());
        assert_eq!(get_competence(&conn, "rapport").unwrap().unwrap().recette, "v1");
        assert_eq!(version_competence(&conn, "rapport").unwrap(), 4); // v3 archivée aussi
        assert!(!reviser_competence(&conn, "inconnue", "x", "").unwrap());
        assert!(delete_competence(&conn, "rapport").unwrap());
        assert!(versions_competence(&conn, "rapport").unwrap().is_empty());
    }

    #[test]
    fn tours_agreges_par_modele() {
        let conn = open(":memory:").unwrap();
        let t = |p, c, d, e| Tour {
            session_id: 1,
            modele: "m".into(),
            prompt_tokens: p,
            completion_tokens: c,
            duree_ms: d,
            premier_ms: Some(100),
            estime: e,
        };
        add_tour(&conn, &t(100, 10, 1000, false)).unwrap();
        add_tour(&conn, &t(300, 30, 3000, true)).unwrap();
        let u = utilisation_par_modele(&conn, 30).unwrap();
        assert_eq!(u, vec![("m".into(), 2, 400, 40, 2000, Some(100), true)]);
        let d = derniers_tours(&conn, 1).unwrap();
        assert_eq!(d[0].1.prompt_tokens, 300);
    }
}

#[cfg(test)]
mod tests_suppression {
    use super::*;

    #[test]
    fn supprimer_une_session_efface_messages_et_cartes() {
        let conn = open(":memory:").unwrap();
        let id = create_session(&conn, "Brouillon", "chat").unwrap();
        append_message_in(&conn, id, "user", "bonjour").unwrap();
        add_session_file(&conn, id, r"C:\docs\note.md").unwrap();
        visual_memory_add(&conn, id, "vu", "une tasse").unwrap();
        visual_memory_add(&conn, MAIN_SESSION, "vu", "journal global").unwrap();
        assert!(delete_session(&conn, id).unwrap());
        assert!(recent_messages_in(&conn, id, 10).unwrap().is_empty());
        assert!(list_session_files(&conn, id).unwrap().1.is_empty());
        assert!(!list_sessions(&conn, "all").unwrap().iter().any(|s| s.0 == id));
        // Plus de « Fil principal » intouchable : la session 1 se supprime
        // comme les autres, mais le journal visuel global reste.
        append_message_in(&conn, MAIN_SESSION, "user", "vocal").unwrap();
        assert!(delete_session(&conn, MAIN_SESSION).unwrap());
        assert!(visual_memory_last_id(&conn, MAIN_SESSION).unwrap() > 0);
        // Déjà supprimée : rien à faire.
        assert!(!delete_session(&conn, id).unwrap());
    }

    #[test]
    fn fichiers_crees_places_apres_le_message_du_tour() {
        let conn = open(":memory:").unwrap();
        let id = create_session(&conn, "Note", "chat").unwrap();
        append_message_in(&conn, id, "user", "salut").unwrap();
        append_message_in(&conn, id, "assistant", "bonjour").unwrap();
        // Tour 2 : le fichier naît avant la persistance du tour.
        add_session_file(&conn, id, r"C:\docs\note.md").unwrap();
        add_session_file(&conn, id, r"C:\docs\note.md").unwrap(); // doublon ignoré
        append_message_in(&conn, id, "user", "fais une note").unwrap();
        append_message_in(&conn, id, "assistant", "voici").unwrap();
        let (total, f) = list_session_files(&conn, id).unwrap();
        assert_eq!(total, 4);
        assert_eq!(f, vec![(r"C:\docs\note.md".to_string(), 3)]);
    }

    #[test]
    fn journal_du_sceau_accepte_les_sorties_ouvertes_meme_sur_une_ancienne_base() {
        let chemin = std::env::temp_dir().join(format!("waly-audit-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&chemin);
        {
            // Ancien schéma : le CHECK ne connaît pas 'sortie'.
            let c = Connection::open(&chemin).unwrap();
            c.execute_batch(
                "CREATE TABLE audit_sceau (id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL,
                   genre TEXT NOT NULL CHECK(genre IN ('pose','levee','bloque')),
                   detail TEXT NOT NULL DEFAULT '', cle TEXT UNIQUE,
                   at TEXT NOT NULL DEFAULT (datetime('now','localtime')));
                 INSERT INTO audit_sceau(session_id, genre, detail) VALUES (0, 'bloque', 'ancienne ligne');",
            )
            .unwrap();
        }
        let conn = open(chemin.to_str().unwrap()).unwrap();
        crate::sceau::noter(&conn, "sortie", "modèle extérieur — test").unwrap();
        let l = crate::sceau::lignes_audit(&conn, 0, 10).unwrap();
        assert_eq!(l.len(), 2, "l'ancienne ligne est gardée");
        assert_eq!(l[0].genre, "sortie");
        drop(conn);
        let _ = std::fs::remove_file(&chemin);
    }

    #[test]
    fn reflexion_rangee_sur_le_message_de_waly() {
        let conn = open(":memory:").unwrap();
        let id = create_session(&conn, "Calcul", "chat").unwrap();
        append_message_in(&conn, id, "user", "ça tient ?").unwrap();
        append_message_in(&conn, id, "assistant", "non").unwrap();
        add_session_reflexion(&conn, id, " je pose le calcul 
").unwrap();
        assert_eq!(list_session_reflexions(&conn, id).unwrap(), vec![(1, "je pose le calcul".to_string())]);
        assert!(delete_session(&conn, id).unwrap());
        assert!(list_session_reflexions(&conn, id).unwrap().is_empty());
    }

    #[test]
    fn pas_de_messages_orphelins() {
        let conn = open(":memory:").unwrap();
        assert!(list_sessions(&conn, "all").unwrap().is_empty(), "plus de session imposée");
        append_message_in(&conn, 42, "user", "voix autonome").unwrap();
        assert!(list_sessions(&conn, "all").unwrap().iter().any(|s| s.0 == 42));
    }
}

pub fn set_session_title(conn: &Connection, session: i64, title: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE chat_sessions SET title=?2 WHERE session_id=?1",
        params![session, title],
    )?;
    Ok(())
}

/// La session du dernier message écrit (reprendre là où on était) ; le fil
/// principal si la base est vierge.
pub fn last_active_session(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COALESCE((SELECT session_id FROM conversations ORDER BY id DESC LIMIT 1), ?1)",
        params![MAIN_SESSION],
        |r| r.get(0),
    )
}

/// Sessions d'un genre ('all' accepté) dont le titre OU un message contient
/// `query` (recherche locale, LIKE insensible à la casse ASCII), même ordre
/// que `list_sessions`.
pub fn search_sessions(
    conn: &Connection,
    query: &str,
    kind: &str,
) -> rusqlite::Result<Vec<SessionRow>> {
    let pat = format!("%{}%", query);
    let mut stmt = conn.prepare(
        "SELECT s.session_id, s.title, s.agent_status, s.kind FROM chat_sessions s
         WHERE (?2 = 'all' OR s.kind = ?2)
           AND (s.title LIKE ?1
            OR EXISTS (SELECT 1 FROM conversations c
                       WHERE c.session_id = s.session_id AND c.content LIKE ?1))
         ORDER BY COALESCE(
           (SELECT MAX(c.id) FROM conversations c WHERE c.session_id = s.session_id),
           s.session_id) DESC",
    )?;
    let rows = stmt
        .query_map(params![pat, kind], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<Vec<_>, _>>();
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_vec_est_charge_et_cherche() {
        let conn = open(":memory:").unwrap();
        let version: String =
            conn.query_row("SELECT vec_version()", [], |r| r.get(0)).unwrap();
        assert!(version.starts_with('v'), "vec_version inattendue: {version}");

        conn.execute_batch(
            "CREATE VIRTUAL TABLE t USING vec0(embedding float[4]);
             INSERT INTO t(rowid, embedding) VALUES
               (1, '[1.0, 0.0, 0.0, 0.0]'),
               (2, '[0.0, 1.0, 0.0, 0.0]');",
        )
        .unwrap();
        let nearest: i64 = conn
            .query_row(
                "SELECT rowid FROM t WHERE embedding MATCH '[0.9, 0.1, 0.0, 0.0]'
                 ORDER BY distance LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(nearest, 1);
    }

    #[test]
    fn sessions_wrappers_cloisonnement_et_recherche() {
        let conn = open(":memory:").unwrap();
        // La session par défaut naît à la première écriture legacy.
        append_message(&conn, "user", "bonjour depuis la voix").unwrap();
        assert_eq!(last_active_session(&conn).unwrap(), MAIN_SESSION);
        // Une nouvelle session, activité plus récente → en tête de liste.
        let s = create_session(&conn, "Conversation", "chat").unwrap();
        append_message_in(&conn, s, "user", "on parle du renard").unwrap();
        set_session_title(&conn, s, "Renard").unwrap();
        let sessions = list_sessions(&conn, "chat").unwrap();
        assert_eq!(sessions[0], (s, "Renard".to_string(), None, "chat".to_string()));
        assert_eq!(last_active_session(&conn).unwrap(), s);
        // L'historique est cloisonné par session.
        assert_eq!(recent_messages(&conn, 10).unwrap().len(), 1);
        assert_eq!(recent_messages_in(&conn, s, 10).unwrap().len(), 1);
        // Recherche locale : par contenu et par titre, cloisonnée par genre.
        assert_eq!(
            search_sessions(&conn, "renard", "chat").unwrap(),
            vec![(s, "Renard".to_string(), None, "chat".to_string())]
        );
        assert!(search_sessions(&conn, "voix", "chat")
            .unwrap()
            .iter()
            .any(|(id, _, _, _)| *id == MAIN_SESSION));
        assert!(search_sessions(&conn, "renard", "agent").unwrap().is_empty());
        // 'all' mélange les genres (l'espace unique de l'UI).
        assert!(search_sessions(&conn, "renard", "all").unwrap().len() == 1);
    }

    #[test]
    fn sessions_agents_genre_et_statut() {
        let conn = open(":memory:").unwrap();
        append_message(&conn, "user", "conversation par défaut").unwrap();
        let a = create_session(&conn, "Ranger mes notes", "agent").unwrap();
        assert_eq!(session_kind(&conn, a).unwrap(), "agent");
        assert_eq!(session_kind(&conn, MAIN_SESSION).unwrap(), "chat");
        // Les genres ne se mélangent pas dans les listes filtrées…
        assert!(list_sessions(&conn, "chat").unwrap().iter().all(|(id, _, _, _)| *id != a));
        assert_eq!(list_sessions(&conn, "agent").unwrap().len(), 1);
        // … mais 'all' rend tout (fil principal + mission).
        assert_eq!(list_sessions(&conn, "all").unwrap().len(), 2);
        // Cycle d'état d'une session-agent.
        set_agent_status(&conn, a, "travail").unwrap();
        set_agent_status(&conn, a, "attente").unwrap();
        set_agent_status(&conn, a, "fini").unwrap();
        assert_eq!(
            list_sessions(&conn, "agent").unwrap()[0],
            (a, "Ranger mes notes".to_string(), Some("fini".to_string()), "agent".to_string())
        );
    }

    #[test]
    fn memoire_upsert_recherche_oubli() {
        let conn = open(":memory:").unwrap();
        upsert_memory(&conn, "preference", "cafe", "L'utilisateur aime le café noir", "declared").unwrap();
        upsert_memory(&conn, "fact", "metier", "L'utilisateur développe Waly", "declared").unwrap();

        let found = search_memories(&conn, "café").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].key, "cafe");

        // UPSERT sur la même clé remplace la valeur.
        upsert_memory(&conn, "preference", "cafe", "café au lait finalement", "declared").unwrap();
        let found = search_memories(&conn, "lait").unwrap();
        assert_eq!(found.len(), 1);

        assert!(forget_memory(&conn, "cafe").unwrap());
        assert!(!forget_memory(&conn, "cafe").unwrap());
        assert!(search_memories(&conn, "café").unwrap().is_empty());
    }

    #[test]
    fn ttl_par_categorie_et_expiration() {
        let conn = open(":memory:").unwrap();
        assert_eq!(ttl_days("fact"), None);
        assert_eq!(ttl_days("event"), Some(7));

        upsert_memory(&conn, "event", "rdv", "dentiste lundi", "declared").unwrap();
        // Forcer l'expiration dans le passé : le souvenir disparaît des actifs.
        conn.execute(
            "UPDATE user_memory SET expires_at = datetime('now', '-1 day') WHERE key='rdv'",
            [],
        )
        .unwrap();
        assert!(search_memories(&conn, "dentiste").unwrap().is_empty());
        assert!(active_memories(&conn).unwrap().is_empty());
    }

    #[test]
    fn refresh_on_read_reinitialise_le_ttl() {
        let conn = open(":memory:").unwrap();
        upsert_memory(&conn, "context", "projet", "on boucle R2", "declared").unwrap();
        // TTL presque écoulé...
        conn.execute(
            "UPDATE user_memory SET expires_at = datetime('now', '+1 hour') WHERE key='projet'",
            [],
        )
        .unwrap();
        // ... la lecture le remet à 30 jours.
        assert_eq!(search_memories(&conn, "R2").unwrap().len(), 1);
        let expires: String = conn
            .query_row("SELECT expires_at FROM user_memory WHERE key='projet'", [], |r| r.get(0))
            .unwrap();
        let in_29_days: String = conn
            .query_row("SELECT datetime('now', '+29 days')", [], |r| r.get(0))
            .unwrap();
        assert!(expires > in_29_days, "TTL non rafraichi: {expires}");
    }

    #[test]
    fn index_semantique_knn_et_hybride() {
        let conn = open(":memory:").unwrap();
        // Vecteurs 384d faits main : e1 pointe sur l'axe 0, e2 sur l'axe 1.
        let mut v1 = vec![0.0f32; 384];
        v1[0] = 1.0;
        let mut v2 = vec![0.0f32; 384];
        v2[1] = 1.0;
        let json = |v: &[f32]| crate::embed::to_vec_json(v);

        upsert_memory(&conn, "fact", "plat", "le ndolé", "declared").unwrap();
        upsert_memory(&conn, "fact", "chat", "un chat, Simba", "declared").unwrap();
        index_entity(&conn, "memory", "plat", &json(&v1)).unwrap();
        index_entity(&conn, "memory", "chat", &json(&v2)).unwrap();

        // Requête proche de v1 (cos 1.0) : « plat » sort en premier.
        let hits = semantic_neighbors(&conn, "memory", &json(&v1), 5).unwrap();
        assert_eq!(hits[0].0, "plat");
        assert!((hits[0].1 - 1.0).abs() < 1e-3, "cos attendu 1.0: {}", hits[0].1);

        let found = hybrid_search_memories(&conn, "aime manger", &json(&v1)).unwrap();
        assert_eq!(found[0].key, "plat");

        // Tainté : exclu de la recherche (spec ancien monde).
        conn.execute("UPDATE embedding_map SET tainted=1 WHERE entity_key='plat'", [])
            .unwrap();
        let hits = semantic_neighbors(&conn, "memory", &json(&v1), 5).unwrap();
        assert!(hits.iter().all(|(k, _)| k != "plat"));

        // Désindexation (oubli).
        unindex_entity(&conn, "memory", "chat").unwrap();
        let hits = semantic_neighbors(&conn, "memory", &json(&v2), 5).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn taches_creation_liste_maj() {
        let conn = open(":memory:").unwrap();
        let a = create_task(&conn, "Appeler le dentiste", None, "haute", Some("2026-07-06")).unwrap();
        create_task(&conn, "Ranger le garage", Some("un jour"), "basse", None).unwrap();
        // Haute priorite AVEC echeance passe devant.
        let open = list_open_tasks(&conn).unwrap();
        assert_eq!(open.len(), 2);
        assert_eq!(open[0].id, a);
        // Terminer une tache la retire des ouvertes.
        assert!(update_task(&conn, a, Some("faite"), None).unwrap());
        assert_eq!(list_open_tasks(&conn).unwrap().len(), 1);
        assert!(!update_task(&conn, 999, Some("faite"), None).unwrap());
    }

    #[test]
    fn rappels_creation_liste_annulation() {
        let conn = open(":memory:").unwrap();
        let r = create_reminder(&conn, "Sortir avec Paul", None, "2027-01-01 18:00", None).unwrap();
        create_reminder(&conn, "Boire de l'eau", None, "2027-01-01 09:00", Some("quotidien")).unwrap();
        // Trie par echeance : 09:00 avant 18:00.
        let active = list_active_reminders(&conn).unwrap();
        assert_eq!(active.len(), 2);
        assert_eq!(active[0].title, "Boire de l'eau");
        assert!(cancel_reminder(&conn, r).unwrap());
        assert_eq!(list_active_reminders(&conn).unwrap().len(), 1);
    }

    #[test]
    fn rappels_echus_ponctuel_vs_recurrent() {
        let conn = open(":memory:").unwrap();
        // Un ponctuel et un quotidien, tous deux dans le passe.
        create_reminder(&conn, "Ponctuel", None, "2020-01-01 08:00", None).unwrap();
        let rec =
            create_reminder(&conn, "Quotidien", None, "2020-01-01 08:00", Some("quotidien")).unwrap();
        let due = due_reminders(&conn).unwrap();
        assert_eq!(due.len(), 2);
        // Le ponctuel ne re-declenche plus ; le recurrent est reprogramme.
        assert!(due_reminders(&conn).unwrap().is_empty());
        let active = list_active_reminders(&conn).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, rec);
        assert!(active[0].remind_at > "2020-01-02".to_string());
    }

    #[test]
    fn notes_et_historique() {
        let conn = open(":memory:").unwrap();
        let id = create_note(&conn, "Idées R2", "embeddings locaux e5-small", "[\"waly\"]").unwrap();
        assert_eq!(search_notes(&conn, "e5-small").unwrap()[0].0, id);
        assert_eq!(list_notes(&conn).unwrap().len(), 1);

        append_message(&conn, "user", "salut").unwrap();
        append_message(&conn, "assistant", "salut !").unwrap();
        let hist = recent_messages(&conn, 10).unwrap();
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0], ("user".into(), "salut".into()));
    }
}
