//! Le tri de ce que le suivi d'événements de Windows rapporte (Garde, étape 3).
//!
//! Tout ce module est PUR et tourne partout : il transforme le brut du noyau
//! (chemins de périphérique, adresses en entier, ports en ordre réseau, des
//! centaines d'ouvertures de bibliothèques au démarrage d'un programme) en
//! observations lisibles et peu nombreuses. L'écoute elle-même est dans
//! `regard.rs` (Windows). Mesures de départ : `lab/garde-banc/README.md`.

use crate::ipc::Observation;

/// `\Device\HarddiskVolume3\x\y` -> `C:\x\y`, d'après la table des volumes
/// (périphérique, lettre). Inconnu : rendu tel quel.
pub fn chemin_dos(nt: &str, volumes: &[(String, String)]) -> String {
    let bas = nt.to_ascii_lowercase();
    for (peripherique, lettre) in volumes {
        let p = peripherique.to_ascii_lowercase();
        if bas.starts_with(&p) && bas[p.len()..].starts_with('\\') {
            return format!("{lettre}{}", &nt[p.len()..]);
        }
    }
    nt.to_string()
}

/// Ce chemin est-il du bruit ? Un programme qui démarre ouvre des centaines
/// de fichiers du système : les montrer noierait ce qui compte. On écarte le
/// système, les bibliothèques, les caches et les dossiers eux-mêmes. Ce qui
/// reste ressemble à « les fichiers de l'utilisateur ».
pub fn est_bruit(chemin: &str) -> bool {
    let c = chemin.to_ascii_lowercase();
    if c.is_empty() || c.ends_with('\\') || !c.contains('\\') {
        return true; // un dossier, un volume, un nom nu
    }
    // Flux secondaires (« :Zone.Identifier ») et fichiers de service.
    let nom = c.rsplit('\\').next().unwrap_or("");
    if nom.contains(':') || matches!(nom, "desktop.ini" | "thumbs.db" | "ntuser.dat" | "pagefile.sys") {
        return true;
    }
    const DOSSIERS: &[&str] = &[
        "\\windows\\", "\\program files\\", "\\program files (x86)\\", "\\programdata\\microsoft\\",
        "\\programdata\\packages\\", "\\$recycle.bin\\", "\\system volume information\\",
        "\\appdata\\local\\microsoft\\", "\\appdata\\local\\packages\\", "\\appdata\\local\\temp\\",
        "\\appdata\\locallow\\", "\\appdata\\roaming\\microsoft\\", "\\appdata\\local\\programs\\",
        "\\node_modules\\", "\\site-packages\\", "\\__pycache__\\", "\\.git\\", "\\.cache\\",
        "\\device\\", "\\$extend\\",
    ];
    if DOSSIERS.iter().any(|d| c.contains(d)) {
        return true;
    }
    const EXTENSIONS: &[&str] = &[
        ".dll", ".exe", ".sys", ".mui", ".nls", ".pyd", ".pyc", ".node", ".so", ".manifest", ".cat",
        ".pf", ".etl", ".tmp", ".lock", ".db-wal", ".db-shm", ".log",
    ];
    EXTENSIONS.iter().any(|e| nom.ends_with(e))
}

/// Une ouverture mérite-t-elle une ligne ? Le noyau signale aussi les
/// DOSSIERS qu'un programme parcourt, et rien dans le nom ne les distingue.
/// On s'appuie sur les options d'ouverture : « c'est un dossier » (bit 0x1)
/// écarte ; « ce n'est pas un dossier » (bit 0x40) garde ; sans indication, on
/// ne garde que ce qui porte une extension (mesuré au banc du 2026-10-06 :
/// un PowerShell qui démarre « ouvre » Documents, Musique, Program Files…).
pub fn ouverture_de_fichier(chemin: &str, options: u32) -> bool {
    const DOSSIER: u32 = 0x1;
    const PAS_DOSSIER: u32 = 0x40;
    if options & DOSSIER != 0 {
        return false;
    }
    if options & PAS_DOSSIER != 0 {
        return true;
    }
    a_une_extension(chemin)
}

/// Le dernier élément du chemin porte-t-il une extension ?
pub fn a_une_extension(chemin: &str) -> bool {
    let nom = chemin.rsplit('\\').next().unwrap_or("");
    matches!(nom.rfind('.'), Some(i) if i > 0 && i + 1 < nom.len())
}

/// Chaque programme en console lance la fenêtre de console de Windows : ce
/// n'est pas un geste de l'agent.
pub fn lancement_sans_interet(programme: &str) -> bool {
    programme.to_ascii_lowercase().ends_with("\\conhost.exe")
}

