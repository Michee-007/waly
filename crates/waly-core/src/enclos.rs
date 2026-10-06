//! L'enclos (Garde, étape 4) : couper un dossier à un AUTRE agent.
//!
//! Voir ce qu'un programme touche (`regard`) n'empêche rien. Pour empêcher, il
//! faut que Windows lui-même refuse : l'agent est relancé sous un **compte
//! Windows à part**, `WalyEnclos`. Ce compte n'a aucun droit sur le profil de
//! l'utilisateur (ses documents, son bureau, ses clés) : c'est la règle de
//! Windows entre deux comptes, pas une promesse de Waly. On lui **donne** un
//! dossier (lecture, ou lecture et écriture), on le lui **reprend**, on lui
//! **coupe** un dossier situé hors du profil.
//!
//! Mesuré au banc (`lab/garde-banc`, essai 3) : tout ce module tourne SANS
//! élévation. Seule la création du compte en demande une (`creer`).
//!
//! Trois choses à savoir, dites aussi dans l'interface :
//! - **hors du profil**, le compte lit et écrit comme n'importe quel compte
//!   de la machine (`C:\waly`, un second disque) tant qu'on ne coupe pas ;
//! - **l'enclos est commun** : ce qu'on donne, tous les agents qu'on y met le
//!   voient ;
//! - **le réseau n'est pas concerné** : un agent de l'enclos sort sur internet
//!   tant qu'on ne le lui coupe pas (scellé, par programme).
//!
//! Règle du dépôt (piège 14) : un état affiché se prouve par un essai. Chaque
//! réglage est suivi d'une **sonde** lancée sous le compte de l'enclos, qui
//! tente réellement de lire puis d'écrire ; l'interface montre son résultat.

use rusqlite::Connection;
use serde::Serialize;

/// Le compte Windows de l'enclos.
pub const COMPTE: &str = waly_seal::enclos::COMPTE;

/// Ce que l'enclos peut faire d'un dossier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Droit {
    /// Lire et exécuter, sans rien modifier.
    Lecture,
    /// Lire, créer, modifier, supprimer.
    Ecriture,
    /// Refus explicite, y compris pour les sous-dossiers.
    Coupe,
}

impl Droit {
    pub fn cle(self) -> &'static str {
        match self {
            Droit::Lecture => "lecture",
            Droit::Ecriture => "ecriture",
            Droit::Coupe => "coupe",
        }
    }

    pub fn depuis(cle: &str) -> Option<Droit> {
        [Droit::Lecture, Droit::Ecriture, Droit::Coupe].into_iter().find(|d| d.cle() == cle)
    }

    /// Ce que la sonde doit trouver : (lit, écrit).
    pub fn attendu(self) -> (bool, bool) {
        match self {
            Droit::Lecture => (true, false),
            Droit::Ecriture => (true, true),
            Droit::Coupe => (false, false),
        }
    }
}

/// Un dossier réglé pour l'enclos, avec le résultat de son dernier essai.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Dossier {
    pub chemin: String,
    pub droit: Droit,
    /// Pourquoi il est là, quand c'est Waly qui l'a réglé (« pour démarrer
    /// Ollama », « la mémoire de Waly »). Vide : choix de l'utilisateur.
    pub pourquoi: String,
    /// Dernier essai : l'enclos a-t-il pu lire, écrire ? `None` : pas essayé.
    pub lit: Option<bool>,
    pub ecrit: Option<bool>,
    pub essaye_at: Option<String>,
}

impl Dossier {
    /// L'essai confirme-t-il le réglage ? `None` : pas encore essayé.
    pub fn conforme(&self) -> Option<bool> {
        Some((self.lit?, self.ecrit?) == self.droit.attendu())
    }
}

/// Un agent mis dans l'enclos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentEnclos {
    pub nom: String,
    pub exe: String,
    /// Ligne de commande de lancement (programme compris).
    pub ligne: String,
    /// Ses processus en cours (vide : arrêté).
    pub pids: Vec<u32>,
}

// --- Règles pures (testées sur toute plateforme) ----------------------------

/// Chemin Windows normalisé : `\`, sans `\` final (sauf la racine d'un lecteur).
pub fn normaliser(chemin: &str) -> String {
    let mut c = chemin.trim().trim_matches('"').replace('/', "\\");
    while c.len() > 3 && c.ends_with('\\') {
        c.pop();
    }
    c
}

/// `enfant` est-il `parent` ou l'un de ses sous-dossiers ? (casse ignorée)
pub fn sous(parent: &str, enfant: &str) -> bool {
    let (p, e) = (normaliser(parent).to_lowercase(), normaliser(enfant).to_lowercase());
    !p.is_empty() && (e == p || e.starts_with(&format!("{}\\", p.trim_end_matches('\\'))))
}

fn absolu(chemin: &str) -> bool {
    let b = chemin.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\'
}

/// Peut-on régler ce dossier ? `None` : oui. Sinon, la raison, dite à
/// l'utilisateur. `profil` : le dossier de l'utilisateur ; `garde` : les
/// dossiers de Waly qu'on ne donne jamais (sa base : ta mémoire).
pub fn refus_de_regler(chemin: &str, droit: Droit, profil: &str, garde: &[String]) -> Option<String> {
    let c = normaliser(chemin);
    if !absolu(&c) || c.contains("..") {
        return Some("Donne un chemin complet, par exemple C:\\projets\\site.".into());
    }
    if c.len() <= 3 {
        return Some("Un lecteur entier ne se règle pas : choisis un dossier.".into());
    }
    let bas = c.to_lowercase();
    if bas[1..].starts_with(":\\windows") || bas[1..].starts_with(":\\program files") || bas[1..].starts_with(":\\programdata") {
        return Some("C'est un dossier du système : le régler casserait des programmes.".into());
    }
    if droit == Droit::Coupe {
        return None;
    }
    if sous(&c, profil) {
        return Some("Donne un dossier précis, pas tout ton profil : l'enclos verrait tes clés, tes navigateurs et tes documents.".into());
    }
    let appdata = format!("{}\\AppData", normaliser(profil));
    if [appdata.clone(), format!("{appdata}\\Local"), format!("{appdata}\\Roaming")].iter().any(|a| a.eq_ignore_ascii_case(&c)) {
        return Some("Trop large : ce dossier contient les données de tous tes programmes. Donne celui de l'agent.".into());
    }
    if garde.iter().any(|g| sous(g, &c) || sous(&c, g)) {
        return Some("C'est là que Waly garde ta mémoire : ce dossier ne se donne pas.".into());
    }
    None
}

