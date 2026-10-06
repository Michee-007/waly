//! Binaire de test du cœur : `waly turn "question"` (un tour, outils actifs)
//! et `waly chat` (REPL texte, historique persisté). Sert à valider la boucle
//! agentique contre FLM avant le branchement voix (R2 ch. 6).
//! Moteur requis : qwen3vl-it:4b port 42626 — WALY_LLM_PORT (cerveau unique texte+vision R4,
//! surcharge WALY_MODEL). Base : C:\waly\data\waly.db
//! (surcharge : WALY_DB, ':memory:' pour un essai jetable).

use std::cell::RefCell;
use std::io::Write as _;
use std::rc::Rc;

use waly_core::chat::run_turn;
use waly_core::embed::{self, Embedder};
use waly_core::llm::{LlmClient, Msg};
use waly_core::native_tools::{register_core_tools, SharedEmbedder};
use waly_core::store;
use waly_core::tools::Registry;

// L'outil sensible de démo (WALY_DEMO_SENSIBLE=1) vit dans native_tools
// depuis le ch. 3 desktop : `EnvoyerMessageDemo` est partagé bin/desktop.
use waly_core::native_tools::EnvoyerMessageDemo;

fn embed_dir_defaut() -> String {
    waly_core::chemins::modele("e5-small-int8")
}

/// Charge l'embedder si le modèle est là ; sans lui, la mémoire retombe sur
/// le mot-clé (dégradation gracieuse, signalée une fois).
fn load_embedder() -> SharedEmbedder {
    let dir = std::env::var("WALY_EMBED_DIR").unwrap_or_else(|_| embed_dir_defaut());
    match Embedder::load(&dir) {
        Ok(e) => Some(Rc::new(RefCell::new(e))),
        Err(err) => {
            eprintln!("(memoire semantique indisponible: {err})");
            None
        }
    }
}

/// Prompt système STABLE : base + souvenirs actifs injectés (pattern de
/// l'ancien monde — le modèle VOIT sa mémoire, il ne doit pas deviner qu'il
/// faut la chercher ; vécu : « comment s'appelle mon animal ? » → « j'ignore
/// son nom » sans même appeler chercher_memoire). ⚠ Append-only (GATE A
/// R4.5) : à ne générer qu'en début de fil, jamais par tour — le cache FLM
/// ne survit qu'à une conversation étendue verbatim.
fn system_prompt(conn: &rusqlite::Connection) -> Msg {
    let u = waly_core::user::designation();
    let p = format!(
        "Tu es Waly, l'assistant personnel local de {u}. Tu réponds en français, \
         simplement et directement, en une à trois phrases. L'horodatage [jour date \
         heure] devant chaque message de {u} te donne la date et l'heure ACTUELLES : \
         tu les connais, réponds directement. Tu as des outils : \
         utilise-les quand ils servent la question, sinon réponds directement. \
         Quand {u} te confie une info durable sur lui, retiens-la avec \
         memoriser sans le lui demander."
    );
    Msg::System(waly_core::prompt::inject_context(&p, conn))
}

fn open_db() -> Rc<rusqlite::Connection> {
    let path = waly_core::store::chemin_par_defaut();
    if path != ":memory:" {
        if let Some(dir) = std::path::Path::new(&path).parent() {
            std::fs::create_dir_all(dir).expect("creation du dossier data");
        }
    }
    Rc::new(store::open(&path).expect("ouverture de la base"))
}

/// Fournisseur de capture d'écran de BANC : un PNG (WALY_ECRAN_FAKE) + un
/// texte OCR simulé (WALY_ECRAN_OCR). La vraie capture+OCR (waly-sight) est
/// branchée par l'hôte au ch. 3 — waly-core reste découplé de waly-sight.
fn fake_ecran_fournisseur() -> Option<waly_core::native_tools::FournisseurEcran> {
    let png = std::env::var("WALY_ECRAN_FAKE").ok()?;
    let ocr = std::env::var("WALY_ECRAN_OCR").unwrap_or_default();
    Some(Box::new(move |_cadrage| {
        let image = waly_core::native_tools::image_depuis_fichier(&png)?;
        Ok(waly_core::native_tools::CaptureEcran { image, texte_ocr: ocr.clone(), conf: 0.9 })
    }))
}

