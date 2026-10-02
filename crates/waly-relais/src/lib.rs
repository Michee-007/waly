//! waly-relais — le relais de partage entre deux Waly.
//!
//! Il ne sait faire qu'une chose : garder des **enveloppes chiffrées** dans
//! des **boîtes**, le temps que leur destinataire les relève. Il ne voit
//! jamais le contenu (chiffré de bout en bout par les deux Waly), ne connaît
//! aucun compte, n'écrit rien sur disque.
//!
//! Protocole (HTTP, corps JSON) :
//! - `GET /v1/boites/<boite>` + `Authorization: Bearer <jeton>` (+ `X-Attente:
//!   <s>`) : relève. La PREMIÈRE relève ouvre la boîte et lui attache ce
//!   jeton ; ensuite seul ce jeton la relève. Attente longue jusqu'à
//!   l'arrivée d'une enveloppe. → `{"messages":[{"id":n,"corps":"…"}]}`
//! - `POST /v1/boites/<boite>` : dépôt d'une enveloppe dans une boîte
//!   OUVERTE (l'identifiant de boîte, 128 bits tirés au hasard, n'est connu
//!   que des contacts du destinataire). → 201
//! - `DELETE /v1/boites/<boite>/<id>` + jeton : l'enveloppe relevée quitte le
//!   relais.
//! - `GET /v1/sante` → `ok`
//!
//! Bornes : enveloppe ≤ 1 Mo, 100 enveloppes par boîte, 7 jours de garde,
//! 5 000 boîtes ; tout vit en mémoire (un redémarrage vide le relais — les
//! expéditeurs renvoient). Le TLS se met devant (Caddy, nginx).

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub const ENVELOPPE_MAX: usize = 1_000_000;
pub const PAR_BOITE: usize = 100;
pub const BOITES_MAX: usize = 5_000;
pub const GARDE: Duration = Duration::from_secs(7 * 24 * 3600);
/// Une boîte jamais relevée depuis ce délai est fermée.
pub const ABANDON: Duration = Duration::from_secs(30 * 24 * 3600);
const ATTENTE_MAX: u64 = 25;
const ENTETES_MAX: usize = 8192;

struct Boite {
    jeton: String,
    messages: VecDeque<(u64, String, Instant)>,
    suivant: u64,
    vue: Instant,
}

#[derive(Default)]
pub struct Relais {
    boites: Mutex<HashMap<String, Boite>>,
    arrivee: Condvar,
}

/// Une requête, déjà lue.
pub struct Requete {
    pub methode: String,
    pub chemin: String,
    pub jeton: Option<String>,
    pub attente: u64,
    pub corps: String,
}