/// La ligne de commande sans son premier mot (le programme).
pub fn sans_programme(ligne: &str) -> &str {
    let l = ligne.trim_start();
    let reste = if let Some(apres) = l.strip_prefix('"') {
        apres.split_once('"').map(|(_, r)| r).unwrap_or("")
    } else {
        l.split_once(char::is_whitespace).map(|(_, r)| r).unwrap_or("")
    };
    reste.trim_start()
}

/// La ligne à lancer dans l'enclos : le chemin COMPLET du programme (jamais
/// un nom nu, qui se résoudrait dans le PATH de l'autre compte), puis les
/// mêmes arguments.
pub fn ligne_de_lancement(exe: &str, ligne: &str) -> String {
    let reste = sans_programme(ligne);
    if reste.is_empty() {
        format!("\"{exe}\"")
    } else {
        format!("\"{exe}\" {reste}")
    }
}

/// Les mots d'une ligne de commande (guillemets doubles respectés).
pub fn mots(ligne: &str) -> Vec<String> {
    let (mut out, mut cur, mut dans) = (Vec::new(), String::new(), false);
    for ch in ligne.chars() {
        match ch {
            '"' => dans = !dans,
            c if c.is_whitespace() && !dans => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Les dossiers du PROFIL qu'il faut donner en lecture pour que l'agent
/// démarre seulement : celui de son programme, et ceux que sa ligne de
/// commande nomme (un script, un paquet Node). Hors du profil, rien à donner.
pub fn besoins(exe: &str, ligne: &str, profil: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let mut ajouter = |d: String| {
        if sous(profil, &d) && !normaliser(&d).eq_ignore_ascii_case(&normaliser(profil)) && !v.iter().any(|x| x.eq_ignore_ascii_case(&d)) {
            v.push(d);
        }
    };
    let parent = |c: &str| normaliser(c).rsplit_once('\\').map(|(p, _)| p.to_string()).unwrap_or_default();
    ajouter(parent(exe));
    for m in mots(sans_programme(ligne)) {
        // `--option=C:\chemin` : on garde ce qui suit le signe égal.
        let m = normaliser(m.rsplit_once('=').map(|(_, r)| r).unwrap_or(&m));
        if !absolu(&m) {
            continue;
        }
        let bas = m.to_lowercase();
        if let Some(i) = bas.find("\\node_modules\\") {
            ajouter(m[..i + "\\node_modules".len()].to_string());
        } else if m.rsplit('\\').next().is_some_and(|f| f.contains('.')) {
            ajouter(parent(&m));
        } else {
            ajouter(m);
        }
    }
    // Un dossier déjà couvert par un autre de la liste est inutile.
    let tous = v.clone();
    v.retain(|d| !tous.iter().any(|a| !a.eq_ignore_ascii_case(d) && sous(a, d)));
    v
}

/// Nom de l'objet « job » Windows qui regroupe les processus d'un agent de
/// l'enclos (lui et tout ce qu'il lance). Stable : retrouvé après un
/// redémarrage de Waly.
pub fn nom_job(exe: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in normaliser(exe).to_lowercase().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    format!("Local\\WalyEnclos-{h:016x}")
}

/// Mot de passe du compte, tiré de 32 octets d'aléa : 36 caractères, avec
/// majuscule, minuscule, chiffre et signe (règles de Windows).
pub fn mot_de_passe(alea: &[u8; 32]) -> String {
    const ALPHABET: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut s = String::from("Wy!7");
    s.extend(alea.iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char));
    s
}

/// Les deux sondes, lancées sous le compte de l'enclos. Leur code de sortie
/// est la réponse : 0 = l'enclos a pu.
pub fn ligne_sonde(dossier: &str, ecrire: bool) -> String {
    let d = normaliser(dossier);
    if ecrire {
        let f = format!("{d}\\~waly-essai.tmp");
        format!("cmd.exe /c copy /y nul \"{f}\" >nul 2>&1 && del /q \"{f}\" >nul 2>&1")
    } else {
        format!("cmd.exe /c dir /a \"{d}\" >nul 2>&1")
    }
}

// --- Base -------------------------------------------------------------------

pub fn migrer(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS enclos_dossiers (
           chemin    TEXT PRIMARY KEY COLLATE NOCASE,
           droit     TEXT NOT NULL,
           pourquoi  TEXT NOT NULL DEFAULT '',
           lit       INTEGER,
           ecrit     INTEGER,
           essaye_at TEXT
         );
         CREATE TABLE IF NOT EXISTS enclos_agents (
           exe   TEXT PRIMARY KEY COLLATE NOCASE,
           nom   TEXT NOT NULL,
           ligne TEXT NOT NULL DEFAULT '',
           pid   INTEGER NOT NULL DEFAULT 0,
           at    TEXT NOT NULL DEFAULT (datetime('now','localtime'))
         );",
    )
}

/// Les dossiers réglés, par chemin.
pub fn dossiers(conn: &Connection) -> Vec<Dossier> {
    let Ok(mut st) = conn.prepare("SELECT chemin, droit, pourquoi, lit, ecrit, essaye_at FROM enclos_dossiers ORDER BY chemin COLLATE NOCASE") else {
        return Vec::new();
    };
    let lignes = st.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<i64>>(3)?, r.get::<_, Option<i64>>(4)?, r.get::<_, Option<String>>(5)?))
    });
    let Ok(lignes) = lignes else { return Vec::new() };
    lignes
        .flatten()
        .filter_map(|(chemin, droit, pourquoi, lit, ecrit, essaye_at)| {
            Some(Dossier { chemin, droit: Droit::depuis(&droit)?, pourquoi, lit: lit.map(|x| x != 0), ecrit: ecrit.map(|x| x != 0), essaye_at })
        })
        .collect()
}

