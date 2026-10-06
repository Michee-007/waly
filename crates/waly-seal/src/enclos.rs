//! L'enclos (Garde, étape 4) : le **compte Windows à part** sous lequel un
//! autre agent est lancé pour qu'on puisse lui couper un dossier.
//!
//! Ce module ne fait que ce qui exige un administrateur : créer le compte, lui
//! donner un mot de passe, le supprimer. Tout le reste (lancer un programme
//! sous ce compte, régler les droits d'un dossier, sonder) se fait sans
//! élévation, dans l'app (`waly_core::enclos`) — mesuré au banc
//! (`lab/garde-banc`, essai 3).
//!
//! Le mot de passe n'est JAMAIS un argument (il se lirait dans la liste des
//! processus) : l'app l'écrit dans un fichier de son profil, cette commande
//! le lit puis l'efface.

/// Nom du compte. Un seul enclos par machine, commun aux agents qu'on y met.
pub const COMPTE: &str = "WalyEnclos";

/// Longueur minimale acceptée pour le mot de passe remis par l'app.
pub const MDP_MIN: usize = 24;

/// Lit le mot de passe remis par l'app et efface le fichier, quoi qu'il
/// arrive ensuite. PUR côté système de fichiers : testé partout.
pub fn lire_secret(fichier: &str) -> Result<String, String> {
    let lu = std::fs::read_to_string(fichier);
    let _ = std::fs::remove_file(fichier);
    let mdp = lu.map_err(|e| format!("fichier du mot de passe illisible : {e}"))?.trim().to_string();
    if mdp.chars().count() < MDP_MIN {
        return Err("mot de passe trop court".into());
    }
    Ok(mdp)
}

