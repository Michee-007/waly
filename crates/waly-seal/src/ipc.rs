//! Protocole IPC du service scelleur — **volontairement étroit**.
//!
//! Le client ne peut demander QUE de sceller/desceller/interroger SA session,
//! sur un ensemble d'exes qu'il nomme. **Jamais** de définition de filtre
//! brute : l'app ne peut pas demander un trou dans le sceau. C'est la garantie
//! centrale de la voie service.
//!
//! Transport : named pipe `\\.\pipe\waly-seal`, une requête JSON par ligne,
//! une réponse JSON par ligne (UTF-8, `\n` terminal). Le pipe est ACLé
//! SYSTEM (plein) + utilisateurs interactifs authentifiés (lecture/écriture) —
//! posé par le service (chantier 0b).

use serde::{Deserialize, Serialize};

/// Un évènement de sortie bloquée, prêt pour le journal d'audit. Donnée pure
/// (aucune dépendance Windows) → partagée client/serveur.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sortie {
    /// horodatage unix (s)
    pub t: u64,
    /// chemin d'exe (app-id décodé), ou vide si indispo
    pub exe: String,
    pub proto: u8,
    pub adresse: String,
    pub port: u16,
}

/// Nom du pipe (côté client : `\\.\pipe\waly-seal`).
pub const PIPE_NAME: &str = r"\\.\pipe\waly-seal";
/// Nom du service Windows.
pub const SERVICE_NAME: &str = "WalySeal";

/// Id réservé du PÉRIMÈTRE permanent (scellé par défaut, ADR 2026-07-21) : ne
/// peut PAS être descellé via le pipe. Sessions Waly = positives ; agents tiers
/// = négatives (`session_pour_exe`).
pub const PERIMETRE: i64 = 0;

/// Un chemin d'exe appartient-il au périmètre Waly ? (Pur, sans dépendance
/// Windows → partagé serveur/CLI.) Sceller un exe de Waly ou le moteur reste
/// user-level (add-only) ; sceller tout autre exe = agent TIERS, action admin
/// (ADR 2026-09-16).
pub fn exe_admissible(chemin: &str) -> bool {
    let base = chemin
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(chemin)
        .to_ascii_lowercase();
    base == "flm.exe" || (base.starts_with("waly") && base.ends_with(".exe"))
}

/// Id de session STABLE dérivé du chemin d'exe d'un agent tiers (chantier C).
/// Toujours **négatif** (≠ périmètre 0, ≠ sessions Waly positives) → `seal` /
/// `unseal` / `journal` par exe sont cohérents et idempotents. FNV-1a sur le
/// chemin en minuscules.
pub fn session_pour_exe(exe: &str) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in exe.to_ascii_lowercase().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    // Ramener dans la plage négative non nulle : -1 .. i64::MIN+1.
    let v = (h >> 1) as i64; // 0 .. i64::MAX
    -(v.max(1))
}

/// Heuristique : l'exe ressemble-t-il à un interpréteur/runtime PARTAGÉ (le
/// sceller couperait d'AUTRES logiciels que l'agent visé) ? Sert à AVERTIR
/// l'admin, jamais à interdire (l'admin décide).
pub fn ressemble_interpreteur(exe: &str) -> bool {
    let base = exe
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(exe)
        .to_ascii_lowercase();
    matches!(
        base.as_str(),
        "python.exe"
            | "pythonw.exe"
            | "python3.exe"
            | "node.exe"
            | "deno.exe"
            | "bun.exe"
            | "java.exe"
            | "javaw.exe"
            | "dotnet.exe"
            | "ruby.exe"
            | "perl.exe"
            | "powershell.exe"
            | "pwsh.exe"
            | "cmd.exe"
            | "wscript.exe"
            | "cscript.exe"
    )
}

