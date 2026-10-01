//! E2E des mains d'écran (B, 2026-09-11) SANS l'app : lecture UIA de la
//! fenêtre choisie → vrai tour LLM (moteur local, catalogue = mains d'écran)
//! → demandes d'approbation au libellé humain → (--oui) approbation EXPLICITE
//! simulée, comme un clic sur « Approuver » → exécution → relecture.
//!
//! Même hôte que le desktop (`waly_sight::mains::HoteUia`). À lancer sur une
//! fenêtre À SOI (banc : `lab/ecran-banc/cible-banc.ps1`), jamais sur celles
//! de l'utilisateur.
//!
//! Build (WSL) : CARGO_TARGET_DIR=~/waly-target-wsl cargo build \
//!   --target x86_64-pc-windows-gnu -p waly-sight --features mains --example agent_ecran
//! Usage (Windows) : agent_ecran --titre "Cible banc Waly" [--oui] [--tours N] "consigne"

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI64};
use std::sync::Arc;
use std::time::Instant;

use waly_core::llm::{LlmClient, Msg};
use waly_core::{chat, mains_ecran, store, tools::Registry};
use waly_sight::mains::HoteUia;

const SYSTEME: &str = "Tu es Waly, l'assistant local de ton utilisateur. Pendant \
son partage d'écran, tu lis sa fenêtre en texte (lecture fournie) et tu peux y \
agir avec agir_ecran — chaque action est approuvée par lui avant d'être faite. \
Une action par élément ; enchaîne les actions nécessaires. Réponds en français, bref.";

fn opt(args: &[String], nom: &str) -> Option<String> {
    args.iter().position(|a| a == nom).and_then(|i| args.get(i + 1)).cloned()
}

fn court(s: &str) -> String {
    let s = s.replace('\n', " ⏎ ");
    if s.chars().count() > 160 {
        format!("{}…", s.chars().take(160).collect::<String>())
    } else {
        s
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let titre = opt(&args, "--titre").expect("--titre requis");
    let oui = args.iter().any(|a| a == "--oui");
    let tours: usize = opt(&args, "--tours").and_then(|t| t.parse().ok()).unwrap_or(4);
    let consigne = args.last().cloned().expect("consigne requise");

    let (hwnd, nom) = waly_sight::screen::list_windows()
        .into_iter()
        .find(|(_, t)| t.to_lowercase().contains(&titre.to_lowercase()))
        .expect("fenêtre introuvable");
    println!("CIBLE « {nom} » (hwnd {hwnd})");
    let hote = Rc::new(HoteUia::new(Arc::new(AtomicBool::new(true)), Arc::new(AtomicI64::new(hwnd as i64))));
    let mut registry = Registry::new();
    mains_ecran::register_mains_ecran(&mut registry, hote.clone());
    let conn = store::open(":memory:").expect("base mémoire");
    let llm = LlmClient::new("127.0.0.1", waly_core::llm::port_par_defaut(), &waly_core::llm::modele_par_defaut());
    println!("CERVEAU {}:{} {}", llm.host, llm.port, llm.model);

    let mut messages = vec![Msg::System(SYSTEME.into())];
    let mut message = consigne.clone();
    for tour in 1..=tours {
        // B1 comme le desktop : lecture fraîche, une seule vivante.
        let t0 = Instant::now();
        let inst = hote.instantane().expect("lecture");
        println!(
            "\n── tour {tour} ── LECTURE {} éléments, {} car. ({}), {} ms",
            inst.elements.len(),
            inst.texte.chars().count(),
            inst.mode,
            t0.elapsed().as_millis()
        );
        mains_ecran::degrader_lectures(&mut messages);
        messages.push(Msg::User(format!(
            "{}\n{}\n{message}",
            mains_ecran::bloc(&inst.fenetre, &inst.texte),
            mains_ecran::CONSIGNE
        )));
        let avant = store::list_pending(&conn).unwrap().len();
        let t1 = Instant::now();
        let rep = chat::run_turn(&llm, &registry, &mut messages, Some(&conn), |name, out| {
            println!("  outil {name} → {}", court(out))
        });
        println!("  WALY ({:.1} s) : {}", t1.elapsed().as_secs_f32(), court(&rep.unwrap_or_else(|e| format!("ERREUR {e}"))));
        let attentes = store::list_pending(&conn).unwrap();
        if attentes.len() == avant && attentes.is_empty() {
            println!("FIN : plus rien à approuver");
            break;
        }
        if !oui {
            for p in &attentes {
                let a: serde_json::Value = serde_json::from_str(&p.tool_args).unwrap_or_default();
                println!("  DEMANDE #{} : {}", p.id, a["libelle"].as_str().unwrap_or(&p.tool_args));
            }
            println!("(sans --oui : rien n'est exécuté)");
            break;
        }
        let mut constats = Vec::new();
        for p in attentes {
            let a: serde_json::Value = serde_json::from_str(&p.tool_args).unwrap_or_default();
            let libelle = a["libelle"].as_str().unwrap_or("?").to_string();
            let t2 = Instant::now();
            let issue = registry.resolve_one(&conn, p.id, true);
            println!("  APPROUVÉ #{} « {libelle} » → {} ({} ms)", p.id, court(&issue), t2.elapsed().as_millis());
            constats.push(format!("« {libelle} » : {issue}"));
        }
        // Comme après un clic « Approuver » : Waly apprend l'issue et continue.
        message = format!("(J'ai approuvé. Résultat : {}.) Continue si la tâche n'est pas finie, sinon dis-le.", constats.join(" ; "));
    }
    let fin = hote.instantane().expect("relecture");
    println!("\nRELECTURE FINALE « {} » :\n{}", fin.fenetre, fin.texte);
}
