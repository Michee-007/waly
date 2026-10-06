//! Construction du prompt sous DISCIPLINE APPEND-ONLY (R4.5 chantier 0).
//!
//! Mesure du 2026-07-08 (GATE A, `engines/README.md` § Cache de conversation
//! FLM) : v0.9.43 réemploie le KV UNIQUEMENT si la requête étend VERBATIM la
//! conversation précédente — toute mutation (système re-généré, fenêtre qui
//! glisse, `system` en milieu de conversation) = tout le préfill se re-paie.
//! D'où le partage des rôles :
//! - [`inject_context`] bâtit le prompt système STABLE (base + souvenirs +
//!   consigne d'attentes permanente) — à appeler AU REBUILD de fenêtre
//!   seulement, jamais par tour.
//! - [`en_tete_frais`] bâtit le suffixe DYNAMIQUE de l'en-tête `[...]` du
//!   dernier message utilisateur (conscience d'appel, actions en attente) —
//!   en QUEUE de conversation, il ne coûte que ses propres tokens.

use rusqlite::Connection;

use crate::store;

/// `base` + souvenirs actifs + consigne permanente `resoudre_attentes`.
/// STABLE : ne le régénérer qu'au (re)build de la fenêtre. Les souvenirs
/// créés en cours de fenêtre n'apparaissent qu'au rebuild suivant — le
/// modèle les connaît déjà par le résultat d'outil dans le contexte.
pub fn inject_context(base: &str, conn: &Connection) -> String {
    let mut p = String::from(base);
    // Personnalité et instructions (Paramètres › Général) : au système
    // STABLE — une modification ne vaut qu'au rebuild suivant (le desktop en
    // provoque un ; la voix au prochain démarrage).
    if let Some(i) = store::reglage(conn, "instructions") {
        p.push_str(&format!(
            "\nInstructions de {} sur ta facon de lui parler (a respecter en priorite) :\n{}",
            crate::user::designation(),
            i
        ));
    }
    // La Garde : mémoire coupée = aucun souvenir au prompt (et c'est noté).
    let memoire_permise = crate::garde::permis(conn, crate::garde::Ressource::Memoire);
    if let Ok(mems) = store::active_memories(conn).map(|m| if memoire_permise { m } else { Vec::new() }) {
        if !mems.is_empty() {
            crate::garde::noter(conn, crate::garde::Ressource::Memoire, "a relu ses souvenirs pour la conversation", false);
            p.push_str(&format!(
                "\nCe que tu sais deja sur {} (tes souvenirs) :",
                crate::user::designation()
            ));
            for m in &mems {
                p.push_str(&format!("\n- {}: {}", m.key, m.value));
            }
        }
    }
    // Consigne PERMANENTE (forme exacte imposée — vécu : sans elle le 4B
    // « annule » avec le mauvais outil puis affirme l'avoir fait). La LISTE
    // des attentes, elle, voyage dans l'en-tête frais du message.
    p.push_str(
        "\nSi l'en-tete [ ... ] d'un message liste des ACTIONS EN ATTENTE \
         (#id outil args), reponds-y UNIQUEMENT via l'outil resoudre_attentes : \
         s'il dit oui -> resoudre_attentes {\"confirmes\":[id]} ; s'il refuse ou \
         annule -> resoudre_attentes {\"rejetes\":[id]}. Tant que tu n'as pas \
         appele cet outil, la demande N'EST PAS traitee — ne dis jamais le \
         contraire.",
    );
    p
}