fn inscrire(conn: &Connection, chemin: &str, droit: Option<Droit>, pourquoi: &str) -> rusqlite::Result<()> {
    match droit {
        Some(d) => conn.execute(
            "INSERT INTO enclos_dossiers (chemin, droit, pourquoi) VALUES (?1, ?2, ?3)
             ON CONFLICT(chemin) DO UPDATE SET droit = ?2, pourquoi = ?3, lit = NULL, ecrit = NULL, essaye_at = NULL",
            rusqlite::params![chemin, d.cle(), pourquoi],
        ),
        None => conn.execute("DELETE FROM enclos_dossiers WHERE chemin = ?1", [chemin]),
    }
    .map(|_| ())
}

fn inscrire_essai(conn: &Connection, chemin: &str, lit: bool, ecrit: bool) {
    let _ = conn.execute(
        "UPDATE enclos_dossiers SET lit = ?2, ecrit = ?3, essaye_at = datetime('now','localtime') WHERE chemin = ?1",
        rusqlite::params![chemin, lit as i64, ecrit as i64],
    );
}

fn agents_inscrits(conn: &Connection) -> Vec<(String, String, String, u32)> {
    let Ok(mut st) = conn.prepare("SELECT nom, exe, ligne, pid FROM enclos_agents ORDER BY nom") else { return Vec::new() };
    let l = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? as u32)));
    l.map(|x| x.flatten().collect()).unwrap_or_default()
}

/// Les agents mis dans l'enclos, avec leurs processus en cours.
pub fn agents(conn: &Connection) -> Vec<AgentEnclos> {
    agents_inscrits(conn)
        .into_iter()
        .map(|(nom, exe, ligne, _)| {
            let pids = pids(&exe);
            AgentEnclos { nom, exe, ligne, pids }
        })
        .collect()
}

/// Le dossier de l'utilisateur (`C:\Users\Nom`).
pub fn profil() -> String {
    std::env::var("USERPROFILE").unwrap_or_default()
}

/// Les dossiers de Waly qu'on ne donne jamais à l'enclos.
pub fn dossiers_gardes() -> Vec<String> {
    vec![crate::chemins::data().to_string_lossy().into_owned()]
}

fn noter(conn: &Connection, detail: &str) {
    let _ = conn.execute("INSERT INTO registre (agent, ressource, detail) VALUES ('Toi', 'fichiers', ?1)", [detail]);
}

/// Règle un dossier pour l'enclos (`None` : retire le réglage), puis fait
/// l'essai. Rend le dossier avec le résultat de la sonde, ou `None` s'il a
/// été retiré.
pub fn regler(conn: &Connection, chemin: &str, droit: Option<Droit>, pourquoi: &str) -> Result<Option<Dossier>, String> {
    let c = normaliser(chemin);
    if let Some(d) = droit {
        if let Some(raison) = refus_de_regler(&c, d, &profil(), &dossiers_gardes()) {
            return Err(raison);
        }
    }
    if droit.is_none() && dossiers_gardes().iter().any(|g| sous(g, &c) && !sous(&profil(), g)) {
        return Err("Ce dossier reste coupé : c'est là que Waly garde ta mémoire.".into());
    }
    if !std::path::Path::new(&c).is_dir() {
        // Un dossier disparu : on oublie son réglage, il n'y a plus rien à régler.
        if droit.is_none() {
            let _ = inscrire(conn, &c, None, "");
            return Ok(None);
        }
        return Err(format!("{c} n'est pas un dossier."));
    }
    systeme::appliquer(&c, droit)?;
    inscrire(conn, &c, droit, pourquoi).map_err(|e| e.to_string())?;
    if pourquoi.is_empty() {
        noter(
            conn,
            &match droit {
                Some(Droit::Lecture) => format!("as donné à l'enclos, en lecture : {c}"),
                Some(Droit::Ecriture) => format!("as donné à l'enclos, en écriture : {c}"),
                Some(Droit::Coupe) => format!("as coupé à l'enclos : {c}"),
                None => format!("as repris à l'enclos : {c}"),
            },
        );
    }
    if droit.is_none() {
        return Ok(None);
    }
    let _ = essayer(conn, &c);
    Ok(dossiers(conn).into_iter().find(|d| d.chemin.eq_ignore_ascii_case(&c)))
}

/// L'essai : une sonde lancée sous le compte de l'enclos tente de lire le
/// dossier, puis d'y écrire. Rend (lit, écrit) et l'inscrit.
pub fn essayer(conn: &Connection, chemin: &str) -> Result<(bool, bool), String> {
    let c = normaliser(chemin);
    let mdp = secret(conn)?;
    let lit = systeme::sonde(&mdp, &ligne_sonde(&c, false))? == 0;
    let ecrit = systeme::sonde(&mdp, &ligne_sonde(&c, true))? == 0;
    // Si la sonde a écrit sans pouvoir effacer, le fichier d'essai ne reste pas.
    let _ = std::fs::remove_file(format!("{c}\\~waly-essai.tmp"));
    inscrire_essai(conn, &c, lit, ecrit);
    Ok((lit, ecrit))
}

/// Refait tous les essais, et celui du profil : sans rien donner, l'enclos ne
/// doit PAS pouvoir lire le dossier de l'utilisateur. Rend ce dernier résultat
/// (`true` : le profil est bien invisible).
pub fn tout_essayer(conn: &Connection) -> Result<bool, String> {
    let mdp = secret(conn)?;
    for d in dossiers(conn) {
        if std::path::Path::new(&d.chemin).is_dir() {
            let _ = essayer(conn, &d.chemin);
        }
    }
    let invisible = systeme::sonde(&mdp, &ligne_sonde(&profil(), false))? != 0;
    let _ = crate::store::set_reglage(conn, "enclos_profil_invisible", if invisible { "1" } else { "0" });
    let _ = crate::store::set_reglage(conn, "enclos_profil_essaye", &maintenant(conn));
    Ok(invisible)
}

fn maintenant(conn: &Connection) -> String {
    conn.query_row("SELECT datetime('now','localtime')", [], |r| r.get(0)).unwrap_or_default()
}