#[cfg(windows)]
mod win {
    use super::COMPTE;
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::LookupAccountNameW;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_SET_VALUE,
        REG_DWORD, REG_OPTION_NON_VOLATILE,
    };

    #[repr(C)]
    struct UserInfo1 {
        nom: *mut u16,
        mdp: *mut u16,
        age: u32,
        privilege: u32,
        dossier: *mut u16,
        commentaire: *mut u16,
        drapeaux: u32,
        script: *mut u16,
    }
    #[repr(C)]
    struct UserInfo1003 {
        mdp: *mut u16,
    }
    #[link(name = "netapi32")]
    extern "system" {
        fn NetUserAdd(serveur: *const u16, niveau: u32, tampon: *const u8, erreur: *mut u32) -> u32;
        fn NetUserSetInfo(serveur: *const u16, nom: *const u16, niveau: u32, tampon: *const u8, erreur: *mut u32) -> u32;
        fn NetUserDel(serveur: *const u16, nom: *const u16) -> u32;
    }
    #[link(name = "userenv")]
    extern "system" {
        fn DeleteProfileW(sid: *const u16, chemin: *const u16, machine: *const u16) -> i32;
    }

    const USER_PRIV_USER: u32 = 1;
    const UF_SCRIPT: u32 = 0x0001;
    const UF_PASSWD_CANT_CHANGE: u32 = 0x0040;
    const UF_DONT_EXPIRE_PASSWD: u32 = 0x1_0000;
    const NERR_USER_NOT_FOUND: u32 = 2221;
    const NERR_USER_EXISTS: u32 = 2224;
    const ACCES_REFUSE: u32 = 5;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn dire(code: u32) -> String {
        match code {
            ACCES_REFUSE => "accès refusé : cette commande demande un administrateur".into(),
            2245 => "Windows refuse ce mot de passe (règles de la machine)".into(),
            c => format!("erreur Windows {c}"),
        }
    }

    /// Le SID du compte, en texte (« S-1-5-21-… »).
    fn sid_texte() -> Option<String> {
        unsafe {
            let nom = w(COMPTE);
            let mut sid = vec![0u8; 256];
            let mut n = sid.len() as u32;
            let mut domaine = vec![0u16; 256];
            let mut nd = domaine.len() as u32;
            let mut usage = 0i32;
            if LookupAccountNameW(std::ptr::null(), nom.as_ptr(), sid.as_mut_ptr() as *mut c_void, &mut n, domaine.as_mut_ptr(), &mut nd, &mut usage) == 0 {
                return None;
            }
            let mut texte: *mut u16 = std::ptr::null_mut();
            if ConvertSidToStringSidW(sid.as_mut_ptr() as *mut c_void, &mut texte) == 0 || texte.is_null() {
                return None;
            }
            let mut l = 0;
            while *texte.add(l) != 0 {
                l += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(texte, l));
            LocalFree(texte as *mut c_void);
            Some(s)
        }
    }

    /// Le compte n'apparaît pas sur l'écran d'ouverture de session.
    fn cacher(cacher: bool) {
        unsafe {
            let cle = w(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon\SpecialAccounts\UserList");
            let nom = w(COMPTE);
            let mut h: HKEY = std::ptr::null_mut();
            if RegCreateKeyExW(HKEY_LOCAL_MACHINE, cle.as_ptr(), 0, std::ptr::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, std::ptr::null(), &mut h, std::ptr::null_mut()) != ERROR_SUCCESS {
                return;
            }
            if cacher {
                let zero = 0u32.to_le_bytes();
                RegSetValueExW(h, nom.as_ptr(), 0, REG_DWORD, zero.as_ptr(), 4);
            } else {
                RegDeleteValueW(h, nom.as_ptr());
            }
            RegCloseKey(h);
        }
    }

    /// Crée le compte, ou lui donne un mot de passe neuf s'il existe déjà.
    /// Rend « cree » ou « regle ».
    pub fn creer(fichier_mdp: &str) -> Result<&'static str, String> {
        let mdp = super::lire_secret(fichier_mdp)?;
        let (mut nom, mut mdp_w) = (w(COMPTE), w(&mdp));
        let mut commentaire = w("Enclos de Waly : les agents qu'on y lance ne voient pas tes dossiers.");
        let info = UserInfo1 {
            nom: nom.as_mut_ptr(),
            mdp: mdp_w.as_mut_ptr(),
            age: 0,
            privilege: USER_PRIV_USER,
            dossier: std::ptr::null_mut(),
            commentaire: commentaire.as_mut_ptr(),
            drapeaux: UF_SCRIPT | UF_DONT_EXPIRE_PASSWD | UF_PASSWD_CANT_CHANGE,
            script: std::ptr::null_mut(),
        };
        let r = unsafe { NetUserAdd(std::ptr::null(), 1, &info as *const _ as *const u8, std::ptr::null_mut()) };
        let fait = match r {
            0 => "cree",
            NERR_USER_EXISTS => {
                let neuf = UserInfo1003 { mdp: mdp_w.as_mut_ptr() };
                let r = unsafe { NetUserSetInfo(std::ptr::null(), nom.as_ptr(), 1003, &neuf as *const _ as *const u8, std::ptr::null_mut()) };
                if r != 0 {
                    return Err(dire(r));
                }
                "regle"
            }
            c => return Err(dire(c)),
        };
        // Le mot de passe ne traîne pas en mémoire plus que nécessaire.
        mdp_w.iter_mut().for_each(|c| *c = 0);
        cacher(true);
        Ok(fait)
    }

    /// Supprime le compte et son profil. Sans effet s'il n'existe pas.
    pub fn supprimer() -> Result<(), String> {
        let sid = sid_texte();
        let nom = w(COMPTE);
        let r = unsafe { NetUserDel(std::ptr::null(), nom.as_ptr()) };
        if r != 0 && r != NERR_USER_NOT_FOUND {
            return Err(dire(r));
        }
        if let Some(s) = sid {
            let s = w(&s);
            // Échoue si un programme de l'enclos tourne encore : sans gravité,
            // le profil vide reste sur le disque.
            unsafe { DeleteProfileW(s.as_ptr(), std::ptr::null(), std::ptr::null()) };
        }
        cacher(false);
        Ok(())
    }

    pub fn etat() -> String {
        match sid_texte() {
            Some(s) => format!("{COMPTE} existe ({s})"),
            None => format!("{COMPTE} n'existe pas"),
        }
    }
}

#[cfg(windows)]
pub use win::{creer, etat, supprimer};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_fichier_du_mot_de_passe_est_efface_meme_s_il_est_refuse() {
        let d = std::env::temp_dir();
        let bon = d.join(format!("waly-enclos-bon-{}.txt", std::process::id()));
        std::fs::write(&bon, "  Aa1!aaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n").unwrap();
        assert_eq!(lire_secret(&bon.to_string_lossy()).unwrap(), "Aa1!aaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert!(!bon.exists(), "le fichier doit disparaître après lecture");

        let court = d.join(format!("waly-enclos-court-{}.txt", std::process::id()));
        std::fs::write(&court, "trop-court").unwrap();
        assert!(lire_secret(&court.to_string_lossy()).is_err());
        assert!(!court.exists(), "refusé, mais effacé quand même");

        assert!(lire_secret(&d.join("waly-enclos-absent.txt").to_string_lossy()).is_err());
    }
}
