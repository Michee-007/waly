//! Découpage en clauses pour le TTS streaming.
//!
//! Frontière de clause (port de `split_clauses` de voice_loop.py) :
//! une ou plusieurs ponctuations fortes `. ! ? …`, éventuellement suivies
//! d'un guillemet/parenthèse fermant, puis d'un blanc ou de la fin.
//!
//! Version streaming : on pousse les fragments du LLM au fil de l'eau et on
//! récupère les clauses complètes dès qu'elles se ferment — c'est ce qui
//! permet de lancer le TTS sur la première clause sans attendre la réponse
//! entière.
//!
//! Garde « tour à outil » (règle héritée de l'ancien monde) : si le flux
//! entame un `<tool_call>`, on ne stream plus rien — le tour se traite en
//! entier (on vocalisera le résultat de l'outil, pas l'appel).

fn is_strong_punct(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '…')
}

fn is_closer(c: char) -> bool {
    matches!(c, '"' | '\'' | ')' | ']' | '»')
}

/// Abréviations qui ne terminent JAMAIS une phrase : titres et initiales,
/// toujours suivis d'un nom (« M. Dupont », « Dr Martin », « J. K. Rowling »).
/// Comparaison sur le token minuscule précédant le point.
const HARD_ABBREVS: &[&str] = &["m", "mm", "mme", "mmes", "mlle", "mlles", "dr", "pr", "me", "st", "ste"];

/// Abréviations qui peuvent clore une phrase (« ..., etc. Ensuite »).
/// Elles ne bloquent la frontière que si la suite commence en minuscule
/// ou par un chiffre (« cf. page 3 », « env. 10 min »).
const SOFT_ABBREVS: &[&str] =
    &["etc", "cf", "ex", "p", "pp", "art", "av", "bd", "env", "min", "max", "vol", "chap", "fig", "no", "nb"];

/// Découpe un texte complet en clauses (utilitaire non-streaming).
pub fn split_clauses(text: &str) -> Vec<String> {
    let mut splitter = ClauseSplitter::new();
    let mut out = splitter.push(text);
    if let Some(rest) = splitter.flush() {
        out.push(rest);
    }
    out
}

/// Accumulateur streaming : `push()` des fragments, il rend les clauses
/// complètes ; `flush()` rend le reliquat final.
/// Longueur minimale (octets) avant d'accepter une virgule comme frontière
/// de la PREMIÈRE clause (évite de vocaliser « Oui, » tout seul).
const FIRST_COMMA_MIN_BYTES: usize = 16;

#[derive(Debug, Default)]
pub struct ClauseSplitter {
    buf: String,
    /// Position (octets) du début de la clause en cours dans `buf`.
    start: usize,
    /// Un `<tool_call>` a été détecté : plus aucune clause n'est émise.
    tool_call_seen: bool,
    /// Au moins une clause a déjà été émise. Tant que c'est faux, une virgule
    /// suffit comme frontière : le premier son part plus tôt (budget ≤ 1 s).
    emitted: bool,
    /// Autorise la coupe à la virgule pour la PREMIÈRE clause (optimisation
    /// Piper). À DÉSACTIVER pour Pocket : un TTS-modèle-de-langage nourri de
    /// fragments hallucine des sons et avale des mots (terrain 2026-07-04).
    comma_first_clause: bool,
}

impl ClauseSplitter {
    pub fn new() -> Self {
        Self { comma_first_clause: true, ..Self::default() }
    }

    /// Variante « phrases entières » : ne coupe qu'aux ponctuations fortes.
    pub fn sentences_only() -> Self {
        Self::default()
    }

    /// Vrai si le flux a entamé un appel d'outil : le tour ne doit plus être
    /// streamé vers le TTS.
    pub fn tool_call_seen(&self) -> bool {
        self.tool_call_seen
    }

    /// Ajoute un fragment et retourne les clauses complétées par ce fragment.
    pub fn push(&mut self, chunk: &str) -> Vec<String> {
        if self.tool_call_seen {
            // Plus rien ne sera émis ni flushé : ne pas accumuler (le texte
            // complet du tour vit chez l'appelant, pas ici) — buffer borné.
            return Vec::new();
        }
        self.buf.push_str(chunk);
        if self.buf[self.start..].contains("<tool_call") {
            self.tool_call_seen = true;
            return Vec::new();
        }

        let mut clauses = Vec::new();
        loop {
            match self.next_boundary() {
                Some(end) => {
                    let clause = self.buf[self.start..end].trim().to_string();
                    if !clause.is_empty() {
                        clauses.push(clause);
                        self.emitted = true;
                    }
                    self.start = end;
                }
                None => break,
            }
        }
        clauses
    }

    /// Rend le reliquat (dernière clause sans ponctuation finale), s'il y a
    /// quelque chose à dire.
    pub fn flush(&mut self) -> Option<String> {
        let rest = self.buf[self.start..].trim().to_string();
        self.buf.clear();
        self.start = 0;
        if self.tool_call_seen || rest.is_empty() {
            None
        } else {
            Some(rest)
        }
    }

