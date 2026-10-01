//! `waly-relais [adresse:port]` — défaut `127.0.0.1:8787` (ou la variable
//! `WALY_RELAIS_ECOUTE`). Mettre un mandataire TLS devant pour l'exposer
//! (voir README.md). Aucun contenu, aucune adresse n'est journalisé : une
//! ligne d'état par heure (boîtes ouvertes, enveloppes en attente).

use std::net::TcpListener;

fn main() {
    let adresse = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("WALY_RELAIS_ECOUTE").ok())
        .unwrap_or_else(|| "127.0.0.1:8787".into());
    let ecoute = match TcpListener::bind(&adresse) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("waly-relais : impossible d'ecouter sur {adresse} : {e}");
            std::process::exit(1);
        }
    };
    println!("waly-relais {} : a l'ecoute sur {adresse}", env!("CARGO_PKG_VERSION"));
    let relais = waly_relais::Relais::new();
    {
        let relais = relais.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
            let (boites, enveloppes) = relais.etat();
            println!("etat : {boites} boite(s), {enveloppes} enveloppe(s) en attente");
        });
    }
    if let Err(e) = waly_relais::servir(relais, ecoute) {
        eprintln!("waly-relais : {e}");
        std::process::exit(1);
    }
}