/// Bloc « projet » du système STABLE (lot 2) : si la session est rangée dans
/// un projet, ses instructions et la liste de ses fichiers de référence (les
/// NOMS seulement — le 4B les lit avec `lire_fichier` quand la question
/// l'exige : le contexte est trop court pour tout précharger). Vide sinon.
pub fn contexte_projet(conn: &Connection, session: i64) -> String {
    let Some(id) = store::session_projet(conn, session) else { return String::new() };
    let Ok(Some(p)) = store::projet(conn, id) else { return String::new() };
    let mut s = format!("\nPROJET : cette conversation fait partie du projet « {} ».", p.nom);
    if !p.instructions.is_empty() {
        s.push_str(&format!(
            "\nInstructions du projet (a respecter en priorite, a chaque reponse) :\n{}",
            p.instructions
        ));
    }
    // Le CONTENU des fichiers entre dans le système (budget borné) : le 4B
    // n'appelait pas lire_fichier malgré une consigne explicite (vécu
    // 2026-09-30). Système STABLE -> payé au rebuild seulement. Au-delà du
    // budget, les fichiers restants sont listés pour lire_fichier.
    let mut budget = PROJET_BUDGET;
    let mut restants = Vec::new();
    for f in &p.fichiers {
        let chemin = std::path::Path::new(f);
        let nom = chemin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let texte = texte_fichier(chemin);
        match texte {
            Some(t) if budget > 200 => {
                let pris: String = t.chars().take(budget).collect();
                budget = budget.saturating_sub(pris.chars().count());
                let coupe = if pris.chars().count() < t.chars().count() { "\n[… suite non chargee]" } else { "" };
                s.push_str(&format!(
                    "\nContenu du fichier de reference « {nom} » (source fiable pour ce projet) :\n{pris}{coupe}\n[fin de {nom}]"
                ));
            }
            _ => restants.push(f.clone()),
        }
    }
    if !restants.is_empty() {
        s.push_str("\nAutres fichiers du projet, a lire avec l'outil lire_fichier (chemin exact) si la question les concerne :");
        for f in &restants {
            s.push_str(&format!("\n- {f}"));
        }
    }
    s
}

/// Rappel de l'en-tête frais : ` | projet X — consigne : …` (instructions
/// tronquées à 160 caractères). Vide hors projet ou sans instructions.
pub fn rappel_projet(conn: &Connection, session: i64) -> String {
    let Some(id) = store::session_projet(conn, session) else { return String::new() };
    let Ok(Some(p)) = store::projet(conn, id) else { return String::new() };
    if p.instructions.is_empty() {
        return format!(" | projet « {} »", p.nom);
    }
    let court: String = p.instructions.chars().take(160).collect();
    let suite = if p.instructions.chars().count() > 160 { "…" } else { "" };
    format!(" | projet « {} » — consigne a appliquer : {court}{suite}", p.nom)
}

/// Budget en caractères du contenu des fichiers de projet dans le système
/// (~1 700 tokens sur une fenêtre de 8 192 dont ~3 000 déjà pris).
const PROJET_BUDGET: usize = 6000;

fn texte_fichier(chemin: &std::path::Path) -> Option<String> {
    let ext = chemin.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let t = if ["docx", "xlsx", "pptx", "csv", "tsv"].contains(&ext.as_str()) {
        crate::apercu::texte(chemin).ok()?
    } else {
        let o = std::fs::read(chemin).ok()?;
        if o.iter().take(4096).any(|&b| b == 0) {
            return None;
        }
        String::from_utf8_lossy(&o).into_owned()
    };
    let t = t.trim().to_string();
    (!t.is_empty()).then_some(t)
}

/// Consigne de longueur selon le réglage « Style de réponse » (conversation
/// écrite ; la voix garde ses répliques courtes). Défaut = « normal ».
pub fn consigne_longueur(conn: &Connection) -> &'static str {
    match store::reglage(conn, "style").as_deref() {
        Some("concis") => "en une ou deux phrases, sans detour",
        Some("detaille") => {
            "de facon complete et structuree : explique, donne des exemples, \
             utilise des listes ou des titres quand ca aide"
        }
        _ => "en une a trois phrases",
    }
}

/// Suffixe frais de l'en-tête du message utilisateur : ` | conscience` +
/// ` | ACTIONS EN ATTENTE : #id outil args…`. Vide s'il n'y a rien. Le tour
/// compose : `[{horodatage}{en_tete_frais}] {message}` — c'est la SEULE
/// place du dynamisme par tour (viser court : chaque token s'y re-paie).
pub fn en_tete_frais(conn: &Connection, conscience: Option<&str>) -> String {
    let mut s = String::new();
    if let Some(c) = conscience {
        if !c.is_empty() {
            s.push_str(" | ");
            s.push_str(c);
        }
    }
    if let Ok(pending) = store::list_pending(conn) {
        if !pending.is_empty() {
            s.push_str(" | ACTIONS EN ATTENTE :");
            for a in &pending {
                s.push_str(&format!(" #{} {} {}", a.id, a.tool_name, a.tool_args));
            }
        }
    }
    s
}