    /// Cherche la prochaine frontière de clause APRÈS `self.start`.
    /// Retourne l'offset (octets) de fin de clause (ponctuation et fermants
    /// inclus). Ne matche que si un blanc ou une suite existe après — en
    /// streaming, un point final de buffer peut encore être suivi d'un
    /// chiffre (« 3.14 ») au fragment suivant.
    fn next_boundary(&self) -> Option<usize> {
        let s = &self.buf[self.start..];
        let mut iter = s.char_indices().peekable();
        while let Some((i, c)) = iter.next() {
            // Première clause : la virgule (ou ; :) suffit, si assez longue.
            if self.comma_first_clause
                && !self.emitted
                && matches!(c, ',' | ';' | ':')
                && i >= FIRST_COMMA_MIN_BYTES
            {
                let end = i + c.len_utf8();
                match s[end..].chars().next() {
                    Some(a) if a.is_whitespace() => return Some(self.start + end),
                    None => return None, // fin de buffer : attendre (« 3,14 »)
                    _ => {}
                }
            }
            if !is_strong_punct(c) {
                continue;
            }
            // Avaler la série de ponctuations fortes puis les fermants, en
            // retenant sa composition (point isolé ? ellipse ? exclamative ?).
            let mut end = i + c.len_utf8();
            let mut n_dots = usize::from(c == '.');
            let mut ellipsis = c == '…';
            let mut emphatic = matches!(c, '!' | '?');
            let mut closed = false;
            let mut chars = s[end..].chars();
            let mut after = chars.next();
            while let Some(a) = after {
                if is_strong_punct(a) || is_closer(a) {
                    n_dots += usize::from(a == '.');
                    ellipsis |= a == '…';
                    emphatic |= matches!(a, '!' | '?');
                    closed |= is_closer(a);
                    end += a.len_utf8();
                    after = chars.next();
                } else {
                    break;
                }
            }
            match after {
                Some(a) if a.is_whitespace() => {
                    // Un point précédé d'un point fait partie d'une ellipse
                    // déjà examinée : même règle que « ... » entier.
                    let prev_dot =
                        s[..i].chars().next_back().is_some_and(|p| matches!(p, '.' | '…'));
                    // « M. Dupont », « cf. page 3 » : un point isolé après
                    // une abréviation n'est pas une fin de phrase.
                    if n_dots == 1 && !prev_dot && !ellipsis && !emphatic && !closed {
                        match abbrev_verdict(s, i, end) {
                            Verdict::NotABoundary => continue,
                            Verdict::WaitForMore => return None,
                            Verdict::Boundary => {}
                        }
                    }
                    // « Attends... j'arrive » : une ellipse suivie d'une
                    // minuscule est une hésitation, pas une fin de clause
                    // (prosody.rs compte dessus pour marquer la réflexion).
                    if (ellipsis || n_dots >= 2 || prev_dot) && !emphatic {
                        match next_word_start(&s[end..]) {
                            None => return None, // attendre le fragment suivant
                            Some(w) if w.is_lowercase() => continue,
                            Some(_) => {}
                        }
                    }
                    return Some(self.start + end);
                }
                // Fin de buffer : frontière incertaine en streaming, on
                // attend le fragment suivant (flush() la rendra sinon).
                None => return None,
                _ => {} // « 3.14 », « M.Dupont » : pas une frontière
            }
        }
        None
    }
}

enum Verdict {
    Boundary,
    NotABoundary,
    /// Il manque du contexte à droite (streaming) pour trancher.
    WaitForMore,
}

/// Premier caractère non blanc, ou None si le buffer s'arrête avant.
fn next_word_start(rest: &str) -> Option<char> {
    rest.chars().find(|c| !c.is_whitespace())
}

