//! Vision ADAPTATIVE — Waly s'ajuste à ce que le modèle sait faire
//! (2026-09-10, stratégie open source « point 2 » ; ADR
//! `docs/ADR-2026-09-10-vision-adaptative.md`).
//!
//! Le cerveau unique texte+vision (qwen3vl sur NPU) reste la voie royale,
//! mais Waly ne doit dépendre d'aucune machine : sur Ollama/Vulkan AMD,
//! qwen3-vl plante ; gemma3 voit mais refuse les outils. Trois modes,
//! résolus UNE fois par processus :
//!
//! - [`Mode::Directe`] : le cerveau voit (ou il est inconnu — FLM, autre
//!   serveur : comportement historique) → les images lui vont telles quelles.
//! - [`Mode::Deleguee`] : un modèle vision déclaré (`WALY_MODEL_VISION` ›
//!   waly.toml `[llm] modele_vision`) REGARDE l'image et la décrit ; le
//!   cerveau reçoit la description en texte (« la vision comme un sens »).
//! - [`Mode::Absente`] : le cerveau déclare ne pas voir et rien n'est
//!   déclaré → aucune image n'est envoyée ; le modèle reçoit une consigne
//!   d'honnêteté, le journal suggère les modèles installés qui voient.
//!
//! Un SEUL point d'application : [`rendre_visible`], appelé au début de
//! chaque round de la boucle de tour — les six chemins d'image (outils
//! `regarder`/`regarder_ecran`, raccourcis desktop/voix, écran) y passent.

use std::sync::Mutex;

use crate::llm::{LlmClient, Msg, Turn};

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Directe,
    Deleguee { modele: String },
    Absente { suggestion: Option<String> },
}

/// Consigne du modèle vision délégué : il est l'ŒIL, pas l'interlocuteur.
const CONSIGNE_OEIL: &str = "Tu es l'oeil de Waly. Decris precisement et \
factuellement ce que montre l'image, en francais, en 2 a 3 phrases : \
personnes (attitude, geste), objets, texte lisible, application ou document \
a l'ecran. Tiens compte de la demande fournie en contexte. N'invente rien ; \
si un detail est illisible, dis-le.";

/// Résolution PURE (testable) : déclaration › capacités sondées › défaut.
pub fn resoudre(
    declare: Option<&str>,
    cerveau: &str,
    capacites: Option<&[String]>,
    suggestion: impl FnOnce() -> Option<String>,
) -> Mode {
    if let Some(m) = declare.map(str::trim).filter(|m| !m.is_empty()) {
        return if m == cerveau {
            Mode::Directe
        } else {
            Mode::Deleguee { modele: m.to_string() }
        };
    }
    match capacites {
        Some(c) if !c.iter().any(|x| x == "vision") => Mode::Absente { suggestion: suggestion() },
        _ => Mode::Directe,
    }
}

/// Le mode de CE processus (sondé à la première image, puis gardé tant que
/// le cerveau ne change pas — le menu des modèles peut le changer en route).
pub fn mode() -> Mode {
    static M: Mutex<Option<(String, Mode)>> = Mutex::new(None);
    let nom = crate::llm::modele_par_defaut();
    let mut garde = M.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((n, m)) = garde.as_ref() {
        if *n == nom {
            return m.clone();
        }
    }
    let m = {
        let cerveau = LlmClient::new("127.0.0.1", crate::llm::port_par_defaut(), &nom);
        let declare = std::env::var("WALY_MODEL_VISION")
            .ok()
            .or_else(|| crate::config::valeur("llm", "modele_vision"));
        let caps = cerveau.capacites();
        let m = resoudre(declare.as_deref(), &cerveau.model, caps.as_deref(), || {
            let v = cerveau.modeles_qui_voient();
            (!v.is_empty()).then(|| v.join(", "))
        });
        eprintln!("[vision] {}", decrire(&m, &cerveau.model));
        m
    };
    *garde = Some((nom, m.clone()));
    m
}

/// Une phrase lisible sur le mode (journaux, diagnostic).
pub fn decrire(m: &Mode, cerveau: &str) -> String {
    match m {
        Mode::Directe => format!("images envoyees directement au cerveau {cerveau}"),
        Mode::Deleguee { modele } => {
            format!("le cerveau {cerveau} ne voit pas : {modele} decrit les images pour lui")
        }
        Mode::Absente { suggestion: Some(s) } => format!(
            "le cerveau {cerveau} ne voit pas et aucun modele vision n'est declare \
             ([llm] modele_vision dans waly.toml) — installes et capables de voir : {s}"
        ),
        Mode::Absente { suggestion: None } => format!(
            "le cerveau {cerveau} ne voit pas et aucun modele vision n'est declare \
             ([llm] modele_vision dans waly.toml)"
        ),
    }
}

/// État lisible de la vision de CE processus (panneau « Ta machine »).
pub fn etat() -> String {
    decrire(&mode(), &crate::llm::modele_par_defaut())
}

/// Client pour un tour PUREMENT visuel (moments proactifs) : le cerveau
/// s'il voit, le modèle délégué sinon ; `None` si personne ne voit.
pub fn client_vision(base: &LlmClient) -> Option<LlmClient> {
    match mode() {
        Mode::Directe => Some(LlmClient::new(&base.host, base.port, &base.model)),
        Mode::Deleguee { modele } => Some(LlmClient::new(&base.host, base.port, &modele)),
        Mode::Absente { .. } => None,
    }
}