/// Conscience du huis clos (R6a) pour le prompt système STABLE. `actif` vient
/// de `sceau::actif()` (état RÉEL, interrogé au rebuild) — on n'affirme le
/// scellé que s'il est vrai (honnêteté). Vide sinon : Waly n'a alors rien de
/// particulier à dire (l'UI signale « réseau non scellé » de son côté).
pub fn conscience_sceau(actif: bool) -> &'static str {
    if actif {
        " Tu es cultive et tu raisonnes tres bien : tu EXPLIQUES directement, \
         avec ce que tu sais, TOUTE question de connaissance ou de definition — \
         un « modele du monde », une notion d'IA, de science, d'histoire, de \
         philosophie : tu reponds, sans reserve, sans dire que c'est « hors de \
         ta portee ». Ne confonds JAMAIS « je ne peux pas chercher sur le web » \
         avec « je ne sais pas » : tu sais enormement de choses en local. La \
         SEULE chose que le huis clos t'interdit, c'est d'aller chercher une \
         donnee EN DIRECT en ligne (actu du jour, meteo, cours, une page web \
         precise) ou d'envoyer quelque chose dehors ; la, et SEULEMENT la, tu \
         dis simplement que tu es scelle : rien ne sort sans son accord."
    } else {
        ""
    }
}

/// Le DELTA du journal visuel depuis le dernier tour (R4.5 ch. 2 — c'est le
/// « remarquer ce qui change » du récit fusionné) : ` | depuis ton dernier
/// tour : X (HH:MM), Y (HH:MM)` (3 max, budget GATE B) + le nouveau curseur.
/// Vide et curseur inchangé s'il ne s'est rien passé. L'appelant garde le
/// curseur d'un tour à l'autre (par processus : au démarrage, le « dernier
/// tour » est le début de l'appel).
pub fn delta_visuel(conn: &Connection, session: i64, apres_id: i64) -> (String, i64) {
    let entries = match store::visual_memory_after(conn, session, apres_id, 3) {
        Ok(e) if !e.is_empty() => e,
        _ => return (String::new(), apres_id),
    };
    let curseur = entries.last().map(|(id, _, _, _)| *id).unwrap_or(apres_id);
    let mut s = String::from(" | depuis ton dernier tour : ");
    for (i, (_, heure, kind, content)) in entries.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        // Les réflexions se présentent comme telles (une hypothèse, pas un
        // fait vu) ; les moments (adressés à l'utilisateur) se citent ; le reste
        // s'énonce tel quel.
        match kind.as_str() {
            "reflexion" => s.push_str(&format!("ton impression : {content} ({heure})")),
            "moment" => {
                s.push_str(&format!("tu as remarque : « {content} » ({heure})"))
            }
            _ => s.push_str(&format!("{content} ({heure})")),
        }
    }
    (s, curseur)
}

/// Bloc « mémoire visuelle » du prompt système STABLE : les dernières
/// observations du journal visuel (R4.5 ch. 1 — ce que Waly a VU survit à
/// la dégradation de l'image). À générer AU REBUILD seulement, comme
/// [`inject_context`] ; le tour courant a de toute façon sa description
/// fraîche dans la fenêtre. Vide hors observations récentes.
pub fn memoire_visuelle(conn: &Connection, session: i64) -> String {
    let entries = match store::visual_memory_recent(conn, session, 6) {
        Ok(e) if !e.is_empty() => e,
        _ => return String::new(),
    };
    // Preambule assertif : les prompts (voix surtout) nient les capacites
    // hors liste (« tu n'as ni corps... ») — sans lui, le 4B repond « je
    // n'ai pas de vision » alors que le souvenir est ecrit juste dessous
    // (vecu au banc 2026-07-08). Et les lignes 'vu' sont des reponses
    // adressees a l'utilisateur (« Tu es... ») : les CITER, sinon le « tu »
    // bascule sur Waly.
    let mut s = String::from(
        "\nTA MEMOIRE VISUELLE — SOUVENIRS PASSES de tes appels video (heure \
         locale). Ils ne decrivent PAS le present : tu ne vois en ce moment QUE \
         si l'en-tete du message dit « en appel video » ou « tu regardes \
         l'ecran » ; sinon ta camera est eteinte et tu ne vois RIEN. Cite ces \
         souvenirs seulement quand on te demande ce que tu as vu, en disant \
         quand :",
    );
    for (heure, kind, content) in &entries {
        // Les contenus adresses a l'utilisateur (« tu ») se CITENT, sinon leur
        // « tu » bascule sur Waly (piege grave au ch. 1).
        match kind.as_str() {
            "vu" => s.push_str(&format!(
                "\n- [{heure}] tu as regarde et repondu : « {content} »"
            )),
            "moment" => {
                s.push_str(&format!("\n- [{heure}] tu as remarque : « {content} »"))
            }
            "reflexion" => s.push_str(&format!("\n- [{heure}] ton impression : {content}")),
            _ => s.push_str(&format!("\n- [{heure}] {content}")),
        }
    }
    s
}