/// Dernier essai du profil : (invisible ?, quand). `None` : jamais essayé.
pub fn essai_profil(conn: &Connection) -> Option<(bool, String)> {
    let v = crate::store::reglage(conn, "enclos_profil_invisible")?;
    Some((v == "1", crate::store::reglage(conn, "enclos_profil_essaye").unwrap_or_default()))
}

// --- Le compte et son secret ------------------------------------------------

/// Le compte de l'enclos existe-t-il sur cette machine ?
pub fn existe() -> bool {
    systeme::existe()
}

fn secret(conn: &Connection) -> Result<String, String> {
    let h = crate::store::reglage(conn, "enclos_secret").filter(|s| !s.is_empty()).ok_or("l'enclos n'est pas encore créé")?;
    let chiffre = crate::partage::dehex(&h).ok_or("secret de l'enclos illisible : recrée l'enclos")?;
    String::from_utf8(crate::exterieur::deproteger(&chiffre)?).map_err(|_| "secret de l'enclos illisible : recrée l'enclos".to_string())
}

/// L'enclos est-il utilisable : le compte existe ET Waly en a le secret ?
pub fn pret(conn: &Connection) -> bool {
    existe() && secret(conn).is_ok()
}

/// Crée le compte (Windows demande l'accord), ou lui redonne un mot de passe
/// si Waly a perdu le sien. Le mot de passe est tiré ici, remis au programme
/// élevé par un fichier du profil qu'il efface, puis gardé chiffré par Windows
/// (DPAPI) dans la base : seul ce compte utilisateur peut le relire.
pub fn creer(conn: &Connection) -> Result<(), String> {
    let svc = crate::sceau::chemin_service().ok_or("service de Waly introuvable : réinstalle Waly")?;
    creer_avec(conn, &svc)
}

/// Comme [`creer`], avec le programme élevé nommé par l'appelant (essais au
/// banc avec un service bâti mais pas installé).
pub fn creer_avec(conn: &Connection, service: &str) -> Result<(), String> {
    use crypto_box::aead::rand_core::RngCore;
    let mut alea = [0u8; 32];
    crypto_box::aead::OsRng.fill_bytes(&mut alea);
    let mdp = mot_de_passe(&alea);
    let fichier = std::env::temp_dir().join(format!("waly-enclos-{}.tmp", crate::partage::hex(&alea[..6])));
    std::fs::write(&fichier, &mdp).map_err(|e| format!("écriture du secret : {e}"))?;
    let r = crate::sceau::lancer_eleve_sur(service, &format!("enclos creer \"{}\"", fichier.to_string_lossy()));
    let _ = std::fs::remove_file(&fichier);
    match r? {
        0 => {}
        2 => return Err("le service installé est trop ancien pour l'enclos : réinstalle Waly".into()),
        c => return Err(format!("le compte n'a pas été créé (code {c})")),
    }
    let chiffre = crate::exterieur::proteger(mdp.as_bytes())?;
    crate::store::set_reglage(conn, "enclos_secret", &crate::partage::hex(&chiffre)).map_err(|e| e.to_string())?;
    let _ = crate::sceau::noter(conn, "pose", &format!("enclos créé (compte Windows {COMPTE})"));
    // La base de Waly vit hors du profil (C:\waly\data) : sans coupure,
    // l'enclos pourrait lire ta mémoire. Coupée d'office, et essayée.
    for g in dossiers_gardes() {
        if !sous(&profil(), &g) && std::path::Path::new(&g).is_dir() {
            let _ = regler(conn, &g, Some(Droit::Coupe), "la mémoire de Waly");
        }
    }
    let _ = tout_essayer(conn);
    Ok(())
}

// --- Lancer, suivre, arrêter ------------------------------------------------

/// Les processus en cours d'un agent de l'enclos (lui et ce qu'il a lancé).
pub fn pids(exe: &str) -> Vec<u32> {
    systeme::pids(&nom_job(exe))
}

/// Donne à l'enclos, en lecture, les dossiers du profil sans lesquels cet
/// agent ne démarrerait pas (`besoins`). À faire AVANT de fermer l'instance
/// en cours : si un dossier se refuse, rien n'a encore été arrêté.
pub fn preparer(conn: &Connection, nom: &str, exe: &str, ligne: &str) -> Result<Vec<String>, String> {
    secret(conn)?;
    let regles = dossiers(conn);
    let mut donnes = Vec::new();
    for b in besoins(exe, ligne, &profil()) {
        if !regles.iter().any(|d| d.droit != Droit::Coupe && sous(&d.chemin, &b)) {
            regler(conn, &b, Some(Droit::Lecture), &format!("pour démarrer {nom}"))?;
            donnes.push(b);
        }
    }
    Ok(donnes)
}

/// Relance un agent déjà mis dans l'enclos, avec la même ligne de commande.
pub fn relancer(conn: &Connection, exe: &str) -> Result<u32, String> {
    let (nom, exe, ligne, _) = agents_inscrits(conn)
        .into_iter()
        .find(|(_, e, _, _)| e.eq_ignore_ascii_case(&normaliser(exe)))
        .ok_or("cet agent n'est pas dans l'enclos")?;
    lancer(conn, &nom, &exe, &ligne)
}

/// Lance un agent dans l'enclos (après [`preparer`]). Rend son PID.
pub fn lancer(conn: &Connection, nom: &str, exe: &str, ligne: &str) -> Result<u32, String> {
    let mdp = secret(conn)?;
    preparer(conn, nom, exe, ligne)?;
    // Dossier de travail : le premier dossier donné en écriture, sinon le
    // dossier public (celui de l'appelant, dans le profil, lui est fermé).
    let travail = dossiers(conn)
        .into_iter()
        .find(|d| d.droit == Droit::Ecriture && d.pourquoi.is_empty())
        .map(|d| d.chemin)
        .unwrap_or_else(|| std::env::var("PUBLIC").unwrap_or_else(|_| r"C:\Users\Public".into()));
    let ligne = ligne_de_lancement(exe, ligne);
    let pid = systeme::lancer(&mdp, &ligne, &travail, &nom_job(exe))?;
    conn.execute(
        "INSERT INTO enclos_agents (exe, nom, ligne, pid) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(exe) DO UPDATE SET nom = ?2, ligne = ?3, pid = ?4, at = datetime('now','localtime')",
        rusqlite::params![normaliser(exe), nom, ligne, pid],
    )
    .map_err(|e| e.to_string())?;
    let _ = crate::sceau::noter(conn, "pose", &format!("{nom} lancé dans l'enclos"));
    Ok(pid)
}