/// Le point en `dot_i` (offsets dans `s`) clôt-il une phrase, ou suit-il une
/// abréviation ? `end` pointe après le point.
fn abbrev_verdict(s: &str, dot_i: usize, end: usize) -> Verdict {
    let token: String = s[..dot_i]
        .chars()
        .rev()
        .take_while(|c| c.is_alphabetic())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if token.is_empty() {
        return Verdict::Boundary;
    }
    // Initiale (« J. K. Rowling ») ou titre : jamais une fin de phrase.
    let is_initial = token.chars().count() == 1 && token.chars().all(|c| c.is_uppercase());
    let lower = token.to_lowercase();
    if is_initial || HARD_ABBREVS.contains(&lower.as_str()) {
        return Verdict::NotABoundary;
    }
    // « etc. », « cf. »… : abréviation seulement si la suite ne démarre pas
    // une nouvelle phrase (minuscule ou chiffre).
    if SOFT_ABBREVS.contains(&lower.as_str()) {
        return match next_word_start(&s[end..]) {
            None => Verdict::WaitForMore,
            Some(w) if w.is_lowercase() || w.is_ascii_digit() => Verdict::NotABoundary,
            Some(_) => Verdict::Boundary,
        };
    }
    Verdict::Boundary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoupe_simple() {
        assert_eq!(
            split_clauses("Bonjour ! Il fera beau demain. Prends un parapluie ?"),
            vec!["Bonjour !", "Il fera beau demain.", "Prends un parapluie ?"]
        );
    }

    #[test]
    fn nombre_decimal_non_coupe() {
        assert_eq!(split_clauses("Il fait 3.14 fois plus chaud."), vec!["Il fait 3.14 fois plus chaud."]);
    }

    #[test]
    fn reliquat_sans_ponctuation() {
        assert_eq!(split_clauses("Une phrase. et une traîne"), vec!["Une phrase.", "et une traîne"]);
    }

    #[test]
    fn streaming_emet_des_que_la_clause_se_ferme() {
        let mut sp = ClauseSplitter::new();
        assert!(sp.push("Il fera beau").is_empty());
        assert!(sp.push(" demain.").is_empty()); // fin de buffer : incertain
        assert_eq!(sp.push(" Ensuite"), vec!["Il fera beau demain."]);
        assert_eq!(sp.flush(), Some("Ensuite".to_string()));
    }

    #[test]
    fn tool_call_stoppe_le_streaming() {
        let mut sp = ClauseSplitter::new();
        assert_eq!(sp.push("Je regarde. <tool_call>{\"name\""), Vec::<String>::new());
        assert!(sp.tool_call_seen());
        assert_eq!(sp.flush(), None);
    }

    #[test]
    fn premiere_clause_des_la_virgule() {
        let mut sp = ClauseSplitter::new();
        let out = sp.push("Merci pour ton message, il fait tres beau. Ensuite on verra. ");
        // 1re clause coupee a la virgule, la suite aux points (plus de virgule).
        assert_eq!(
            out,
            vec!["Merci pour ton message,", "il fait tres beau.", "Ensuite on verra."]
        );
    }

    #[test]
    fn virgule_trop_tot_ou_decimale_ignoree() {
        // « Oui, » : trop court pour couper.
        assert_eq!(split_clauses("Oui, bien sur."), vec!["Oui, bien sur."]);
        // Decimale a la francaise : pas d'espace apres la virgule -> pas de coupe.
        assert_eq!(
            split_clauses("Le resultat vaut exactement 3,14 environ."),
            vec!["Le resultat vaut exactement 3,14 environ."]
        );
    }

    #[test]
    fn sentences_only_ignore_la_virgule() {
        let mut sp = ClauseSplitter::sentences_only();
        let out = sp.push("Merci pour ton message, il fait tres beau. Ensuite on verra. ");
        assert_eq!(out, vec!["Merci pour ton message, il fait tres beau.", "Ensuite on verra."]);
    }

    #[test]
    fn abreviations_titres_non_coupees() {
        assert_eq!(
            split_clauses("Bonjour M. Dupont arrive. Mme Martin suit."),
            vec!["Bonjour M. Dupont arrive.", "Mme Martin suit."]
        );
        assert_eq!(split_clauses("Voir J. K. Rowling demain."), vec!["Voir J. K. Rowling demain."]);
    }

    #[test]
    fn abreviations_douces_selon_la_suite() {
        // Suivie d'une minuscule : abréviation, pas de coupe.
        assert_eq!(
            split_clauses("Prends cf. page trois du livre."),
            vec!["Prends cf. page trois du livre."]
        );
        // Suivie d'une majuscule : « etc. » clot bien la phrase.
        assert_eq!(
            split_clauses("Range les outils les vis etc. Ensuite on mange."),
            vec!["Range les outils les vis etc.", "Ensuite on mange."]
        );
    }

    #[test]
    fn ellipse_hesitation_non_coupee() {
        // Minuscule après « ... » : hésitation dans la même clause.
        assert_eq!(split_clauses("Attends... j'arrive tout de suite."), vec!["Attends... j'arrive tout de suite."]);
        // Majuscule après « ... » : vraie fin de clause.
        assert_eq!(split_clauses("Attends... Bon d'accord."), vec!["Attends...", "Bon d'accord."]);
        // Le caractère « … » suit la même règle.
        assert_eq!(split_clauses("Euh… je crois que oui."), vec!["Euh… je crois que oui."]);
    }

    #[test]
    fn tool_call_a_cheval_sur_deux_fragments() {
        let mut sp = ClauseSplitter::new();
        // La clause d'avant part au TTS (le tag n'est pas encore décidable)…
        assert_eq!(sp.push("Je regarde. <tool_"), vec!["Je regarde."]);
        // …mais le tag coupé en deux fragments est bien détecté ensuite.
        assert!(sp.push("call>{\"name\":\"clock\"}").is_empty());
        assert!(sp.tool_call_seen());
        // Après détection, plus rien n'est accumulé ni émis.
        assert!(sp.push("encore du texte. Et encore.").is_empty());
        assert_eq!(sp.flush(), None);
    }

    #[test]
    fn ponctuation_multiple_et_fermants() {
        assert_eq!(
            split_clauses("Quoi ?!» Il est parti. Bon."),
            vec!["Quoi ?!»", "Il est parti.", "Bon."]
        );
    }
}