/// Tronque proprement à ~`max` caractères sur une frontière de char (pour
/// journaliser une description sans avaler un roman).
pub fn compacte(texte: &str, max: usize) -> String {
    if texte.chars().count() <= max {
        return texte.to_string();
    }
    let mut s: String = texte.chars().take(max).collect();
    s.push('…');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systeme_stable_souvenirs_et_consigne() {
        let conn = store::open(":memory:").unwrap();
        let p = inject_context("Base.", &conn);
        assert!(p.starts_with("Base."), "{p}");
        // La consigne d'attentes est PERMANENTE (stable pour le cache).
        assert!(p.contains("resoudre_attentes"), "{p}");
        assert!(!p.contains("souvenirs"), "{p}");

        store::upsert_memory(&conn, "fact", "plat", "ndolé", "declared").unwrap();
        let p = inject_context("Base.", &conn);
        assert!(p.contains("plat: ndolé"), "{p}");
    }

    #[test]
    fn en_tete_frais_conscience_et_attentes() {
        let conn = store::open(":memory:").unwrap();
        assert_eq!(en_tete_frais(&conn, None), "");
        assert_eq!(
            en_tete_frais(&conn, Some("en appel video, tu vois quelqu'un")),
            " | en appel video, tu vois quelqu'un"
        );

        store::create_pending(&conn, "envoyer_message", r#"{"a":"Paul"}"#).unwrap();
        let s = en_tete_frais(&conn, None);
        assert!(s.contains("ACTIONS EN ATTENTE"), "{s}");
        assert!(s.contains("#1 envoyer_message"), "{s}");
    }

    #[test]
    fn delta_visuel_avance_et_saute_les_vu() {
        let conn = store::open(":memory:").unwrap();
        let (d, cur) = delta_visuel(&conn, 1, 0);
        assert_eq!((d.as_str(), cur), ("", 0));

        store::visual_memory_add(&conn, 1, "evenement", "L'utilisateur sort du champ").unwrap();
        store::visual_memory_add(&conn, 1, "vu", "reponse d'un tour").unwrap();
        store::visual_memory_add(&conn, 1, "evenement", "L'utilisateur apparait a la camera").unwrap();
        let (d, cur) = delta_visuel(&conn, 1, 0);
        assert!(d.starts_with(" | depuis ton dernier tour : "), "{d}");
        assert!(d.contains("L'utilisateur sort du champ ("), "{d}");
        assert!(d.contains("utilisateur apparait"), "{d}");
        // Les 'vu' d'un tour ne se re-racontent pas (deja dans la fenetre).
        assert!(!d.contains("reponse d'un tour"), "{d}");
        assert_eq!(cur, 3);

        // Curseur avance : plus rien ensuite.
        let (d2, cur2) = delta_visuel(&conn, 1, cur);
        assert_eq!((d2.as_str(), cur2), ("", 3));
    }

    #[test]
    fn memoire_visuelle_survit_et_horodate() {
        let conn = store::open(":memory:").unwrap();
        assert_eq!(memoire_visuelle(&conn, 1), "");

        store::visual_memory_add(&conn, 1, "evenement", "L'utilisateur apparait a la camera").unwrap();
        store::visual_memory_add(&conn, 1, "vu", "Je vois un homme sans t-shirt.").unwrap();
        store::visual_memory_add(&conn, 2, "vu", "autre session").unwrap();

        let bloc = memoire_visuelle(&conn, 1);
        assert!(bloc.contains("MEMOIRE VISUELLE"), "{bloc}");
        // Les 'vu' sont CITES (le « tu » de la réponse vise l'utilisateur).
        assert!(bloc.contains("tu as regarde et repondu : « Je vois un homme sans t-shirt. »"), "{bloc}");
        assert!(!bloc.contains("autre session"), "{bloc}");
        // Horodatage HH:MM présent sur chaque ligne d'observation (le bloc
        // commence par \n : ligne vide + en-tête avant les entrées).
        assert!(bloc.lines().skip(2).all(|l| l.starts_with("- [")), "{bloc}");

        let r = store::visual_memory_recent(&conn, 1, 6).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].1, "evenement");
    }

    #[test]
    fn compacte_borne_sur_char() {
        assert_eq!(compacte("court", 10), "court");
        let long = "é".repeat(20);
        let c = compacte(&long, 10);
        assert_eq!(c.chars().count(), 11);
    }
}