/// Requête client -> service. `session` est toujours l'id de session Waly du
/// client (il ne parle que pour lui-même).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Requete {
    /// Sceller `session` en bloquant la sortie réseau de `exes` (loopback
    /// préservé). Ré-émettable (idempotent). Deux usages :
    /// - scellé du périmètre Waly (`exes` tous `exe_admissible`) : user-level ;
    /// - scellé d'un agent TIERS (au moins un exe non admissible) : **exige un
    ///   client élevé** (ADR 2026-09-16 ; garde côté serveur, cf. `traiter`).
    /// Le PÉRIMÈTRE par défaut (id 0) est scellé par le service, pas le client.
    Sceller { session: i64, exes: Vec<String> },
    /// Lever le sceau de `session`. **Refuse le périmètre** (id 0). Lever un
    /// sceau TIERS (session négative) exige aussi un client élevé.
    Desceller { session: i64 },
    /// L'app REJOINT le périmètre permanent en donnant son propre chemin d'exe
    /// (add-only, validé `waly*.exe`/`flm.exe`). ADR 2026-07-21 : scellé par
    /// défaut, l'app se déclare, elle ne « scelle » pas.
    Rejoindre { exe: String },
    /// Journal d'audit (net events bloqués) de `session`.
    Journal { session: i64 },
    /// État : sessions scellées + version du service.
    Etat,
    /// Sonde de vivacité.
    Ping,
}

/// Nombre maximal d'exécutables dans une requête, et dans le périmètre.
pub const EXES_MAX: usize = 64;

/// LA politique du pipe, PURE et testée : ce qu'un client a le droit de
/// demander, selon qu'il est élevé ou non. Appelée par le service AVANT de
/// toucher au moteur.
///
/// Vécu 2026-10-02 (revue) : `Sceller { session: 0, … }` n'était pas gardé.
/// Or sceller une session REMPLACE ses filtres : un programme sans privilège
/// pouvait re-sceller le périmètre sur un exécutable bidon « waly-x.exe » et
/// retirer ainsi les filtres des vrais processus — exactement ce que
/// `Desceller` refuse. Même trou pour une session tierce (négative) scellée
/// avec un exe admissible. Règles :
/// - le périmètre (0) ne se scelle NI ne se lève par le pipe, élevé ou non :
///   il n'évolue que par le service et par `Rejoindre` (ajout seulement) ;
/// - une session tierce (négative) ou un exe hors périmètre Waly exigent un
///   client élevé, pour sceller comme pour lever.
pub fn autoriser(req: &Requete, eleve: bool) -> Result<(), &'static str> {
    match req {
        Requete::Sceller { session, exes } => {
            if *session == PERIMETRE {
                return Err("le perimetre ne se scelle pas par le pipe : il s'etend par « rejoindre »");
            }
            if exes.is_empty() || exes.len() > EXES_MAX {
                return Err("liste d'executables vide ou trop longue");
            }
            let tiers = *session < PERIMETRE || exes.iter().any(|e| !exe_admissible(e));
            if tiers && !eleve {
                return Err("scelle d'un agent tiers refuse : elevation requise (relance en administrateur)");
            }
            Ok(())
        }
        Requete::Desceller { session } => {
            if *session == PERIMETRE {
                return Err("le perimetre ne se leve pas");
            }
            if *session < PERIMETRE && !eleve {
                return Err("levee d'un sceau tiers refusee : elevation requise (relance en administrateur)");
            }
            Ok(())
        }
        Requete::Rejoindre { exe } => {
            if exe_admissible(exe) { Ok(()) } else { Err("exe non admissible au perimetre") }
        }
        Requete::Journal { .. } | Requete::Etat | Requete::Ping => Ok(()),
    }
}