/// Arrête un agent de l'enclos (lui et ce qu'il a lancé). `oublier` : le
/// retire aussi de la liste. Rend le nombre de processus arrêtés.
pub fn arreter(conn: &Connection, exe: &str, oublier: bool) -> usize {
    let n = systeme::arreter(&nom_job(exe));
    if oublier {
        let _ = conn.execute("DELETE FROM enclos_agents WHERE exe = ?1", [normaliser(exe)]);
    }
    n
}

/// Sélecteur de dossier natif. `None` si l'utilisateur annule.
pub fn choisir_dossier(titre: &str) -> Option<String> {
    systeme::choisir_dossier(titre)
}

#[cfg(not(windows))]
mod systeme {
    use super::Droit;
    const NON: &str = "l'enclos : Windows uniquement";
    pub fn existe() -> bool {
        false
    }
    pub fn appliquer(_chemin: &str, _droit: Option<Droit>) -> Result<(), String> {
        Err(NON.into())
    }
    pub fn sonde(_mdp: &str, _ligne: &str) -> Result<u32, String> {
        Err(NON.into())
    }
    pub fn lancer(_mdp: &str, _ligne: &str, _travail: &str, _job: &str) -> Result<u32, String> {
        Err(NON.into())
    }
    pub fn pids(_job: &str) -> Vec<u32> {
        Vec::new()
    }
    pub fn arreter(_job: &str) -> usize {
        0
    }
    pub fn choisir_dossier(_titre: &str) -> Option<String> {
        None
    }
}