/// Fait regarder une image par le modèle délégué ; rend sa description.
pub fn legender(modele: &str, texte: &str, data_url: &str) -> Result<String, String> {
    let mut c = LlmClient::new("127.0.0.1", crate::llm::port_par_defaut(), modele);
    c.max_tokens = 150;
    let msgs = [
        Msg::System(CONSIGNE_OEIL.into()),
        Msg::UserImage {
            texte: format!("Contexte (la demande de l'utilisateur) : {texte}"),
            data_url: data_url.to_string(),
        },
    ];
    match c.chat(&msgs, &[])? {
        Turn::Text(t) if !t.trim().is_empty() => Ok(t.trim().to_string()),
        _ => Err("le modele vision n'a rien decrit".into()),
    }
}

/// Convertit chaque image en TEXTE via `legender` (pur, testable) : le
/// message devient `texte + [CE QUE TU VOIS : …]`, ou une consigne
/// d'honnêteté si la vision échoue.
pub fn convertir(messages: &mut [Msg], mut legender: impl FnMut(&str, &str) -> Result<String, String>) {
    for m in messages.iter_mut() {
        if let Msg::UserImage { texte, data_url } = m {
            let ajout = match legender(texte, data_url) {
                Ok(desc) => format!(
                    "\n[CE QUE TU VOIS (ton sens de la vue a regarde l'image pour toi) : {desc}]"
                ),
                Err(e) => format!(
                    "\n[VISION INDISPONIBLE ({e}) : dis-le honnetement, n'invente \
                     jamais ce que montrerait l'image]"
                ),
            };
            *m = Msg::User(format!("{texte}{ajout}"));
        }
    }
}

/// Le point unique : avant d'envoyer au cerveau, rendre visibles les images
/// qu'il ne sait pas voir. Sans image, aucune sonde n'est déclenchée.
pub fn rendre_visible(messages: &mut [Msg]) {
    if !messages.iter().any(|m| matches!(m, Msg::UserImage { .. })) {
        return;
    }
    match mode() {
        Mode::Directe => {}
        Mode::Deleguee { modele } => convertir(messages, |t, d| legender(&modele, t, d)),
        Mode::Absente { .. } => convertir(messages, |_, _| {
            Err("aucun modele capable de voir n'est configure".into())
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn declaration_prime_et_egale_au_cerveau_donne_directe() {
        let c = caps(&["completion", "tools"]);
        assert_eq!(
            resoudre(Some("gemma3:4b"), "qwen", Some(&c), || None),
            Mode::Deleguee { modele: "gemma3:4b".into() }
        );
        assert_eq!(resoudre(Some(" qwen "), "qwen", Some(&c), || None), Mode::Directe);
        // Déclaration vide = absente.
        assert!(matches!(resoudre(Some("  "), "qwen", Some(&c), || None), Mode::Absente { .. }));
    }

    #[test]
    fn capacites_sondees_ou_inconnues() {
        let voit = caps(&["completion", "vision", "tools"]);
        assert_eq!(resoudre(None, "m", Some(&voit), || None), Mode::Directe);
        // Inconnu (FLM, autre serveur) : comportement historique.
        assert_eq!(resoudre(None, "m", None, || None), Mode::Directe);
        let aveugle = caps(&["completion", "tools"]);
        assert_eq!(
            resoudre(None, "m", Some(&aveugle), || Some("gemma3:4b".into())),
            Mode::Absente { suggestion: Some("gemma3:4b".into()) }
        );
    }

    #[test]
    fn convertir_remplace_les_images_par_du_texte() {
        let mut m = vec![
            Msg::System("s".into()),
            Msg::UserImage { texte: "regarde".into(), data_url: "data:x".into() },
            Msg::User("u".into()),
        ];
        let mut vus = Vec::new();
        convertir(&mut m, |t, d| {
            vus.push((t.to_string(), d.to_string()));
            Ok("un mug rouge".into())
        });
        assert_eq!(vus, vec![("regarde".to_string(), "data:x".to_string())]);
        match &m[1] {
            Msg::User(t) => assert!(t.starts_with("regarde\n[CE QUE TU VOIS") && t.contains("un mug rouge")),
            autre => panic!("attendu User, eu {autre:?}"),
        }
        assert_eq!(m[2], Msg::User("u".into()));
    }

    #[test]
    fn convertir_echec_donne_une_consigne_d_honnetete() {
        let mut m = vec![Msg::UserImage { texte: "t".into(), data_url: "d".into() }];
        convertir(&mut m, |_, _| Err("panne".into()));
        match &m[0] {
            Msg::User(t) => assert!(t.contains("VISION INDISPONIBLE (panne)") && t.contains("honnetement")),
            autre => panic!("attendu User, eu {autre:?}"),
        }
    }

    #[test]
    fn rendre_visible_sans_image_ne_touche_rien() {
        let mut m = vec![Msg::User("u".into())];
        rendre_visible(&mut m); // aucune sonde réseau : pas d'image
        assert_eq!(m, vec![Msg::User("u".into())]);
    }

    #[test]
    fn decrire_les_trois_modes() {
        assert!(decrire(&Mode::Directe, "q").contains("directement"));
        assert!(decrire(&Mode::Deleguee { modele: "g".into() }, "q").contains("g decrit"));
        assert!(decrire(&Mode::Absente { suggestion: Some("g".into()) }, "q").contains("capables de voir : g"));
    }
}
