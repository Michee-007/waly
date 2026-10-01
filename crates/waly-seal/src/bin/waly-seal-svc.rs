//! `waly-seal-svc.exe` — le service scelleur de « Huis clos » (R6a).
//!
//! Usage :
//!   waly-seal-svc install    installe le service (élévation requise)
//!   waly-seal-svc uninstall  arrête et supprime le service (élévation)
//!   waly-seal-svc start      démarre le service
//!   waly-seal-svc run        lancé par le SCM (ne pas appeler à la main)
//!   waly-seal-svc console    sert le pipe en avant-plan (debug)
//!   waly-seal-svc probe <json>  envoie une requête au pipe et imprime la
//!                               réponse (diagnostic terrain, ex.
//!                               probe '{"cmd":"ping"}')

/// Sel de reroll SAC (piège 3) : un nouveau build peut se faire bloquer par
/// Smart App Control (verdict par binaire, imprévisible). Incrémenter change le
/// hash → nouveau verdict. (Le service final sera signé.)
const SAC_REROLL: u32 = 1;

#[cfg(not(windows))]
fn main() {
    eprintln!("waly-seal-svc : Windows uniquement");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    use waly_seal::service;
    std::hint::black_box(SAC_REROLL);
    let arg = std::env::args().nth(1).unwrap_or_default();
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    let r = match arg.as_str() {
        // setup = la commande de l'installeur (élevée une fois) : relocalise
        // l'exe dans Program Files (hors écriture utilisateur), enregistre en
        // AUTO_START, démarre. C'est elle que le NSIS appelle en `runas`.
        "setup" => service::setup(&exe),
        "install" => service::install(&exe),
        "uninstall" => service::uninstall(),
        "start" => service::start(),
        "run" => service::run_service(),
        "console" => service::run_console(),
        "probe" => {
            let json = std::env::args().nth(2).unwrap_or_else(|| r#"{"cmd":"ping"}"#.into());
            match probe(&json) {
                Ok(rep) => {
                    println!("{rep}");
                    return;
                }
                Err(e) => Err(e),
            }
        }
        // --- Brique « Huis clos universel » (chantier C) : sceller n'importe
        // quel agent local. Sceller un exe TIERS exige une console ELEVEE
        // (le service verifie l'elevation du client — ADR 2026-09-16).
        "seal" => {
            let exes: Vec<String> = std::env::args().skip(2).collect();
            match cmd_seal(&exes) {
                Ok(()) => return,
                Err(e) => Err(e),
            }
        }
        "unseal" => match std::env::args().nth(2) {
            Some(exe) => match cmd_unseal(&exe) {
                Ok(()) => return,
                Err(e) => Err(e),
            },
            None => Err("usage: waly-seal-svc unseal <chemin-exe>".into()),
        },
        "list" => match cmd_list() {
            Ok(()) => return,
            Err(e) => Err(e),
        },
        "journal" => match std::env::args().nth(2) {
            Some(exe) => match cmd_journal(&exe) {
                Ok(()) => return,
                Err(e) => Err(e),
            },
            None => Err("usage: waly-seal-svc journal <chemin-exe>".into()),
        },
        other => {
            eprintln!(
                "commande inconnue: {other:?}\n  service : setup|install|uninstall|start|run|console\n  sceau   : seal <exe...>|unseal <exe>|list|journal <exe>|probe <json>"
            );
            std::process::exit(2);
        }
    };
    if let Err(e) = r {
        eprintln!("{arg}: {e}");
        std::process::exit(1);
    }
    if arg != "run" && arg != "console" {
        println!("{arg}: OK");
    }
}

/// Client de diagnostic : ouvre le pipe, envoie une ligne JSON, lit la réponse.
#[cfg(windows)]
fn probe(json: &str) -> Result<String, String> {
    use std::io::{Read, Write};
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(waly_seal::ipc::PIPE_NAME)
        .map_err(|e| format!("pipe: {e}"))?;
    f.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    f.write_all(b"\n").map_err(|e| e.to_string())?;
    f.flush().ok();
    let mut ligne = Vec::new();
    let mut b = [0u8; 1];
    loop {
        match f.read(&mut b) {
            Ok(0) => break,
            Ok(_) if b[0] == b'\n' => break,
            Ok(_) => ligne.push(b[0]),
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(String::from_utf8_lossy(&ligne).into_owned())
}

// --- Sous-commandes de la brique (chantier C) ------------------------------

/// Envoie une requête typée au service et renvoie la réponse typée.
#[cfg(windows)]
fn envoyer(req: &waly_seal::ipc::Requete) -> Result<waly_seal::ipc::Reponse, String> {
    let json = serde_json::to_string(req).map_err(|e| e.to_string())?;
    let brut = probe(&json)?;
    serde_json::from_str(&brut).map_err(|e| format!("reponse illisible: {e} ({brut})"))
}

/// Affiche une erreur de refus d'élévation de façon lisible (le cas le plus
/// courant : l'utilisateur a lancé la commande depuis une console non élevée).
#[cfg(windows)]
fn afficher_erreur(message: &str) -> Result<(), String> {
    if message.contains("elevation requise") {
        eprintln!("REFUSE — {message}");
        eprintln!("  -> ouvre une invite de commandes/PowerShell « en tant qu'administrateur » et relance.");
    } else {
        eprintln!("REFUSE — {message}");
    }
    Err(message.to_string())
}

#[cfg(windows)]
fn cmd_seal(exes: &[String]) -> Result<(), String> {
    use waly_seal::ipc::{ressemble_interpreteur, session_pour_exe, Reponse, Requete, Succes};
    if exes.is_empty() {
        return Err("usage: waly-seal-svc seal <chemin-exe> [<chemin-exe>...]".into());
    }
    let mut echec = false;
    for exe in exes {
        if ressemble_interpreteur(exe) {
            eprintln!(
                "ATTENTION: {exe} ressemble a un interpreteur PARTAGE — le sceller couperait le reseau de TOUS les programmes qui l'utilisent, pas seulement l'agent vise."
            );
        }
        let session = session_pour_exe(exe);
        match envoyer(&Requete::Sceller { session, exes: vec![exe.clone()] })? {
            Reponse::Ok(Succes::Scelle { filtres, .. }) => {
                if filtres == 0 {
                    println!("{exe} : 0 filtre (exe introuvable ? le sceau s'appliquera des qu'il existera) [session {session}]");
                } else {
                    println!("SCELLE {exe} — {filtres} filtres, sortie reseau bloquee [session {session}]");
                }
            }
            Reponse::Ok(_) => println!("{exe} : reponse inattendue"),
            Reponse::Err { message } => {
                let _ = afficher_erreur(&message);
                echec = true;
            }
        }
    }
    if echec {
        Err("au moins un scelle a echoue".into())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn cmd_unseal(exe: &str) -> Result<(), String> {
    use waly_seal::ipc::{session_pour_exe, Reponse, Requete, Succes};
    let session = session_pour_exe(exe);
    match envoyer(&Requete::Desceller { session })? {
        Reponse::Ok(Succes::Fait { .. }) => {
            println!("DESCELLE {exe} — sortie reseau restauree [session {session}]");
            Ok(())
        }
        Reponse::Ok(_) => Ok(()),
        Reponse::Err { message } => afficher_erreur(&message),
    }
}

#[cfg(windows)]
fn cmd_list() -> Result<(), String> {
    use waly_seal::ipc::{Reponse, Requete, Succes};
    match envoyer(&Requete::Etat)? {
        Reponse::Ok(Succes::Etat { sessions, version, privilegie }) => {
            println!("waly-seal v{version} — service {} de sceller", if privilegie { "CAPABLE" } else { "NON PRIVILEGIE (mode degrade)" });
            if sessions.is_empty() {
                println!("aucune session scellee");
            } else {
                println!("{} session(s) scellee(s) :", sessions.len());
                for s in sessions {
                    let quoi = if s == waly_seal::ipc::PERIMETRE {
                        "perimetre Waly (auto-scelle)".to_string()
                    } else if s > 0 {
                        format!("session Waly {s}")
                    } else {
                        format!("agent tiers (cle {s})")
                    };
                    println!("  - {quoi}");
                }
            }
            Ok(())
        }
        Reponse::Ok(_) => Ok(()),
        Reponse::Err { message } => afficher_erreur(&message),
    }
}

#[cfg(windows)]
fn cmd_journal(exe: &str) -> Result<(), String> {
    use waly_seal::ipc::{session_pour_exe, Reponse, Requete, Succes};
    let session = session_pour_exe(exe);
    match envoyer(&Requete::Journal { session })? {
        Reponse::Ok(Succes::Journal { drops }) => {
            if drops.is_empty() {
                println!("aucune sortie bloquee journalisee pour {exe} [session {session}]");
            } else {
                println!("{} sortie(s) bloquee(s) pour {exe} :", drops.len());
                for d in drops {
                    let proto = match d.proto {
                        6 => "TCP",
                        17 => "UDP",
                        _ => "?",
                    };
                    println!("  [{}] {proto} {}:{}", d.t, d.adresse, d.port);
                }
            }
            Ok(())
        }
        Reponse::Ok(_) => Ok(()),
        Reponse::Err { message } => afficher_erreur(&message),
    }
}
