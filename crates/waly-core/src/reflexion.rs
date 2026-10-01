//! Réflexion approfondie (lot 3 des « bientôt », 2026-10-01).
//!
//! Le cerveau local par défaut ne « pense » pas (qwen3 instruct : aucun
//! raisonnement natif, `reasoning_effort` ignoré — banc du 01/10). La
//! réflexion est donc DEMANDÉE : une consigne en queue du message fait écrire
//! au modèle son raisonnement entre balises, puis sa réponse ; le flux est
//! séparé ici (la réflexion part vers un bloc repliable, la réponse vers la
//! bulle). Un modèle qui raisonne nativement (`<think>` brut, ou champ
//! `reasoning` du serveur ré-emballé par `LlmClient::reflexion`) passe par le
//! même filtre.
//!
//! Un seul appel, mêmes outils, consigne dans le message : le système et le
//! bloc d'outils ne bougent pas (discipline append-only R4.5).

/// Consigne ajoutée EN QUEUE du message utilisateur (jamais persistée).
pub const CONSIGNE: &str = "\n\n(Réflexion approfondie activée : commence par réfléchir \
entre <reflexion> et </reflexion> — ce qui est demandé, ce que tu sais, les pièges, les \
calculs posés un par un, ton plan — puis, après </reflexion>, donne ta réponse sans \
répéter la réflexion.)";

/// Relance quand le modèle a réfléchi sans conclure (plafond de tokens
/// atteint dans la réflexion, ou arrêt après la balise fermante).
pub const RELANCE: &str = "Donne maintenant ta réponse finale, directement, sans balise.";

/// Balises reconnues : la nôtre, et celles des modèles qui raisonnent.
const BALISES: [&str; 4] = ["reflexion", "réflexion", "think", "thinking"];

/// Ce qu'un fragment de flux contient, une fois séparé.
#[derive(Debug, Default, PartialEq)]
pub struct Morceaux {
    pub reflexion: String,
    pub reponse: String,
}

/// Séparateur AU FIL DE L'EAU : les balises peuvent arriver coupées entre deux
/// deltas — un suffixe qui pourrait être un début de balise est retenu.
#[derive(Default)]
pub struct Filtre {
    retenu: String,
    /// Balise ouverte en cours (index dans [`BALISES`]).
    dedans: Option<usize>,
}

impl Filtre {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, delta: &str) -> Morceaux {
        let mut m = Morceaux::default();
        self.retenu.push_str(delta);
        loop {
            match self.dedans {
                None => {
                    let ouvre = BALISES
                        .iter()
                        .enumerate()
                        .filter_map(|(k, b)| self.retenu.find(&format!("<{b}>")).map(|i| (i, k)))
                        .min();
                    match ouvre {
                        Some((i, k)) => {
                            m.reponse.push_str(&self.retenu[..i]);
                            self.retenu.drain(..i + BALISES[k].len() + 2);
                            self.dedans = Some(k);
                        }
                        None => {
                            let garde = suffixe_ambigu(&self.retenu, |b| format!("<{b}>"));
                            let fin = self.retenu.len() - garde;
                            m.reponse.push_str(&self.retenu[..fin]);
                            self.retenu.drain(..fin);
                            return m;
                        }
                    }
                }
                Some(k) => {
                    let ferme = format!("</{}>", BALISES[k]);
                    match self.retenu.find(&ferme) {
                        Some(i) => {
                            m.reflexion.push_str(&self.retenu[..i]);
                            self.retenu.drain(..i + ferme.len());
                            self.dedans = None;
                        }
                        None => {
                            let garde = suffixe_ambigu(&self.retenu, |b| format!("</{b}>"));
                            let fin = self.retenu.len() - garde;
                            m.reflexion.push_str(&self.retenu[..fin]);
                            self.retenu.drain(..fin);
                            return m;
                        }
                    }
                }
            }
        }
    }

    /// Fin de flux : rend ce qui restait retenu, du côté où l'on se trouvait
    /// (une balise jamais fermée reste de la réflexion — jamais de texte avalé).
    pub fn finish(&mut self) -> Morceaux {
        let reste = std::mem::take(&mut self.retenu);
        let dedans = self.dedans.take().is_some();
        if dedans {
            Morceaux { reflexion: reste, reponse: String::new() }
        } else {
            Morceaux { reflexion: String::new(), reponse: reste }
        }
    }
}

/// Longueur du plus long suffixe de `s` qui est un PRÉFIXE strict d'une balise.
fn suffixe_ambigu(s: &str, balise: impl Fn(&str) -> String) -> usize {
    let balises: Vec<String> = BALISES.iter().map(|b| balise(b)).collect();
    let Some(i) = s.rfind('<') else { return 0 };
    let queue = &s[i..];
    if balises.iter().any(|b| b.len() > queue.len() && b.starts_with(queue)) {
        queue.len()
    } else {
        0
    }
}

/// (réflexion, réponse) d'un texte complet.
pub fn separer(texte: &str) -> (String, String) {
    let mut f = Filtre::new();
    let mut m = f.feed(texte);
    let fin = f.finish();
    m.reflexion.push_str(&fin.reflexion);
    m.reponse.push_str(&fin.reponse);
    (m.reflexion.trim().to_string(), m.reponse.trim().to_string())
}

/// La réponse seule (ce qui entre dans l'historique et en base).
pub fn retirer(texte: &str) -> String {
    separer(texte).1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tout(deltas: &[&str]) -> (String, String) {
        let mut f = Filtre::new();
        let (mut r, mut p) = (String::new(), String::new());
        for d in deltas {
            let m = f.feed(d);
            r.push_str(&m.reflexion);
            p.push_str(&m.reponse);
        }
        let m = f.finish();
        r.push_str(&m.reflexion);
        p.push_str(&m.reponse);
        (r, p)
    }

    #[test]
    fn separe_reflexion_et_reponse() {
        let (r, p) = tout(&["<reflexion>je pose le calcul</reflexion>\nOui, ça tient."]);
        assert_eq!(r, "je pose le calcul");
        assert_eq!(p, "\nOui, ça tient.");
    }

    #[test]
    fn balises_coupees_entre_deux_deltas() {
        let (r, p) = tout(&["<refl", "exion>un", " deux</ref", "lexion>", "Voilà."]);
        assert_eq!(r, "un deux");
        assert_eq!(p, "Voilà.");
    }

    #[test]
    fn un_chevron_ordinaire_n_est_pas_retenu_pour_toujours() {
        let (r, p) = tout(&["a < b et <b>gras</b>", " fin <"]);
        assert_eq!(r, "");
        assert_eq!(p, "a < b et <b>gras</b> fin <");
    }

    #[test]
    fn think_natif_et_balise_jamais_fermee() {
        assert_eq!(separer("<think>hum</think>Réponse."), ("hum".into(), "Réponse.".into()));
        // Coupé en pleine réflexion : rien ne passe pour une réponse.
        assert_eq!(separer("<reflexion>je commence"), ("je commence".into(), String::new()));
        assert_eq!(retirer("Sans balise."), "Sans balise.");
    }

    #[test]
    fn texte_avant_la_balise_reste_de_la_reponse() {
        let (r, p) = tout(&["D'accord. <reflexion>x</reflexion> Fin."]);
        assert_eq!(r, "x");
        assert_eq!(p, "D'accord.  Fin.");
    }
}
