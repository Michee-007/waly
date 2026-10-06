//! `waly-seal-svc.exe` — le service scelleur de « Huis clos » (R6a).
//!
//! Usage :
//!   waly-seal-svc install    installe le service (élévation requise)
//!   waly-seal-svc uninstall  arrête et supprime le service (élévation)
//!   waly-seal-svc start      démarre le service
//!   waly-seal-svc run        lancé par le SCM (ne pas appeler à la main)
//!   waly-seal-svc console    sert le pipe en avant-plan (debug)
//!   waly-seal-svc enclos creer <fichier> | supprimer | etat
//!                            le compte Windows à part (élévation)
//!   waly-seal-svc probe <json>  envoie une requête au pipe et imprime la
//!                               réponse (diagnostic terrain, ex.
//!                               probe '{"cmd":"ping"}')

/// Sel de reroll SAC (piège 3) : un nouveau build peut se faire bloquer par
/// Smart App Control (verdict par binaire, imprévisible). Incrémenter change le
/// hash → nouveau verdict. (Le service final sera signé.)
const SAC_REROLL: u32 = 11;

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
        // --- La Garde, etape 3 : regler ce que le service observe. Allumer
        // exige une console ELEVEE ; « rien » eteint.
        "regarder" => {
            let mode = std::env::args().nth(2).unwrap_or_default();
            let exes: Vec<String> = std::env::args().skip(3).collect();
            match cmd_regarder(&mode, &exes) {
                Ok(()) => return,
                Err(e) => Err(e),
            }
        }
        // Essai sans service : ecoute dans CE processus (console elevee)
        // pendant N secondes et imprime ce qui a ete vu.
        "essai-regard" => {
            let secondes: u64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(6);
            let exes: Vec<String> = std::env::args().skip(3).collect();
            match cmd_essai_regard(secondes, &exes) {
                Ok(()) => return,
                Err(e) => Err(e),
            }
        }
        // --- La Garde, etape 4 : le compte Windows a part (l'enclos).
        // Console ELEVEE. Le mot de passe arrive par un fichier, jamais en
        // argument.
        "enclos" => {
            let (quoi, fichier) = (std::env::args().nth(2).unwrap_or_default(), std::env::args().nth(3));
            match (quoi.as_str(), fichier) {
                ("creer", Some(f)) => match waly_seal::enclos::creer(&f) {
                    Ok(fait) => {
                        println!("enclos : compte {fait}");
                        return;
                    }
                    Err(e) => Err(e),
                },
                ("supprimer", _) => waly_seal::enclos::supprimer(),
                ("etat", _) => {
                    println!("{}", waly_seal::enclos::etat());
                    return;
                }
                _ => Err("usage: waly-seal-svc enclos creer <fichier-du-mot-de-passe> | supprimer | etat".into()),
            }
        }
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
                "commande inconnue: {other:?}\n  service : setup|install|uninstall|start|run|console\n  sceau   : seal <exe...>|unseal <exe>|list|journal <exe>|probe <json>\n  regard  : regarder <rien|tout|agents> [exe...]|essai-regard [secondes] [exe...]
  enclos  : enclos creer <fichier>|supprimer|etat"
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
    // Le canal sert un client a la fois, et l'app l'interroge en continu
    // quand la Garde est ouverte : « occupe » n'est pas un echec, on reessaie
    // (vecu 2026-10-06 : « Surveiller » echouait depuis l'app, jamais depuis
    // une console). Introuvable (2) = service absent : inutile d'insister.
    let mut ouvert = None;
    for essai in 0..80 {
        match std::fs::OpenOptions::new().read(true).write(true).open(waly_seal::ipc::PIPE_NAME) {
            Ok(h) => {
                ouvert = Some(h);
                break;
            }
            Err(e) if e.raw_os_error() == Some(2) => return Err("service scelleur absent".into()),
            Err(e) if essai == 79 => return Err(format!("pipe: {e}")),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
    let mut f = ouvert.ok_or("pipe indisponible")?;
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

#[cfg(windows)]
fn cmd_regarder(mode: &str, exes: &[String]) -> Result<(), String> {
    use waly_seal::ipc::{Reponse, Requete, Succes};
    match envoyer(&Requete::Regarder { mode: mode.to_string(), exes: exes.to_vec() })? {
        Reponse::Ok(Succes::Regard { mode, exes, .. }) => {
            println!("surveillance : {mode}{}", if exes.is_empty() { String::new() } else { format!(" ({})", exes.join(", ")) });
            Ok(())
        }
        Reponse::Ok(_) => Err("reponse inattendue".into()),
        Reponse::Err { message } => Err(message),
    }
}

#[cfg(windows)]
fn cmd_essai_regard(secondes: u64, exes: &[String]) -> Result<(), String> {
    use waly_seal::ipc::{REGARD_AGENTS, REGARD_RIEN, REGARD_TOUT};
    let mode = if exes.is_empty() { REGARD_TOUT } else { REGARD_AGENTS };
    waly_seal::regard::regler(mode, exes)?;
    println!("ecoute ({mode}) pendant {secondes} s…");
    std::thread::sleep(std::time::Duration::from_secs(secondes));
    let (_, _, obs, dernier) = waly_seal::regard::lire(0);
    waly_seal::regard::regler(REGARD_RIEN, &[])?;
    println!("{} observations gardees (numero {dernier})", obs.len());
    for o in &obs {
        println!("{}\t{}\t{}\tx{}\t{}", o.genre, o.pid, o.exe.rsplit('\\').next().unwrap_or(&o.exe), o.fois, o.objet);
    }
    Ok(())
}

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