/// Un fichier du dossier même du programme (ses propres ressources) est du
/// bruit aussi.
pub fn chez_lui(chemin: &str, exe: &str) -> bool {
    let dossier = match exe.rfind(['\\', '/']) {
        Some(i) => exe[..=i].to_ascii_lowercase(),
        None => return false,
    };
    chemin.to_ascii_lowercase().starts_with(&dossier)
}

/// Adresse IPv4 telle que le noyau la donne : un entier dont les octets, en
/// mémoire, sont ceux de l'adresse.
pub fn adresse_v4(brut: u32) -> String {
    let o = brut.to_le_bytes();
    format!("{}.{}.{}.{}", o[0], o[1], o[2], o[3])
}

/// Le port arrive en ordre réseau (443 se lit 47873).
pub fn port(brut: u16) -> u16 {
    brut.swap_bytes()
}

/// Une connexion vers la machine elle-même ne sort pas : on ne la montre pas.
pub fn est_local(adresse: &str) -> bool {
    adresse.starts_with("127.") || adresse == "0.0.0.0" || adresse == "::1" || adresse == "::"
}

/// Le carnet : les observations récentes, regroupées. Le même programme qui
/// touche cent fois le même fichier fait UNE ligne, comptée, remontée en tête.
pub struct Carnet {
    lignes: Vec<Observation>,
    suivant: u64,
    max: usize,
}

impl Carnet {
    pub fn nouveau(max: usize) -> Carnet {
        Carnet { lignes: Vec::new(), suivant: 1, max: max.max(1) }
    }

    pub fn noter(&mut self, t: u64, exe: &str, pid: u32, genre: &str, objet: &str) {
        let n = self.suivant;
        self.suivant += 1;
        if let Some(i) = self
            .lignes
            .iter()
            .rposition(|o| o.genre == genre && o.objet.eq_ignore_ascii_case(objet) && o.exe.eq_ignore_ascii_case(exe))
        {
            let mut o = self.lignes.remove(i);
            o.fois = o.fois.saturating_add(1);
            o.t = t;
            o.n = n;
            o.pid = pid;
            self.lignes.push(o);
            return;
        }
        if self.lignes.len() >= self.max {
            self.lignes.remove(0);
        }
        self.lignes.push(Observation { n, t, exe: exe.to_string(), pid, genre: genre.to_string(), objet: objet.to_string(), fois: 1 });
    }

    /// Les observations plus récentes que `apres` (au plus `max`), anciennes d'abord.
    pub fn depuis(&self, apres: u64, max: usize) -> Vec<Observation> {
        let v: Vec<Observation> = self.lignes.iter().filter(|o| o.n > apres).cloned().collect();
        let debut = v.len().saturating_sub(max);
        v[debut..].to_vec()
    }

    pub fn dernier(&self) -> u64 {
        self.suivant - 1
    }