#[cfg(windows)]
mod systeme {
    use super::{Droit, COMPTE};
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{CloseHandle, DuplicateHandle, GetLastError, LocalFree, DUPLICATE_SAME_ACCESS, HANDLE, WAIT_OBJECT_0};
    use windows_sys::Win32::Security::Authorization::{
        GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, DENY_ACCESS, EXPLICIT_ACCESS_W, GRANT_ACCESS,
        NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{DeleteAce, EqualSid, GetAce, LookupAccountNameW, ACL, DACL_SECURITY_INFORMATION};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, OpenJobObjectW, QueryInformationJobObject, TerminateJobObject,
        JobObjectBasicProcessIdList,
    };
    use windows_sys::Win32::System::Threading::{
        CreateProcessWithLogonW, GetCurrentProcess, GetExitCodeProcess, ResumeThread, TerminateProcess, WaitForSingleObject,
        CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, LOGON_WITH_PROFILE, PROCESS_INFORMATION, STARTF_USESHOWWINDOW,
        STARTUPINFOW,
    };

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Le SID du compte de l'enclos (octets), s'il existe.
    fn sid() -> Option<Vec<u8>> {
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
            Some(sid)
        }
    }

    pub fn existe() -> bool {
        sid().is_some()
    }

    // Droits de fichier (winnt.h) : lecture et exécution ; modification ; tout.
    const LIRE: u32 = 0x0012_00A9;
    const MODIFIER: u32 = 0x0013_01BF;
    const TOUT: u32 = 0x001F_01FF;
    /// Écrire, ajouter, changer les attributs, supprimer (soi ou un enfant).
    const ECRIRE: u32 = 0x0001_0156;
    /// Vaut pour le dossier, ses sous-dossiers et ses fichiers.
    const HERITE: u32 = 0x3;

    /// Pose (ou retire) les droits du compte de l'enclos sur un dossier. Les
    /// entrées déjà posées pour ce compte sont d'abord retirées : un dossier
    /// n'a jamais deux réglages. Les droits des AUTRES comptes ne bougent pas.
    pub fn appliquer(chemin: &str, droit: Option<Droit>) -> Result<(), String> {
        let mut sid = sid().ok_or("le compte de l'enclos n'existe pas")?;
        let chemin_w = w(chemin);
        unsafe {
            let mut ancien: *mut ACL = std::ptr::null_mut();
            let mut descripteur: *mut c_void = std::ptr::null_mut();
            let e = GetNamedSecurityInfoW(chemin_w.as_ptr(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, std::ptr::null_mut(), std::ptr::null_mut(), &mut ancien, std::ptr::null_mut(), &mut descripteur);
            if e != 0 {
                return Err(format!("lecture des droits de {chemin} : erreur Windows {e}"));
            }
            let qui = TRUSTEE_W {
                pMultipleTrustee: std::ptr::null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_USER,
                ptstrName: sid.as_mut_ptr() as *mut u16,
            };
            // Vécu au banc : REVOKE_ACCESS retire les permissions mais LAISSE
            // les refus. « Rendre » un dossier coupé le laissait coupé. On
            // retire donc nous-mêmes toute entrée explicite de ce compte.
            retirer_entrees(ancien, sid.as_mut_ptr() as *mut c_void);
            let mut entrees: Vec<EXPLICIT_ACCESS_W> = Vec::new();
            if let Some(d) = droit {
                let (droits, mode) = match d {
                    Droit::Lecture => (LIRE, GRANT_ACCESS),
                    Droit::Ecriture => (MODIFIER, GRANT_ACCESS),
                    Droit::Coupe => (TOUT, DENY_ACCESS),
                };
                entrees.push(EXPLICIT_ACCESS_W { grfAccessPermissions: droits, grfAccessMode: mode, grfInheritance: HERITE, Trustee: qui });
                if d == Droit::Lecture {
                    // Hors du profil, tout compte de la machine écrit déjà :
                    // « lecture » doit donc REFUSER l'écriture, pas seulement
                    // ne pas la donner.
                    entrees.push(EXPLICIT_ACCESS_W { grfAccessPermissions: ECRIRE, grfAccessMode: DENY_ACCESS, grfInheritance: HERITE, Trustee: qui });
                }
            }
            let mut neuf: *mut ACL = std::ptr::null_mut();
            let e = SetEntriesInAclW(entrees.len() as u32, if entrees.is_empty() { std::ptr::null() } else { entrees.as_ptr() }, ancien, &mut neuf);
            if e != 0 {
                LocalFree(descripteur);
                return Err(format!("calcul des droits de {chemin} : erreur Windows {e}"));
            }
            let e = SetNamedSecurityInfoW(chemin_w.as_ptr(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, std::ptr::null_mut(), std::ptr::null_mut(), neuf, std::ptr::null_mut());
            LocalFree(neuf as *mut c_void);
            LocalFree(descripteur);
            match e {
                0 => Ok(()),
                5 => Err(format!("Windows refuse de changer les droits de {chemin} : ce dossier ne t'appartient pas.")),
                e => Err(format!("réglage des droits de {chemin} : erreur Windows {e}")),
            }
        }
    }

    /// Retire d'une liste de droits les entrées posées directement pour ce
    /// compte (permissions et refus). Les entrées héritées d'un dossier
    /// parent ne se retirent pas ici : elles se règlent sur le parent.
    unsafe fn retirer_entrees(liste: *mut ACL, sid: *mut c_void) {
        const HERITEE: u8 = 0x10;
        if liste.is_null() {
            return;
        }
        let n = (*liste).AceCount as u32;
        for i in (0..n).rev() {
            let mut entree: *mut c_void = std::ptr::null_mut();
            if GetAce(liste, i, &mut entree) == 0 || entree.is_null() {
                continue;
            }
            // En-tête : type (0 = permission, 1 = refus), drapeaux, taille ;
            // puis le masque (4 octets), puis le SID.
            let (genre, drapeaux) = (*(entree as *const u8), *(entree as *const u8).add(1));
            if genre > 1 || drapeaux & HERITEE != 0 {
                continue;
            }
            if EqualSid((entree as *mut u8).add(8) as *mut c_void, sid) != 0 {
                DeleteAce(liste, i);
            }
        }
    }

    fn dire(code: u32) -> String {
        match code {
            1326 => "le mot de passe de l'enclos n'est plus le bon : recrée l'enclos".into(),
            1385 | 1327 | 1328 | 1330 | 1331 => "Windows refuse d'ouvrir une session pour le compte de l'enclos (règle de la machine)".into(),
            1058 | 1062 => "le service « Connexion secondaire » de Windows est désactivé : il faut le réactiver pour l'enclos".into(),
            2 | 3 => "programme introuvable".into(),
            5 => "l'enclos n'a pas le droit de lire ce programme : donne-lui son dossier".into(),
            267 => "dossier de travail inaccessible à l'enclos".into(),
            c => format!("erreur Windows {c}"),
        }
    }

    /// Lance une ligne de commande sous le compte de l'enclos, suspendue.
    /// `cache` : sans fenêtre (les sondes).
    unsafe fn creer_processus(mdp: &str, ligne: &str, travail: &str, cache: bool) -> Result<PROCESS_INFORMATION, String> {
        let (compte, ici, mut mdp_w, mut ligne_w, travail_w) = (w(COMPTE), w("."), w(mdp), w(ligne), w(travail));
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        if cache {
            si.dwFlags = STARTF_USESHOWWINDOW;
            si.wShowWindow = 0; // SW_HIDE
        }
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessWithLogonW(
            compte.as_ptr(),
            ici.as_ptr(),
            mdp_w.as_ptr(),
            LOGON_WITH_PROFILE,
            std::ptr::null(),
            ligne_w.as_mut_ptr(),
            CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
            std::ptr::null(),
            travail_w.as_ptr(),
            &si,
            &mut pi,
        );
        let e = GetLastError();
        mdp_w.iter_mut().for_each(|c| *c = 0);
        if ok == 0 {
            return Err(dire(e));
        }
        Ok(pi)
    }

    fn systeme32() -> String {
        format!("{}\\System32", std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()))
    }

    /// Lance une sonde et rend son code de sortie.
    pub fn sonde(mdp: &str, ligne: &str) -> Result<u32, String> {
        // Chemin complet de cmd.exe : jamais un nom nu résolu par le PATH.
        let ligne = format!("{}\\{ligne}", systeme32());
        unsafe {
            let pi = creer_processus(mdp, &ligne, &systeme32(), true)?;
            ResumeThread(pi.hThread);
            let fini = WaitForSingleObject(pi.hProcess, 20_000) == WAIT_OBJECT_0;
            let mut code = 1u32;
            if fini {
                GetExitCodeProcess(pi.hProcess, &mut code);
            } else {
                TerminateProcess(pi.hProcess, 1);
            }
            CloseHandle(pi.hThread);
            CloseHandle(pi.hProcess);
            if fini {
                Ok(code)
            } else {
                Err("la sonde de l'enclos n'a pas répondu".into())
            }
        }
    }

    /// Lance l'agent, le place dans son « job » (lui et ses enfants : on sait
    /// lesquels tournent, on les arrête ensemble), puis le laisse partir.
    pub fn lancer(mdp: &str, ligne: &str, travail: &str, job: &str) -> Result<u32, String> {
        unsafe {
            let pi = creer_processus(mdp, ligne, travail, false)?;
            let nom = w(job);
            let j: HANDLE = CreateJobObjectW(std::ptr::null(), nom.as_ptr());
            let mut suivi = false;
            if !j.is_null() {
                suivi = AssignProcessToJobObject(j, pi.hProcess) != 0;
                // Vécu au banc : le nom d'un job disparaît quand plus personne
                // n'en tient de poignée, même si des processus y tournent. On
                // en dépose une DANS l'agent : le job reste retrouvable par
                // son nom tant que l'agent vit, même après la fermeture de Waly.
                let mut chez_lui: HANDLE = std::ptr::null_mut();
                suivi = suivi && DuplicateHandle(GetCurrentProcess(), j, pi.hProcess, &mut chez_lui, 0, 0, DUPLICATE_SAME_ACCESS) != 0;
                CloseHandle(j);
            }
            if !suivi {
                // Sans job, Waly ne saurait ni le suivre ni l'arrêter : on ne
                // laisse pas partir un agent qu'on ne tient pas.
                TerminateProcess(pi.hProcess, 1);
                CloseHandle(pi.hThread);
                CloseHandle(pi.hProcess);
                return Err("Windows a refusé de regrouper les processus de l'agent : il n'a pas été lancé".into());
            }
            ResumeThread(pi.hThread);
            CloseHandle(pi.hThread);
            CloseHandle(pi.hProcess);
            Ok(pi.dwProcessId)
        }
    }

    const JOB_OBJECT_TERMINATE: u32 = 0x0008;
    const JOB_OBJECT_QUERY: u32 = 0x0004;

    pub fn pids(job: &str) -> Vec<u32> {
        unsafe {
            let nom = w(job);
            let j = OpenJobObjectW(JOB_OBJECT_QUERY, 0, nom.as_ptr());
            if j.is_null() {
                return Vec::new();
            }
            // En-tête (deux u32) puis les identifiants, de la taille d'un pointeur.
            let mut tampon = vec![0usize; 1 + 1024];
            let ok = QueryInformationJobObject(j, JobObjectBasicProcessIdList, tampon.as_mut_ptr() as *mut c_void, (tampon.len() * std::mem::size_of::<usize>()) as u32, std::ptr::null_mut());
            CloseHandle(j);
            if ok == 0 {
                return Vec::new();
            }
            let n = (*(tampon.as_ptr() as *const [u32; 2]))[1] as usize;
            tampon[1..].iter().take(n.min(1024)).map(|p| *p as u32).collect()
        }
    }

    pub fn arreter(job: &str) -> usize {
        let n = pids(job).len();
        unsafe {
            let nom = w(job);
            let j = OpenJobObjectW(JOB_OBJECT_TERMINATE, 0, nom.as_ptr());
            if j.is_null() {
                return 0;
            }
            let ok = TerminateJobObject(j, 1);
            CloseHandle(j);
            if ok == 0 {
                0
            } else {
                n
            }
        }
    }

    pub fn choisir_dossier(titre: &str) -> Option<String> {
        use windows_sys::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED};
        use windows_sys::Win32::UI::Shell::{SHBrowseForFolderW, SHGetPathFromIDListW, BROWSEINFOW};
        const BIF_RETURNONLYFSDIRS: u32 = 0x1;
        const BIF_NEWDIALOGSTYLE: u32 = 0x40;
        const BIF_NONEWFOLDERBUTTON: u32 = 0x200;
        let titre = w(titre);
        unsafe {
            let com = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
            let mut bi: BROWSEINFOW = std::mem::zeroed();
            bi.hwndOwner = windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow();
            bi.lpszTitle = titre.as_ptr();
            bi.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE | BIF_NONEWFOLDERBUTTON;
            let choix = SHBrowseForFolderW(&bi);
            let mut rendu = None;
            if !choix.is_null() {
                let mut tampon = vec![0u16; 1024];
                if SHGetPathFromIDListW(choix, tampon.as_mut_ptr()) != 0 {
                    let n = tampon.iter().position(|&c| c == 0).unwrap_or(0);
                    rendu = Some(String::from_utf16_lossy(&tampon[..n])).filter(|s| !s.is_empty());
                }
                CoTaskMemFree(choix as *const c_void);
            }
            if com >= 0 {
                CoUninitialize();
            }
            rendu
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFIL: &str = r"C:\Users\Ada Lovelace";

    #[test]
    fn un_sous_dossier_se_reconnait_sans_se_laisser_tromper_par_un_prefixe() {
        assert!(sous(PROFIL, r"C:\Users\Ada Lovelace\Documents"));
        assert!(sous(PROFIL, r"c:\users\ADA LOVELACE"));
        assert!(sous(PROFIL, "C:/Users/Ada Lovelace/Desktop/"));
        // Même début de nom, autre dossier.
        assert!(!sous(PROFIL, r"C:\Users\Ada Lovelace2\Documents"));
        assert!(!sous(PROFIL, r"C:\Users"));
        assert!(!sous("", r"C:\x"));
        assert_eq!(normaliser(r#""C:\a\b\""#), r"C:\a\b");
        assert_eq!(normaliser("C:\\"), "C:\\");
    }

    #[test]
    fn on_ne_donne_ni_un_lecteur_ni_le_profil_entier_ni_la_memoire_de_waly() {
        let garde = vec![r"C:\waly\data".to_string()];
        let non = |c: &str, d: Droit| refus_de_regler(c, d, PROFIL, &garde).is_some();
        assert!(non(r"C:\", Droit::Lecture));
        assert!(non("projets", Droit::Lecture), "chemin relatif");
        assert!(non(r"C:\projets\..\Windows", Droit::Lecture));
        assert!(non(PROFIL, Droit::Lecture));
        assert!(non(r"C:\Users", Droit::Ecriture), "un parent du profil le contient");
        assert!(non(r"C:\Users\Ada Lovelace\AppData\Roaming", Droit::Lecture));
        assert!(non(r"C:\waly\data", Droit::Lecture));
        assert!(non(r"C:\waly", Droit::Ecriture), "contient la mémoire de Waly");
        assert!(non(r"C:\waly\data\sous", Droit::Lecture));
        assert!(non(r"C:\Windows\System32", Droit::Coupe));
        assert!(non(r"C:\Program Files\Outil", Droit::Ecriture));
        // Permis : un dossier précis, dans le profil ou ailleurs.
        assert!(!non(r"C:\Users\Ada Lovelace\Documents\projet", Droit::Ecriture));
        assert!(!non(r"C:\Users\Ada Lovelace\AppData\Local\Programs\Ollama", Droit::Lecture));
        assert!(!non(r"D:\projets\site", Droit::Lecture));
        // Couper est toujours permis hors du système, la mémoire de Waly comprise.
        assert!(!non(r"C:\waly\data", Droit::Coupe));
        assert!(!non(r"C:\waly", Droit::Coupe));
        assert!(!non(r"C:\Users\Ada Lovelace\Documents\prive", Droit::Coupe));
    }

    #[test]
    fn la_ligne_de_lancement_porte_le_chemin_complet_du_programme() {
        let exe = r"C:\Users\Ada Lovelace\AppData\Local\Programs\Ollama\ollama.exe";
        assert_eq!(ligne_de_lancement(exe, "ollama serve"), format!("\"{exe}\" serve"));
        assert_eq!(ligne_de_lancement(exe, &format!("\"{exe}\" serve --port 1")), format!("\"{exe}\" serve --port 1"));
        assert_eq!(ligne_de_lancement(exe, ""), format!("\"{exe}\""));
        assert_eq!(ligne_de_lancement(exe, "ollama"), format!("\"{exe}\""));
        assert_eq!(sans_programme(r#"  "C:\a b\x.exe"   -m  hermes "#), "-m  hermes ");
        assert_eq!(mots(r#"node "C:\a b\index.js" --x=1"#), vec!["node", r"C:\a b\index.js", "--x=1"]);
    }

    #[test]
    fn les_besoins_de_demarrage_ne_sortent_pas_du_profil() {
        // Programme installé dans le profil : son dossier, en lecture.
        let ollama = r"C:\Users\Ada Lovelace\AppData\Local\Programs\Ollama\ollama.exe";
        assert_eq!(besoins(ollama, "ollama serve", PROFIL), vec![r"C:\Users\Ada Lovelace\AppData\Local\Programs\Ollama"]);
        // Moteur hors profil, paquet Node dans le profil : le dossier node_modules.
        let node = r"C:\Program Files\nodejs\node.exe";
        let l = r#"node "C:\Users\Ada Lovelace\AppData\Roaming\npm\node_modules\openclaw\dist\index.js" --port 8080"#;
        assert_eq!(besoins(node, l, PROFIL), vec![r"C:\Users\Ada Lovelace\AppData\Roaming\npm\node_modules"]);
        // Un script et un dossier nommés en argument ; ce qui est hors profil est ignoré.
        let py = r"C:\Python314\python.exe";
        let l = r#"python "C:\Users\Ada Lovelace\agents\hermes\run.py" --data=C:\Users\Ada Lovelace\agents\hermes\data D:\ailleurs\x.txt"#;
        assert_eq!(besoins(py, l, PROFIL), vec![r"C:\Users\Ada Lovelace\agents\hermes"]);
        // Tout est hors du profil : rien à donner.
        assert!(besoins(node, r"node D:\outils\agent.js", PROFIL).is_empty());
        // Jamais le profil entier, même si le programme est posé à sa racine.
        assert!(besoins(r"C:\Users\Ada Lovelace\agent.exe", "", PROFIL).is_empty());
    }

    #[test]
    fn le_mot_de_passe_suit_les_regles_de_windows_et_change_avec_l_alea() {
        let a = mot_de_passe(&[0u8; 32]);
        let b = mot_de_passe(&[7u8; 32]);
        assert_eq!(a.chars().count(), 36);
        assert_ne!(a, b);
        assert!(a.len() >= waly_seal::enclos::MDP_MIN);
        for m in [&a, &b] {
            assert!(m.chars().any(|c| c.is_ascii_uppercase()) && m.chars().any(|c| c.is_ascii_lowercase()));
            assert!(m.chars().any(|c| c.is_ascii_digit()) && m.chars().any(|c| !c.is_ascii_alphanumeric()));
            assert!(!m.contains(['"', ' ', '%']), "{m}");
        }
    }

    #[test]
    fn les_sondes_et_le_job_sont_stables() {
        assert_eq!(ligne_sonde(r"C:\a b\", false), r#"cmd.exe /c dir /a "C:\a b" >nul 2>&1"#);
        let e = ligne_sonde(r"C:\a b", true);
        assert!(e.starts_with(r#"cmd.exe /c copy /y nul "C:\a b\~waly-essai.tmp""#) && e.contains("&& del /q"), "{e}");
        assert_eq!(nom_job(r"C:\X\Agent.exe"), nom_job("c:/x/agent.exe"));
        assert_ne!(nom_job(r"C:\X\agent.exe"), nom_job(r"C:\Y\agent.exe"));
        assert!(nom_job("x").starts_with("Local\\WalyEnclos-"));
    }

    fn base() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE reglages (cle TEXT PRIMARY KEY, valeur TEXT NOT NULL, updated_at TEXT NOT NULL DEFAULT (datetime('now')));").unwrap();
        migrer(&c).unwrap();
        c
    }

    #[test]
    fn un_dossier_n_a_qu_un_reglage_et_un_essai_dit_s_il_tient() {
        let c = base();
        inscrire(&c, r"C:\projets\site", Some(Droit::Lecture), "").unwrap();
        inscrire(&c, r"c:\PROJETS\site", Some(Droit::Ecriture), "").unwrap();
        let d = dossiers(&c);
        assert_eq!(d.len(), 1, "même dossier, autre casse : un seul réglage");
        assert_eq!(d[0].droit, Droit::Ecriture);
        assert_eq!(d[0].conforme(), None, "pas encore essayé");
        inscrire_essai(&c, r"C:\projets\site", true, true);
        assert_eq!(dossiers(&c)[0].conforme(), Some(true));
        // Changer le réglage efface l'ancien essai : il ne prouve plus rien.
        inscrire(&c, r"C:\projets\site", Some(Droit::Coupe), "").unwrap();
        assert_eq!(dossiers(&c)[0].conforme(), None);
        inscrire_essai(&c, r"C:\projets\site", true, false);
        assert_eq!(dossiers(&c)[0].conforme(), Some(false), "coupé, mais l'enclos a lu : l'essai le dit");
        inscrire(&c, r"C:\projets\site", None, "").unwrap();
        assert!(dossiers(&c).is_empty());
    }

    #[test]
    fn sans_compte_rien_n_est_dit_pret() {
        let c = base();
        assert!(secret(&c).is_err());
        assert!(!pret(&c));
        assert!(essai_profil(&c).is_none());
        assert_eq!(Droit::depuis("lecture"), Some(Droit::Lecture));
        assert_eq!(Droit::depuis("tout"), None);
        assert_eq!(Droit::Coupe.attendu(), (false, false));
    }
}