/// `waly materiel` : profil de la machine + modèle recommandé (point 3 de la
/// stratégie open source). Waly ne télécharge rien : il donne les gestes.
fn materiel() {
    let p = waly_core::materiel::detecter();
    println!("Memoire : {:.1} Go ({:.1} Go libres)", p.ram_totale_go, p.ram_dispo_go);
    if p.gpus.is_empty() {
        println!("Carte graphique : non detectee");
    }
    for g in &p.gpus {
        let genre = if g.integre { "integree" } else { "dediee" };
        println!("Carte graphique : {} — {genre}, {:.1} Go reserves", g.nom, g.vram_go);
    }
    println!("NPU : {}", p.npu.as_deref().unwrap_or("aucun"));
    println!("Smart App Control : {}", if p.sac_actif { "actif" } else { "inactif" });
    let installes =
        LlmClient::new("127.0.0.1", waly_core::llm::PORT_MOTEUR_B, "").modeles_installes();
    let r = waly_core::materiel::recommander(&p, &installes);
    let vision = if r.cerveau_voit {
        "le cerveau voit lui-meme".to_string()
    } else {
        r.modele_vision.clone().unwrap_or_else(|| "aucune".into())
    };
    println!("\nRecommande : cerveau {} ({}), vision : {vision}", r.cerveau, r.moteur);
    for x in &r.raisons {
        println!("  - {x}");
    }
    println!("A faire :");
    for x in &r.a_faire {
        println!("  {x}");
    }
    println!("\nBloc waly.toml :\n{}", r.toml);
    println!(
        "Actuel : {} — {}",
        waly_core::llm::modele_par_defaut(),
        waly_core::vision::etat()
    );
}