fn hexa(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Comparaison en temps constant (le jeton ne fuit pas par le chronomètre).
fn egal(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Relais {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Traite une requête : (statut HTTP, corps JSON).
    pub fn traiter(&self, r: &Requete) -> (u16, String) {
        let parts: Vec<&str> = r.chemin.trim_matches('/').split('/').collect();
        match (r.methode.as_str(), parts.as_slice()) {
            ("GET", ["v1", "sante"]) => (200, "\"ok\"".into()),
            ("POST", ["v1", "boites", b]) if hexa(b, 32) => self.deposer(b, &r.corps),
            ("GET", ["v1", "boites", b]) if hexa(b, 32) => self.relever(b, r.jeton.as_deref(), r.attente),
            ("DELETE", ["v1", "boites", b, id]) if hexa(b, 32) => self.effacer(b, r.jeton.as_deref(), id),
            _ => (404, erreur("introuvable")),
        }
    }

    fn purger(boites: &mut HashMap<String, Boite>) {
        let maintenant = Instant::now();
        boites.retain(|_, b| maintenant.duration_since(b.vue) < ABANDON);
        for b in boites.values_mut() {
            while b.messages.front().is_some_and(|m| maintenant.duration_since(m.2) > GARDE) {
                b.messages.pop_front();
            }
        }
    }

    fn deposer(&self, boite: &str, corps: &str) -> (u16, String) {
        if corps.is_empty() {
            return (400, erreur("enveloppe vide"));
        }
        if corps.len() > ENVELOPPE_MAX {
            return (413, erreur("enveloppe trop grande"));
        }
        let mut boites = self.boites.lock().unwrap_or_else(|e| e.into_inner());
        Self::purger(&mut boites);
        let Some(b) = boites.get_mut(boite) else {
            return (404, erreur("boite inconnue"));
        };
        if b.messages.len() >= PAR_BOITE {
            return (429, erreur("boite pleine"));
        }
        b.suivant += 1;
        b.messages.push_back((b.suivant, corps.to_string(), Instant::now()));
        self.arrivee.notify_all();
        (201, "{\"depose\":true}".into())
    }

    fn relever(&self, boite: &str, jeton: Option<&str>, attente: u64) -> (u16, String) {
        let Some(jeton) = jeton.filter(|j| hexa(j, 64)) else {
            return (401, erreur("jeton requis"));
        };
        let fin = Instant::now() + Duration::from_secs(attente.min(ATTENTE_MAX));
        let mut boites = self.boites.lock().unwrap_or_else(|e| e.into_inner());
        Self::purger(&mut boites);
        loop {
            match boites.get_mut(boite) {
                Some(b) if !egal(&b.jeton, jeton) => return (403, erreur("cette boite appartient a un autre jeton")),
                Some(b) => {
                    b.vue = Instant::now();
                    if !b.messages.is_empty() {
                        let l: Vec<serde_json::Value> =
                            b.messages.iter().map(|(id, c, _)| serde_json::json!({"id": id, "corps": c})).collect();
                        return (200, serde_json::json!({"messages": l}).to_string());
                    }
                }
                None => {
                    if boites.len() >= BOITES_MAX {
                        return (507, erreur("relais plein"));
                    }
                    boites.insert(
                        boite.to_string(),
                        Boite { jeton: jeton.to_string(), messages: VecDeque::new(), suivant: 0, vue: Instant::now() },
                    );
                }
            }
            let reste = fin.saturating_duration_since(Instant::now());
            if reste.is_zero() {
                return (200, "{\"messages\":[]}".into());
            }
            boites = self.arrivee.wait_timeout(boites, reste).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    fn effacer(&self, boite: &str, jeton: Option<&str>, id: &str) -> (u16, String) {
        let Ok(id) = id.parse::<u64>() else { return (400, erreur("identifiant invalide")) };
        let mut boites = self.boites.lock().unwrap_or_else(|e| e.into_inner());
        match (boites.get_mut(boite), jeton) {
            (Some(b), Some(j)) if egal(&b.jeton, j) => {
                b.messages.retain(|m| m.0 != id);
                (200, "{\"efface\":true}".into())
            }
            (Some(_), _) => (403, erreur("jeton refuse")),
            (None, _) => (404, erreur("boite inconnue")),
        }
    }

    /// (boîtes ouvertes, enveloppes en attente) — pour le journal du relais.
    pub fn etat(&self) -> (usize, usize) {
        let b = self.boites.lock().unwrap_or_else(|e| e.into_inner());
        (b.len(), b.values().map(|x| x.messages.len()).sum())
    }
}

fn erreur(m: &str) -> String {
    serde_json::json!({"erreur": m}).to_string()
}

/// Lit une requête HTTP/1.1 (en-têtes ≤ 8 Ko, corps borné par
/// `Content-Length` ≤ [`ENVELOPPE_MAX`]). `Err(statut)` si elle est refusée.
pub fn lire(flux: &mut impl Read) -> Result<Requete, u16> {
    let mut tampon = Vec::new();
    let mut bloc = [0u8; 4096];
    let fin = loop {
        // La borne porte sur les EN-TÊTES eux-mêmes : un bloc lu peut dépasser
        // 8 Ko d'un coup et contenir déjà la fin des en-têtes.
        if let Some(i) = tampon.windows(4).position(|w| w == b"\r\n\r\n") {
            if i > ENTETES_MAX {
                return Err(431);
            }
            break i;
        }
        if tampon.len() > ENTETES_MAX {
            return Err(431);
        }
        match flux.read(&mut bloc) {
            Ok(0) | Err(_) => return Err(400),
            Ok(n) => tampon.extend_from_slice(&bloc[..n]),
        }
    };
    let tete = String::from_utf8_lossy(&tampon[..fin]).into_owned();
    let mut lignes = tete.split("\r\n");
    let mut premiere = lignes.next().unwrap_or("").split(' ');
    let (methode, chemin) = (premiere.next().unwrap_or("").to_string(), premiere.next().unwrap_or("").to_string());
    let (mut longueur, mut jeton, mut attente) = (0usize, None, 0u64);
    for l in lignes {
        let Some((nom, valeur)) = l.split_once(':') else { continue };
        let valeur = valeur.trim();
        match nom.to_ascii_lowercase().as_str() {
            "content-length" => longueur = valeur.parse().map_err(|_| 400u16)?,
            "authorization" => jeton = valeur.strip_prefix("Bearer ").map(|j| j.trim().to_string()),
            "x-attente" => attente = valeur.parse().unwrap_or(0),
            _ => {}
        }
    }
    if longueur > ENVELOPPE_MAX {
        return Err(413);
    }
    let mut corps = tampon[fin + 4..].to_vec();
    while corps.len() < longueur {
        match flux.read(&mut bloc) {
            Ok(0) | Err(_) => return Err(400),
            Ok(n) => corps.extend_from_slice(&bloc[..n]),
        }
    }
    corps.truncate(longueur);
    Ok(Requete { methode, chemin: chemin.split('?').next().unwrap_or("").to_string(), jeton, attente, corps: String::from_utf8(corps).map_err(|_| 400u16)? })
}

fn repondre(flux: &mut TcpStream, statut: u16, corps: &str) {
    let _ = write!(
        flux,
        "HTTP/1.1 {statut} -\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{corps}",
        corps.len()
    );
}

/// Sert le relais sur `ecoute` (un fil par connexion, connexions bornées).
/// Rend la main seulement sur erreur d'écoute.
pub fn servir(relais: Arc<Relais>, ecoute: TcpListener) -> std::io::Result<()> {
    let ouvertes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for flux in ecoute.incoming() {
        let Ok(mut flux) = flux else { continue };
        if ouvertes.load(std::sync::atomic::Ordering::Relaxed) >= 2_000 {
            repondre(&mut flux, 503, &erreur("relais occupe"));
            continue;
        }
        let (relais, ouvertes) = (relais.clone(), ouvertes.clone());
        ouvertes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::thread::spawn(move || {
            // Une requête lente ne tient pas un fil indéfiniment.
            let _ = flux.set_read_timeout(Some(Duration::from_secs(15)));
            match lire(&mut flux) {
                Ok(r) => {
                    let (statut, corps) = relais.traiter(&r);
                    repondre(&mut flux, statut, &corps);
                }
                Err(statut) => repondre(&mut flux, statut, &erreur("requete refusee")),
            }
            ouvertes.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOITE: &str = "0123456789abcdef0123456789abcdef";
    const JETON: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const AUTRE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn req(methode: &str, chemin: &str, jeton: Option<&str>, corps: &str) -> Requete {
        Requete { methode: methode.into(), chemin: chemin.into(), jeton: jeton.map(String::from), attente: 0, corps: corps.into() }
    }

    #[test]
    fn une_boite_s_ouvre_a_la_premiere_releve_puis_garde_son_jeton() {
        let r = Relais::new();
        let chemin = format!("/v1/boites/{BOITE}");
        // Dépôt dans une boîte jamais ouverte : refusé (pas de remplissage à l'aveugle).
        assert_eq!(r.traiter(&req("POST", &chemin, None, "env")).0, 404);
        assert_eq!(r.traiter(&req("GET", &chemin, None, "")).0, 401);
        assert_eq!(r.traiter(&req("GET", &chemin, Some(JETON), "")), (200, "{\"messages\":[]}".into()));
        assert_eq!(r.traiter(&req("POST", &chemin, None, "enveloppe-1")).0, 201);
        assert_eq!(r.traiter(&req("POST", &chemin, None, "enveloppe-2")).0, 201);
        // Un autre jeton ne relève ni n'efface.
        assert_eq!(r.traiter(&req("GET", &chemin, Some(AUTRE), "")).0, 403);
        assert_eq!(r.traiter(&req("DELETE", &format!("{chemin}/1"), Some(AUTRE), "")).0, 403);
        let (statut, corps) = r.traiter(&req("GET", &chemin, Some(JETON), ""));
        let v: serde_json::Value = serde_json::from_str(&corps).unwrap();
        assert_eq!((statut, v["messages"].as_array().unwrap().len()), (200, 2));
        assert_eq!(v["messages"][0]["corps"], "enveloppe-1");
        // Relevée puis effacée : elle quitte le relais.
        assert_eq!(r.traiter(&req("DELETE", &format!("{chemin}/1"), Some(JETON), "")).0, 200);
        assert_eq!(r.etat(), (1, 1));
    }

    #[test]
    fn bornes_et_chemins() {
        let r = Relais::new();
        let chemin = format!("/v1/boites/{BOITE}");
        r.traiter(&req("GET", &chemin, Some(JETON), ""));
        assert_eq!(r.traiter(&req("POST", &chemin, None, &"x".repeat(ENVELOPPE_MAX + 1))).0, 413);
        assert_eq!(r.traiter(&req("POST", &chemin, None, "")).0, 400);
        for _ in 0..PAR_BOITE {
            assert_eq!(r.traiter(&req("POST", &chemin, None, "e")).0, 201);
        }
        assert_eq!(r.traiter(&req("POST", &chemin, None, "e")).0, 429);
        assert_eq!(r.traiter(&req("GET", "/v1/boites/pas-une-boite", Some(JETON), "")).0, 404);
        assert_eq!(r.traiter(&req("GET", "/v1/sante", None, "")).0, 200);
        assert_eq!(r.traiter(&req("GET", "/", None, "")).0, 404);
    }

    #[test]
    fn l_attente_longue_se_reveille_a_l_arrivee() {
        let r = Relais::new();
        let chemin = format!("/v1/boites/{BOITE}");
        r.traiter(&req("GET", &chemin, Some(JETON), ""));
        let (r2, c2) = (r.clone(), chemin.clone());
        let depot = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            r2.traiter(&req("POST", &c2, None, "arrivee"))
        });
        let t0 = Instant::now();
        let mut attente = req("GET", &chemin, Some(JETON), "");
        attente.attente = 10;
        let (statut, corps) = r.traiter(&attente);
        assert!(statut == 200 && corps.contains("arrivee") && t0.elapsed() < Duration::from_secs(5));
        assert_eq!(depot.join().unwrap().0, 201);
    }

    #[test]
    fn sans_bon_jeton_ni_releve_ni_effacement() {
        let r = Relais::new();
        let chemin = format!("/v1/boites/{BOITE}");
        // Un jeton malformé n'ouvre PAS de boîte (sinon n'importe quoi la réserverait).
        for mauvais in ["", "court", &"g".repeat(64), &"a".repeat(63), &"a".repeat(65)] {
            assert_eq!(r.traiter(&req("GET", &chemin, Some(mauvais), "")).0, 401, "{mauvais:?}");
        }
        assert_eq!(r.etat(), (0, 0));
        assert_eq!(r.traiter(&req("POST", &chemin, None, "env")).0, 404);
        // Le premier jeton valable ouvre la boîte et en devient le seul maître.
        assert_eq!(r.traiter(&req("GET", &chemin, Some(JETON), "")).0, 200);
        r.traiter(&req("POST", &chemin, None, "secret-chiffre"));
        for (jeton, attendu) in [(None, 401), (Some(AUTRE), 403), (Some("court"), 401)] {
            let (statut, corps) = r.traiter(&req("GET", &chemin, jeton, ""));
            assert_eq!(statut, attendu);
            assert!(!corps.contains("secret-chiffre"), "aucune enveloppe ne fuit sur un refus");
        }
        for jeton in [None, Some(AUTRE)] {
            assert_eq!(r.traiter(&req("DELETE", &format!("{chemin}/1"), jeton, "")).0, 403);
        }
        assert_eq!(r.etat(), (1, 1), "l'enveloppe est toujours la");
        // Le dépôt, lui, ne demande pas de jeton : connaître la boîte suffit.
        assert_eq!(r.traiter(&req("POST", &chemin, Some(AUTRE), "autre")).0, 201);
    }

    #[test]
    fn les_reponses_ne_rendent_jamais_le_jeton() {
        let r = Relais::new();
        let chemin = format!("/v1/boites/{BOITE}");
        r.traiter(&req("GET", &chemin, Some(JETON), ""));
        r.traiter(&req("POST", &chemin, None, "e"));
        for q in [
            req("GET", &chemin, Some(JETON), ""),
            req("GET", &chemin, Some(AUTRE), ""),
            req("DELETE", &format!("{chemin}/1"), Some(AUTRE), ""),
            req("POST", &chemin, None, "e"),
            req("GET", "/v1/sante", None, ""),
        ] {
            assert!(!r.traiter(&q).1.contains(JETON));
        }
    }

    #[test]
    fn effacement_identifiants_et_boite_pleine() {
        let r = Relais::new();
        let chemin = format!("/v1/boites/{BOITE}");
        assert_eq!(r.traiter(&req("DELETE", &format!("{chemin}/1"), Some(JETON), "")).0, 404, "boite inconnue");
        r.traiter(&req("GET", &chemin, Some(JETON), ""));
        assert_eq!(r.traiter(&req("DELETE", &format!("{chemin}/abc"), Some(JETON), "")).0, 400);
        assert_eq!(r.traiter(&req("DELETE", &format!("{chemin}/99"), Some(JETON), "")).0, 200, "effacer deux fois ne casse rien");
        for _ in 0..PAR_BOITE {
            r.traiter(&req("POST", &chemin, None, "e"));
        }
        assert_eq!(r.traiter(&req("POST", &chemin, None, "e")).0, 429);
        // Une enveloppe relevée et effacée libère une place ; les identifiants
        // ne sont jamais réutilisés (un effacement tardif ne vise pas une autre).
        assert_eq!(r.traiter(&req("DELETE", &format!("{chemin}/1"), Some(JETON), "")).0, 200);
        assert_eq!(r.traiter(&req("POST", &chemin, None, "nouvelle")).0, 201);
        let v: serde_json::Value = serde_json::from_str(&r.traiter(&req("GET", &chemin, Some(JETON), "")).1).unwrap();
        let ids: Vec<u64> = v["messages"].as_array().unwrap().iter().map(|m| m["id"].as_u64().unwrap()).collect();
        assert_eq!(ids.len(), PAR_BOITE);
        assert!(!ids.contains(&1) && ids.contains(&(PAR_BOITE as u64 + 1)));
        assert!(ids.windows(2).all(|w| w[0] < w[1]), "ordre d'arrivee conserve");
    }

    #[test]
    fn identifiants_de_boite_stricts() {
        let r = Relais::new();
        for boite in ["", "abc", &"z".repeat(32), &"a".repeat(31), &"a".repeat(33), "..", "../v1/sante", "a/b"] {
            let chemin = format!("/v1/boites/{boite}");
            assert_eq!(r.traiter(&req("GET", &chemin, Some(JETON), "")).0, 404, "{boite:?}");
            assert_eq!(r.traiter(&req("POST", &chemin, None, "e")).0, 404, "{boite:?}");
        }
        for (methode, chemin) in [("PUT", format!("/v1/boites/{BOITE}")), ("GET", "/v2/boites".into()), ("POST", "/v1/sante".into())] {
            assert_eq!(r.traiter(&req(methode, &chemin, Some(JETON), "e")).0, 404);
        }
        assert_eq!(r.etat(), (0, 0), "rien n'a ete cree");
    }

    #[test]
    fn comparaison_des_jetons() {
        assert!(egal(JETON, JETON));
        assert!(!egal(JETON, AUTRE));
        assert!(!egal(JETON, &JETON[..63]));
        assert!(!egal("", JETON));
        assert!(egal("", ""));
    }

    #[test]
    fn requetes_http_malformees_refusees() {
        // En-têtes démesurés, longueur annoncée fausse ou énorme, corps non textuel.
        let enorme = format!("GET /v1/sante HTTP/1.1\r\nX-Bourrage: {}\r\n\r\n", "a".repeat(9000));
        assert_eq!(lire(&mut enorme.as_bytes()).err(), Some(431));
        assert_eq!(lire(&mut "POST /x HTTP/1.1\r\nContent-Length: beaucoup\r\n\r\n".as_bytes()).err(), Some(400));
        assert_eq!(lire(&mut "POST /x HTTP/1.1\r\nContent-Length: 10\r\n\r\ncourt".as_bytes()).err(), Some(400), "corps plus court qu'annonce");
        assert_eq!(lire(&mut "GET /x HTTP/1.1\r\n".as_bytes()).err(), Some(400), "en-tetes jamais termines");
        let mut binaire = b"POST /x HTTP/1.1\r\nContent-Length: 2\r\n\r\n".to_vec();
        binaire.extend([0xff, 0xfe]);
        assert_eq!(lire(&mut binaire.as_slice()).err(), Some(400));
        // Noms d'en-têtes insensibles à la casse, chaîne de requête ignorée.
        let r = lire(&mut format!("GET /v1/sante?x=1 HTTP/1.1\r\nauthorization: Bearer {JETON}\r\nx-attente: 3\r\n\r\n").as_bytes()).unwrap();
        assert_eq!((r.chemin.as_str(), r.attente, r.jeton.as_deref()), ("/v1/sante", 3, Some(JETON)));
        // Un autre schéma d'autorisation n'est pas un jeton.
        let r = lire(&mut format!("GET /x HTTP/1.1\r\nAuthorization: Basic {JETON}\r\n\r\n").as_bytes()).unwrap();
        assert_eq!(r.jeton, None);
    }

    #[test]
    fn lecture_http_et_service_reel() {
        let brut = format!("POST /v1/boites/{BOITE} HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nAuthorization: Bearer {JETON}\r\nX-Attente: 7\r\n\r\nhello-en-trop");
        let r = lire(&mut brut.as_bytes()).unwrap();
        assert_eq!((r.methode.as_str(), r.corps.as_str(), r.attente, r.jeton.as_deref()), ("POST", "hello", 7, Some(JETON)));
        let trop = format!("POST /x HTTP/1.1\r\nContent-Length: {}\r\n\r\n", ENVELOPPE_MAX + 1);
        assert_eq!(lire(&mut trop.as_bytes()).err(), Some(413));
        // Bout en bout sur un vrai port.
        let ecoute = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = ecoute.local_addr().unwrap().port();
        let relais = Relais::new();
        std::thread::spawn(move || servir(relais, ecoute));
        let appel = |req: String| {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(req.as_bytes()).unwrap();
            let mut rep = String::new();
            s.read_to_string(&mut rep).unwrap();
            rep
        };
        assert!(appel(format!("GET /v1/boites/{BOITE} HTTP/1.1\r\nAuthorization: Bearer {JETON}\r\n\r\n")).starts_with("HTTP/1.1 200"));
        assert!(appel(format!("POST /v1/boites/{BOITE} HTTP/1.1\r\nContent-Length: 3\r\n\r\nenv")).starts_with("HTTP/1.1 201"));
        assert!(appel(format!("GET /v1/boites/{BOITE} HTTP/1.1\r\nAuthorization: Bearer {JETON}\r\n\r\n")).contains("\"corps\":\"env\""));
    }
}