/// Réponse service -> client.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "ok")]
pub enum Reponse {
    #[serde(rename = "true")]
    Ok(Succes),
    #[serde(rename = "false")]
    Err { message: String },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Succes {
    /// réponse à Sceller : nombre de filtres posés
    Scelle { filtres: usize, exes_vus: usize },
    /// réponse à Desceller / Ping
    Fait { fait: bool },
    /// réponse à Journal
    Journal { drops: Vec<Sortie> },
    /// réponse à Etat
    Etat {
        sessions: Vec<i64>,
        version: String,
        /// le service a-t-il le privilège de sceller (SYSTEM) ?
        privilegie: bool,
    },
}

impl Reponse {
    pub fn err(m: impl Into<String>) -> Reponse {
        Reponse::Err { message: m.into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perimetre_waly_admissible_user_level() {
        for p in [
            r"C:\waly\bin\waly.exe",
            r"C:\Users\x\Programs\Waly\waly-voice.exe",
            r"D:\a\b\FLM.EXE",
            "waly-desktop.exe",
        ] {
            assert!(exe_admissible(p), "{p} devrait etre admissible");
        }
    }

    #[test]
    fn agent_tiers_non_admissible() {
        for p in [
            r"C:\Program Files\Hermes\hermes.exe",
            r"C:\Windows\System32\notepad.exe",
            "malware-waly.txt",     // ne finit pas par .exe
            "notwaly.exe",          // ne commence pas par waly
        ] {
            assert!(!exe_admissible(p), "{p} ne devrait PAS etre admissible");
        }
    }

    #[test]
    fn session_pour_exe_stable_negative_et_distincte() {
        let a = session_pour_exe(r"C:\Program Files\Hermes\hermes.exe");
        let b = session_pour_exe(r"c:\program files\hermes\HERMES.EXE"); // meme, casse
        let c = session_pour_exe(r"C:\Program Files\Autre\agent.exe");
        assert_eq!(a, b, "meme chemin (casse ignoree) -> meme session");
        assert_ne!(a, c, "chemins differents -> sessions differentes");
        assert!(a < 0 && c < 0, "session tiers toujours negative");
        assert_ne!(a, PERIMETRE, "jamais le perimetre");
    }

    fn sceller(session: i64, exes: &[&str]) -> Requete {
        Requete::Sceller { session, exes: exes.iter().map(|e| e.to_string()).collect() }
    }

    #[test]
    fn le_perimetre_ne_se_remplace_ni_ne_se_leve_par_le_pipe() {
        // Le trou : re-sceller la session 0 sur un exe bidon retirait les
        // filtres des vrais processus. Refusé, même pour un client élevé.
        for eleve in [false, true] {
            assert!(autoriser(&sceller(PERIMETRE, &[r"C:\x\waly-bidon.exe"]), eleve).is_err());
            assert!(autoriser(&sceller(PERIMETRE, &[r"C:\waly\bin\waly.exe"]), eleve).is_err());
            assert!(autoriser(&Requete::Desceller { session: PERIMETRE }, eleve).is_err());
        }
        // Le périmètre ne fait que GRANDIR, et seulement d'exes de Waly.
        assert!(autoriser(&Requete::Rejoindre { exe: r"C:\Users\x\Programs\Waly\waly-desktop.exe".into() }, false).is_ok());
        assert!(autoriser(&Requete::Rejoindre { exe: r"C:\Windows\System32\curl.exe".into() }, false).is_err());
        assert!(autoriser(&Requete::Rejoindre { exe: r"C:\Windows\System32\curl.exe".into() }, true).is_err());
    }

    #[test]
    fn un_sceau_tiers_ne_se_touche_pas_sans_elevation() {
        let hermes = r"C:\Program Files\Hermes\hermes.exe";
        let s = session_pour_exe(hermes);
        // Sceller ou lever un agent tiers : administrateur seulement.
        assert!(autoriser(&sceller(s, &[hermes]), false).is_err());
        assert!(autoriser(&sceller(s, &[hermes]), true).is_ok());
        assert!(autoriser(&Requete::Desceller { session: s }, false).is_err());
        assert!(autoriser(&Requete::Desceller { session: s }, true).is_ok());
        // Le contournement : viser la session du tiers avec un exe « de Waly »
        // remplacerait son sceau sans élévation. Refusé.
        assert!(autoriser(&sceller(s, &[r"C:\x\waly-bidon.exe"]), false).is_err());
        // Un exe tiers glissé parmi des exes de Waly, sur une session Waly.
        assert!(autoriser(&sceller(7, &[r"C:\waly\bin\waly.exe", hermes]), false).is_err());
    }

    #[test]
    fn sessions_waly_user_level_bornes_et_lectures() {
        assert!(autoriser(&sceller(7, &[r"C:\waly\bin\waly.exe", r"D:\x\flm.exe"]), false).is_ok());
        assert!(autoriser(&Requete::Desceller { session: 7 }, false).is_ok());
        assert!(autoriser(&sceller(7, &[]), false).is_err(), "une liste vide leverait le sceau en douce");
        let trop: Vec<String> = (0..=EXES_MAX).map(|i| format!("waly-{i}.exe")).collect();
        assert!(autoriser(&Requete::Sceller { session: 7, exes: trop }, true).is_err());
        for r in [Requete::Ping, Requete::Etat, Requete::Journal { session: 0 }, Requete::Journal { session: -5 }] {
            assert!(autoriser(&r, false).is_ok());
        }
    }

    #[test]
    fn admissibilite_ruses_de_chemin() {
        // Seul le NOM du fichier compte : un dossier « waly » ne rend rien admissible.
        for p in [
            r"C:\waly\notepad.exe",
            r"C:\waly-tools\python.exe",
            "C:/waly/bin/../../Windows/System32/curl.exe",
            "waly.exe.txt",
            "waly-voice.exe.bak",
            r"C:\x\evil.exe:waly.exe", // flux alternatif : le nom reste evil.exe:…
            "",
            "waly",
            ".exe",
        ] {
            assert!(!exe_admissible(p), "{p:?} ne doit PAS etre admissible");
        }
        // Séparateurs mêlés et casse : admissibles.
        for p in ["C:/Users/x/AppData/Local/Programs/Waly/WALY-DESKTOP.EXE", r"C:\a/b\flm.exe"] {
            assert!(exe_admissible(p), "{p:?} devrait etre admissible");
        }
    }

    #[test]
    fn sessions_tierces_jamais_nulles_ni_positives() {
        for e in ["", "a", r"C:\x\y.exe", "é", &"z".repeat(4000)] {
            let s = session_pour_exe(e);
            assert!(s < PERIMETRE, "{e:?} -> {s}");
        }
    }

    #[test]
    fn format_du_fil_fige() {
        // Le protocole est une ligne JSON : ces formes sont un contrat.
        let r: Requete = serde_json::from_str(r#"{"cmd":"sceller","session":7,"exes":["waly.exe"]}"#).unwrap();
        assert!(matches!(r, Requete::Sceller { session: 7, .. }));
        assert!(matches!(serde_json::from_str::<Requete>(r#"{"cmd":"rejoindre","exe":"waly.exe"}"#).unwrap(), Requete::Rejoindre { .. }));
        assert!(matches!(serde_json::from_str::<Requete>(r#"{"cmd":"ping"}"#).unwrap(), Requete::Ping));
        // Jamais de filtre brut : une commande inconnue ou un champ manquant est refusé.
        for mauvais in [r#"{"cmd":"filtre","regle":"permit any"}"#, r#"{"cmd":"sceller"}"#, r#"{"session":0}"#, "pas du json", ""] {
            assert!(serde_json::from_str::<Requete>(mauvais).is_err(), "{mauvais}");
        }
        assert_eq!(serde_json::to_string(&Reponse::err("x")).unwrap(), r#"{"ok":"false","message":"x"}"#);
        let ok = serde_json::to_string(&Reponse::Ok(Succes::Fait { fait: true })).unwrap();
        assert_eq!(ok, r#"{"ok":"true","fait":true}"#);
        let e: Reponse = serde_json::from_str(r#"{"ok":"true","sessions":[0],"version":"v","privilegie":true}"#).unwrap();
        assert!(matches!(e, Reponse::Ok(Succes::Etat { privilegie: true, .. })));
    }

    #[test]
    fn interpreteurs_partages_detectes() {
        assert!(ressemble_interpreteur(r"C:\Python\python.exe"));
        assert!(ressemble_interpreteur("node.exe"));
        assert!(!ressemble_interpreteur(r"C:\Hermes\hermes.exe"));
    }
}