/// `waly enclos …` : l'enclos de la Garde, sans l'interface.
///   etat | essai
///   creer [chemin-du-service]        (Windows demande l'accord)
///   donner <dossier> [ecriture]      couper <dossier>      reprendre <dossier>
///   lancer <programme> [arguments…]  arreter <programme>
fn enclos(a: &[String]) {
    use waly_core::enclos::{self, Droit};
    let conn = match store::open(&store::chemin_par_defaut()) {
        Ok(c) => c,
        Err(e) => return eprintln!("base : {e}"),
    };
    let dire = |d: &enclos::Dossier| {
        let essai = match (d.lit, d.ecrit) {
            (Some(l), Some(e)) => format!("essai : {}, {}", if l { "lit" } else { "ne lit pas" }, if e { "écrit" } else { "n'écrit pas" }),
            _ => "pas essayé".into(),
        };
        let tenu = match d.conforme() {
            Some(true) => "TENU",
            Some(false) => "NON TENU",
            None => "?",
        };
        println!("  {:9} {}  [{essai} -> {tenu}] {}", d.droit.cle(), d.chemin, d.pourquoi);
    };
    let regler = |chemin: Option<&String>, droit: Option<Droit>| match chemin {
        None => eprintln!("il manque le dossier"),
        Some(c) => match enclos::regler(&conn, c, droit, "") {
            Ok(Some(d)) => dire(&d),
            Ok(None) => println!("  repris : {c}"),
            Err(e) => eprintln!("refusé : {e}"),
        },
    };
    match a.first().map(String::as_str) {
        Some("creer") => {
            let r = match a.get(1) {
                Some(svc) => enclos::creer_avec(&conn, svc),
                None => enclos::creer(&conn),
            };
            match r {
                Ok(()) => println!("enclos créé (compte {})", enclos::COMPTE),
                Err(e) => eprintln!("échec : {e}"),
            }
        }
        Some("donner") => regler(a.get(1), Some(if a.get(2).map(String::as_str) == Some("ecriture") { Droit::Ecriture } else { Droit::Lecture })),
        Some("couper") => regler(a.get(1), Some(Droit::Coupe)),
        Some("reprendre") => regler(a.get(1), None),
        Some("essai") => match enclos::tout_essayer(&conn) {
            Ok(invisible) => {
                println!("ton profil ({}) : {}", enclos::profil(), if invisible { "invisible pour l'enclos (essai)" } else { "LISIBLE par l'enclos" });
                enclos::dossiers(&conn).iter().for_each(dire);
            }
            Err(e) => eprintln!("échec : {e}"),
        },
        Some("lancer") if a.len() > 1 => {
            let ligne = a[1..].iter().map(|m| if m.contains(' ') { format!("\"{m}\"") } else { m.clone() }).collect::<Vec<_>>().join(" ");
            let nom = std::path::Path::new(&a[1]).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            match enclos::lancer(&conn, &nom, &a[1], &ligne) {
                Ok(pid) => println!("lancé dans l'enclos : PID {pid}"),
                Err(e) => eprintln!("échec : {e}"),
            }
        }
        Some("arreter") if a.len() > 1 => println!("{} processus arrêté(s)", enclos::arreter(&conn, &a[1], true)),
        _ => {
            println!("compte {} : {}", enclos::COMPTE, if enclos::existe() { if enclos::pret(&conn) { "prêt" } else { "existe, mais Waly n'a pas son secret (« waly enclos creer »)" } } else { "absent" });
            if let Some((invisible, quand)) = enclos::essai_profil(&conn) {
                println!("ton profil : {} (essai du {quand})", if invisible { "invisible pour l'enclos" } else { "LISIBLE par l'enclos" });
            }
            enclos::dossiers(&conn).iter().for_each(dire);
            for ag in enclos::agents(&conn) {
                println!("  agent {} : {} processus {:?}", ag.nom, ag.pids.len(), ag.pids);
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("embed-bench") {
        return embed_bench();
    }
    if args.get(1).map(String::as_str) == Some("prompt-tokens") {
        return prompt_tokens();
    }
    if args.get(1).map(String::as_str) == Some("materiel") {
        return materiel();
    }
    if args.get(1).map(String::as_str) == Some("enclos") {
        return enclos(&args[2..]);
    }
    let llm = LlmClient::new("127.0.0.1", waly_core::llm::port_par_defaut(),&waly_core::llm::modele_par_defaut());
    let conn = open_db();
    let mut registry = Registry::new();
    register_core_tools(&mut registry, conn.clone(), load_embedder());
    waly_core::fichiers::register_fichier_tools(
        &mut registry,
        waly_core::fichiers::PolitiqueFichiers::defaut(),
    );
    // Serveurs MCP stdio déclarés dans waly.toml (jamais bloquant).
    for ligne in waly_core::mcp::register_mcp_tools(&mut registry) {
        eprintln!("[mcp] {ligne}");
    }
    if std::env::var("WALY_DEMO_SENSIBLE").as_deref() == Ok("1") {
        registry.register(Box::new(EnvoyerMessageDemo));
    }
    // Outil vision (R4 ch. 4) : au banc, la « caméra » du bin est un fichier
    // (WALY_CAM_FAKE) — la vraie caméra arrive par le desktop/voix (ch. 5).
    if let Ok(fake) = std::env::var("WALY_CAM_FAKE") {
        waly_core::native_tools::register_vision_tool(
            &mut registry,
            Box::new(move || waly_core::native_tools::image_depuis_fichier(&fake)),
        );
    }
    // Outil vision ÉCRAN (R5 ch. 2) : « écran » de banc = un PNG (WALY_ECRAN_FAKE)
    // + texte OCR simulé (WALY_ECRAN_OCR) — la vraie capture+OCR arrive par le
    // mode « Écran » desktop (ch. 3).
    if let Some(f) = fake_ecran_fournisseur() {
        waly_core::native_tools::register_vision_ecran_tool(&mut registry, f);
    }

    match args.get(1).map(String::as_str) {
        Some("ecran") => {
            // Raccourci OCR-first (ch. 2) : intention_ecran → capture → fusion.
            // Registre VIDE = tour ZÉRO outil (discipline R4.5, le modèle ne
            // peut pas re-regarder en boucle).
            let question = args
                .get(2)
                .cloned()
                .unwrap_or_else(|| "Qu'est-ce qui est affiché à l'écran ?".into());
            let Some(fournisseur) = fake_ecran_fournisseur() else {
                eprintln!("définir WALY_ECRAN_FAKE=chemin.png (et WALY_ECRAN_OCR=\"texte\")");
                return;
            };
            let cadrage = waly_core::chat::intention_ecran(&question)
                .unwrap_or(waly_core::native_tools::CadrageEcran::Fenetre);
            let cap = match fournisseur(cadrage) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("capture écran: {e}");
                    return;
                }
            };
            println!("cadrage: {cadrage:?} | conf OCR: {:.2}", cap.conf);
            let (contexte, image) = waly_core::chat::fusion_ecran(&question, cap);
            println!(
                "mode: {}",
                if image.is_some() { "compréhension (image jointe)" } else { "lecture (OCR seul)" }
            );
            let frais = waly_core::prompt::en_tete_frais(&conn, None);
            // Le contexte OCR va DANS le tour (jamais persisté ici — banc).
            let stamped = format!(
                "[{}{frais}] {contexte}{question}",
                waly_core::clock::french_timestamp()
            );
            let mut messages = vec![system_prompt(&conn)];
            match image {
                Some(data_url) => messages.push(Msg::UserImage {
                    texte: stamped,
                    data_url,
                }),
                None => messages.push(Msg::User(stamped)),
            }
            let empty = Registry::new();
            // Plafond de décodage (GATE 3 R5) : les tours écran sont brefs — le
            // décodage NPU domine, un cap tient le budget (< 3 s / < 8 s).
            let mut llm_ecran =
                LlmClient::new("127.0.0.1", waly_core::llm::port_par_defaut(),&waly_core::llm::modele_par_defaut());
            llm_ecran.max_tokens = 100;
            let t0 = std::time::Instant::now();
            match run_turn(&llm_ecran, &empty, &mut messages, Some(&conn), |name, out| {
                println!("  [outil {name}] {out}");
            }) {
                Ok(text) => {
                    let text = waly_core::llm::BracketFilter::strip(&text);
                    println!("Waly: {text}");
                    println!("(tour: {:.2} s)", t0.elapsed().as_secs_f32());
                }
                Err(e) => eprintln!("erreur: {e}"),
            }
            return;
        }
        Some("turn") => {
            let question = args.get(2).cloned().unwrap_or_else(|| "Quelle heure est-il ?".into());
            // Horodatage prefixe au message (comme la voix) : le modele DOIT
            // connaitre « maintenant » pour calculer les dates de rappel.
            let frais = waly_core::prompt::en_tete_frais(&conn, None);
            let stamped =
                format!("[{}{frais}] {question}", waly_core::clock::french_timestamp());
            let mut messages = vec![system_prompt(&conn), Msg::User(stamped)];
            let t0 = std::time::Instant::now();
            match run_turn(&llm, &registry, &mut messages, Some(&conn), |name, out| {
                println!("  [outil {name}] {out}");
            }) {
                Ok(text) => {
                    // Le modèle singe parfois l'horodatage en tête de réponse.
                    let text = waly_core::llm::BracketFilter::strip(&text);
                    store::append_message(&conn, "user", &question).ok();
                    store::append_message(&conn, "assistant", &text).ok();
                    println!("Waly: {text}");
                    println!("(tour: {:.2} s)", t0.elapsed().as_secs_f32());
                }
                Err(e) => eprintln!("erreur: {e}"),
            }
        }
        Some("chat") => {
            let mut messages = vec![system_prompt(&conn)];
            // Reprendre le fil : les 10 derniers messages persistés.
            for (role, content) in store::recent_messages(&conn, 10).unwrap_or_default() {
                messages.push(match role.as_str() {
                    "user" => Msg::User(content),
                    _ => Msg::Assistant(content),
                });
            }
            println!("Waly (texte). Ligne vide pour sortir.");
            loop {
                print!("> ");
                std::io::stdout().flush().ok();
                let mut line = String::new();
                if std::io::stdin().read_line(&mut line).is_err() || line.trim().is_empty() {
                    break;
                }
                let user = line.trim().to_owned();
                // Append-only (GATE A R4.5) : le systeme ne bouge pas, le
                // frais (attentes) part dans l'en-tete du message.
                let frais = waly_core::prompt::en_tete_frais(&conn, None);
                let stamped =
                    format!("[{}{frais}] {user}", waly_core::clock::french_timestamp());
                let base = messages.len();
                messages.push(Msg::User(stamped));
                let t0 = std::time::Instant::now();
                match run_turn(&llm, &registry, &mut messages, Some(&conn), |name, out| {
                    println!("  [outil {name}] {out}");
                }) {
                    Ok(text) => {
                        // Le modèle singe parfois l'horodatage en tête de réponse.
                        let text = waly_core::llm::BracketFilter::strip(&text);
                        store::append_message(&conn, "user", &user).ok();
                        store::append_message(&conn, "assistant", &text).ok();
                        println!("Waly: {text}");
                        println!("(tour: {:.2} s)", t0.elapsed().as_secs_f32());
                    }
                    Err(e) => {
                        // Nettoyer le tour raté : sinon le user orphelin (et
                        // d'éventuelles paires tool_call incomplètes) polluent
                        // tous les tours suivants (leçon voix).
                        messages.truncate(base);
                        eprintln!("erreur: {e}");
                    }
                }
            }
        }
        _ => {
            eprintln!("usage: waly turn [\"question\"] | waly chat | waly ecran [\"question\"] | waly embed-bench");
        }
    }
}

/// Mesure le budget prompt RÉEL (critère R2 : < 1k tokens) : envoie le
/// prompt système + outils du binaire texte à FLM (non-streaming, 1 token)
/// et lit usage.prompt_tokens — le tokenizer de FLM fait foi.
fn prompt_tokens() {
    let conn = open_db();
    let mut registry = Registry::new();
    register_core_tools(&mut registry, conn.clone(), None);
    waly_core::fichiers::register_fichier_tools(
        &mut registry,
        waly_core::fichiers::PolitiqueFichiers::defaut(),
    );
    // Serveurs MCP stdio déclarés dans waly.toml (jamais bloquant).
    for ligne in waly_core::mcp::register_mcp_tools(&mut registry) {
        eprintln!("[mcp] {ligne}");
    }
    if std::env::var("WALY_DEMO_SENSIBLE").as_deref() == Ok("1") {
        registry.register(Box::new(EnvoyerMessageDemo));
    }
    if let Ok(fake) = std::env::var("WALY_CAM_FAKE") {
        waly_core::native_tools::register_vision_tool(
            &mut registry,
            Box::new(move || waly_core::native_tools::image_depuis_fichier(&fake)),
        );
    }
    let system = system_prompt(&conn);
    let system_len = match &system {
        Msg::System(s) => s.len(),
        _ => 0,
    };
    let mut llm = LlmClient::new("127.0.0.1", waly_core::llm::port_par_defaut(),&waly_core::llm::modele_par_defaut());
    llm.max_tokens = 1;
    let messages = vec![system, Msg::User("ok".into())];
    match llm.chat_with_usage(&messages, &registry.specs()) {
        Ok((_, Some(tokens))) => println!(
            "prompt_tokens = {tokens} (outils: {}, systeme: {system_len} caracteres)",
            registry.specs().len()
        ),
        Ok((_, None)) => eprintln!("FLM n'a pas renvoye usage"),
        Err(e) => eprintln!("erreur: {e}"),
    }
}

/// Banc de l'embedder : latence de chargement/inférence et matrice de
/// cosinus FR (paires proches vs éloignées) pour calibrer SEMANTIC_MIN_COS.
fn embed_bench() {
    let dir = std::env::var("WALY_EMBED_DIR").unwrap_or_else(|_| embed_dir_defaut());
    let t0 = std::time::Instant::now();
    let mut e = match Embedder::load(&dir) {
        Ok(e) => e,
        Err(err) => return eprintln!("erreur: {err}"),
    };
    println!("chargement: {:.2} s", t0.elapsed().as_secs_f32());

    let passages = [
        "plat_prefere: le ndolé",
        "cafe: café noir sans sucre",
        "metier: développeur, construit Waly",
        "velo: fait du vélo le dimanche matin",
        "chat: a un chat qui s'appelle Simba",
    ];
    let queries = [
        "qu'est-ce qu'il aime manger ?",
        "comment prend-il son café ?",
        "quel est son travail ?",
        "que fait-il le week-end ?",
        "a-t-il un animal de compagnie ?",
        "quelle est sa couleur préférée ?", // absent : doit rester sous le seuil
    ];

    let t0 = std::time::Instant::now();
    let pvecs: Vec<Vec<f32>> =
        passages.iter().map(|p| e.embed_passage(p).expect("embed passage")).collect();
    println!(
        "embed {} passages: {:.0} ms/texte",
        passages.len(),
        t0.elapsed().as_millis() as f32 / passages.len() as f32
    );

    println!("\n            {}", (0..passages.len()).map(|i| format!("  p{i}  ")).collect::<String>());
    for (qi, q) in queries.iter().enumerate() {
        let t = std::time::Instant::now();
        let qv = e.embed_query(q).expect("embed query");
        let ms = t.elapsed().as_millis();
        let cos: Vec<String> =
            pvecs.iter().map(|pv| format!("{:+.3}", embed::cosine(&qv, pv))).collect();
        println!("q{qi} ({ms:3} ms) {}   {q}", cos.join(" "));
    }
    println!("\npassages:");
    for (i, p) in passages.iter().enumerate() {
        println!("  p{i}: {p}");
    }
}

