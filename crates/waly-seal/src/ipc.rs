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

    #[test]
    fn interpreteurs_partages_detectes() {
        assert!(ressemble_interpreteur(r"C:\Python\python.exe"));
        assert!(ressemble_interpreteur("node.exe"));
        assert!(!ressemble_interpreteur(r"C:\Hermes\hermes.exe"));
    }
}