    pub fn vider(&mut self) {
        self.lignes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volumes() -> Vec<(String, String)> {
        vec![(r"\Device\HarddiskVolume3".into(), "C:".into()), (r"\Device\HarddiskVolume30".into(), "E:".into())]
    }

    #[test]
    fn le_chemin_du_noyau_devient_une_lettre_de_lecteur() {
        assert_eq!(chemin_dos(r"\Device\HarddiskVolume3\waly\README.md", &volumes()), r"C:\waly\README.md");
        // Le volume 30 n'est pas le volume 3 suivi d'un zéro.
        assert_eq!(chemin_dos(r"\Device\HarddiskVolume30\a.txt", &volumes()), r"E:\a.txt");
        assert_eq!(chemin_dos(r"\device\harddiskvolume3\A.txt", &volumes()), r"C:\A.txt");
        assert_eq!(chemin_dos(r"\Device\Mup\serveur\partage\a.txt", &volumes()), r"\Device\Mup\serveur\partage\a.txt");
    }

    #[test]
    fn le_bruit_du_systeme_est_ecarte_les_fichiers_de_l_utilisateur_restent() {
        for bruit in [
            r"C:\Windows\System32\kernel32.dll",
            r"C:\Program Files\nodejs\node.exe",
            r"C:\Users\x\AppData\Local\Temp\a.txt",
            r"C:\Users\x\AppData\Local\Microsoft\Windows\cache.dat",
            r"C:\Users\x\Documents\",
            r"C:\Users\x\Videos\desktop.ini",
            r"C:\Users\x\Downloads\a.pdf:Zone.Identifier",
            r"C:\projets\app\node_modules\x\index.js",
            r"C:\Python314\Lib\site-packages\x.py",
            r"C:\Users\x\AppData\Local\Programs\Ollama\lib\a.bin",
            "",
        ] {
            assert!(est_bruit(bruit), "devrait être écarté : {bruit}");
        }
        for garde in [
            r"C:\Users\x\Documents\budget-2026.xlsx",
            r"C:\waly\README.md",
            r"C:\Users\x\.ollama\models\blobs\sha256-abc",
            r"D:\projets\notes\todo.md",
        ] {
            assert!(!est_bruit(garde), "devrait rester : {garde}");
        }
    }

    #[test]
    fn un_dossier_parcouru_n_est_pas_un_fichier_ouvert() {
        // Le noyau le dit : dossier.
        assert!(!ouverture_de_fichier(r"C:\Users\x\Documents", 0x1));
        assert!(!ouverture_de_fichier(r"C:\Users\x\rapport.final", 0x21));
        // Le noyau le dit : fichier, même sans extension.
        assert!(ouverture_de_fichier(r"C:\Users\x\.ollama\models\blobs\sha256-abc", 0x40));
        // Il ne dit rien : on garde ce qui a une extension.
        assert!(ouverture_de_fichier(r"C:\waly\README.md", 0));
        assert!(!ouverture_de_fichier(r"C:\Users\x\Documents", 0));
        assert!(!ouverture_de_fichier(r"C:\Users\x\.config", 0), "un nom qui commence par un point n'est pas une extension");
        assert!(!ouverture_de_fichier(r"C:\Users\x\dossier.", 0));
        assert!(lancement_sans_interet(r"C:\Windows\System32\conhost.exe"));
        assert!(!lancement_sans_interet(r"C:\Windows\System32\PING.EXE"));
    }

    #[test]
    fn les_ressources_du_programme_lui_meme_sont_du_bruit() {
        assert!(chez_lui(r"C:\Outils\Agent\data\modele.bin", r"C:\Outils\Agent\agent.exe"));
        assert!(!chez_lui(r"C:\Users\x\Documents\a.txt", r"C:\Outils\Agent\agent.exe"));
    }

    #[test]
    fn adresse_et_port_tels_que_mesures_au_banc() {
        // Banc du 2026-10-06 : 1.1.1.1:443 arrivait « 16843009:47873 ».
        assert_eq!(adresse_v4(16843009), "1.1.1.1");
        assert_eq!(port(47873), 443);
        assert_eq!(adresse_v4(u32::from_le_bytes([127, 0, 0, 1])), "127.0.0.1");
        assert_eq!(adresse_v4(u32::from_le_bytes([104, 18, 2, 1])), "104.18.2.1");
        assert!(est_local("127.0.0.1") && est_local("::1") && !est_local("1.1.1.1"));
    }

    #[test]
    fn le_carnet_regroupe_compte_et_borne() {
        let mut c = Carnet::nouveau(3);
        c.noter(10, r"C:\a.exe", 1, "ouvert", r"C:\d\x.txt");
        c.noter(11, r"C:\a.exe", 1, "ouvert", r"C:\d\y.txt");
        c.noter(12, r"C:\a.exe", 1, "ouvert", r"C:\D\X.txt"); // le même, autre casse
        let v = c.depuis(0, 10);
        assert_eq!(v.len(), 2);
        // Le fichier retouché est remonté en dernier, compté deux fois.
        assert_eq!((v[1].objet.as_str(), v[1].fois, v[1].t), (r"C:\d\x.txt", 2, 12));
        // Un autre programme sur le même fichier fait une autre ligne.
        c.noter(13, r"C:\b.exe", 2, "ouvert", r"C:\d\x.txt");
        // Un autre geste aussi ; le carnet est borné à 3, le plus ancien sort.
        c.noter(14, r"C:\a.exe", 1, "ecrit", r"C:\d\x.txt");
        let v = c.depuis(0, 10);
        assert_eq!(v.len(), 3);
        assert!(v.iter().all(|o| o.objet != r"C:\d\y.txt"));
        // « depuis » ne rend que le neuf, et s'arrête au maximum demandé.
        let vu = c.dernier();
        c.noter(15, r"C:\a.exe", 1, "connecte", "1.1.1.1:443");
        let neuf = c.depuis(vu, 10);
        assert_eq!(neuf.len(), 1);
        assert_eq!(neuf[0].genre, "connecte");
        assert_eq!(c.depuis(0, 2).len(), 2);
    }
}
