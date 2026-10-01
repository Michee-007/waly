//! Nettoyage du texte AVANT synthèse vocale.
//!
//! Port fidèle de `clean_for_tts` de l'ancien `voice_loop.py` (logique
//! validée à l'oreille) : la ponctuation forte (. ! ?) reste — le TTS en fait
//! des PAUSES, pas le mot « point ». On retire ce que le TTS vocaliserait à
//! tort : blocs tool_call, emojis, symboles markdown ; on épelle € / % / &.

/// Vrai si le caractère est du « bruit » pour le TTS (emoji, pictogrammes,
/// flèches, puces). Mêmes plages que l'ancien monde.
fn is_tts_noise(c: char) -> bool {
    matches!(c,
        '\u{1F000}'..='\u{1FAFF}'   // emojis / pictogrammes
        | '\u{2600}'..='\u{27BF}'   // symboles divers ☀..➿
        | '\u{2190}'..='\u{21FF}'   // flèches ←..⇿
        | '\u{2B00}'..='\u{2BFF}'   // flèches/symboles ⬀..⯿
        | '\u{FE0E}' | '\u{FE0F}'   // sélecteurs de variante texte/emoji
        | '\u{200B}'..='\u{200F}'   // invisibles : ZWSP, ZWNJ, ZWJ, marques bidi
        | '•')
}

/// Symboles markdown / structure que le TTS lirait à voix haute.
fn is_markdown_symbol(c: char) -> bool {
    matches!(c, '*' | '_' | '`' | '#' | '>' | '|' | '~' | '[' | ']' | '{' | '}' | '<' | '^' | '=')
}

/// Retire les blocs `<tool_call>…</tool_call>` (un tour à outil ne se
/// vocalise pas tel quel — on parle le RÉSULTAT, pas l'appel).
fn strip_tool_calls(text: &str) -> String {
    const OPEN: &str = "<tool_call>";
    const CLOSE: &str = "</tool_call>";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        out.push(' ');
        match rest[start..].find(CLOSE) {
            Some(end_rel) => rest = &rest[start + end_rel + CLOSE.len()..],
            None => return out, // bloc ouvert non fermé : on coupe tout
        }
    }
    out.push_str(rest);
    out
}

/// Remplace les liens markdown `[texte](url)` par leur seul texte — sinon
/// les crochets partent mais l'URL entre parenthèses est vocalisée.
fn strip_markdown_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(mid) = rest.find("](") {
        let open = rest[..mid].rfind('[');
        let close = rest[mid + 2..].find(')').map(|p| mid + 2 + p);
        match (open, close) {
            (Some(o), Some(c)) => {
                out.push_str(&rest[..o]);
                out.push_str(&rest[o + 1..mid]); // le texte du lien
                rest = &rest[c + 1..];
            }
            _ => break, // pas un lien complet : laisser tel quel
        }
    }
    out.push_str(rest);
    out
}

/// Retire les tirets de puce en tête de ligne (« - item ») que le TTS
/// lirait « tiret item ». Ne touche pas aux tirets internes (porte-monnaie).
fn strip_bullets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (n, l) in text.lines().enumerate() {
        if n > 0 {
            out.push('\n');
        }
        let t = l.trim_start();
        let stripped = t
            .strip_prefix("- ")
            .or_else(|| t.strip_prefix("– "))
            .or_else(|| t.strip_prefix("— "))
            .unwrap_or(t);
        out.push_str(stripped);
    }
    out
}

/// Nombre en toutes lettres (0-59, assez pour heures et minutes).
fn fr_number(n: u32, feminine: bool) -> String {
    const UNITS: [&str; 17] = [
        "zéro", "un", "deux", "trois", "quatre", "cinq", "six", "sept", "huit", "neuf", "dix",
        "onze", "douze", "treize", "quatorze", "quinze", "seize",
    ];
    let one = if feminine { "une" } else { "un" };
    match n {
        1 => one.to_string(),
        0..=16 => UNITS[n as usize].to_string(),
        17..=19 => format!("dix-{}", UNITS[(n - 10) as usize]),
        20 | 30 | 40 | 50 => ["vingt", "trente", "quarante", "cinquante"][(n / 10 - 2) as usize].to_string(),
        21 | 31 | 41 | 51 => format!("{} et {one}", fr_number(n - 1 - (n - 1) % 10, false)),
        22..=59 => format!("{}-{}", fr_number(n - n % 10, false), fr_number(n % 10, feminine)),
        _ => n.to_string(),
    }
}

/// Verbalise les heures « 01h09 » / « 14:30 » en toutes lettres — sinon le
/// TTS les épelle comme une suite de chiffres (retour d'écoute Michée).
fn verbalize_times(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 16);
    let mut i = 0;
    while i < chars.len() {
        // Motif : 1-2 chiffres + (h|H|:) + 0-2 chiffres de minutes, non collé
        // à d'autres chiffres. « 8h » (sans minutes) compte aussi — cas très
        // fréquent en français parlé — mais uniquement avec h/H (pas « 8: »)
        // et si rien d'alphanumérique ne suit (« 24heures » reste intact).
        let start_ok = i == 0 || !chars[i - 1].is_ascii_digit();
        if start_ok && chars[i].is_ascii_digit() {
            let h_len = if i + 1 < chars.len() && chars[i + 1].is_ascii_digit() { 2 } else { 1 };
            let sep = i + h_len;
            if sep < chars.len() && matches!(chars[sep], 'h' | 'H' | ':') {
                let d1 = chars.get(sep + 1).is_some_and(|c| c.is_ascii_digit());
                let d2 = chars.get(sep + 2).is_some_and(|c| c.is_ascii_digit());
                let d3 = chars.get(sep + 3).is_some_and(|c| c.is_ascii_digit());
                let mm_len: usize = if d1 && d2 && !d3 {
                    2
                } else if d1 && !d2 {
                    1
                } else {
                    0 // zéro chiffre exploitable (ou ≥ 3 : « 25h999 », intact)
                };
                let bare_hour_ok = mm_len == 0
                    && !d1
                    && matches!(chars[sep], 'h' | 'H')
                    && chars.get(sep + 1).map_or(true, |c| !c.is_alphanumeric());
                if mm_len > 0 || bare_hour_ok {
                    let hh: u32 = chars[i..sep].iter().collect::<String>().parse().unwrap_or(99);
                    let mm: u32 = if mm_len > 0 {
                        chars[sep + 1..sep + 1 + mm_len]
                            .iter()
                            .collect::<String>()
                            .parse()
                            .unwrap_or(99)
                    } else {
                        0
                    };
                    if hh < 24 && mm < 60 {
                        let hours = match hh {
                            0 => "minuit".to_string(),
                            12 => "midi".to_string(),
                            1 => "une heure".to_string(),
                            h => format!("{} heures", fr_number(h, true)),
                        };
                        out.push_str(&hours);
                        if mm != 0 {
                            out.push(' ');
                            out.push_str(&fr_number(mm, true));
                        }
                        i = sep + 1 + mm_len;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Nettoie un texte destiné au TTS. Idempotent.
pub fn clean_for_tts(text: &str) -> String {
    let stripped =
        verbalize_times(&strip_bullets(&strip_markdown_links(&strip_tool_calls(text))));
    let mut out = String::with_capacity(stripped.len());
    for c in stripped.chars() {
        match c {
            '&' => out.push_str(" et "),
            '€' => out.push_str(" euros"),
            '%' => out.push_str(" pour cent"),
            '\u{2019}' => out.push('\''), // apostrophe typographique → droite
            c if is_tts_noise(c) || is_markdown_symbol(c) => out.push(' '),
            c => out.push(c),
        }
    }
    // Effondrer les espaces (équivalent de `\s+` → ` `).
    let mut collapsed = String::with_capacity(out.len());
    let mut in_ws = true; // true pour rogner l'avant
    for c in out.chars() {
        if c.is_whitespace() {
            if !in_ws {
                collapsed.push(' ');
                in_ws = true;
            }
        } else {
            collapsed.push(c);
            in_ws = false;
        }
    }
    while collapsed.ends_with(' ') {
        collapsed.pop();
    }
    collapsed
}

/// Vrai s'il reste quelque chose à prononcer (au moins un alphanumérique).
pub fn has_speech(text: &str) -> bool {
    text.chars().any(|c| c.is_alphanumeric())
}

/// Marqueurs français : mots courants et lettres accentuées.
const FR_WORDS: &[&str] = &[
    "le", "la", "les", "un", "une", "des", "je", "tu", "il", "elle", "on", "nous", "vous",
    "est", "et", "ou", "mais", "oui", "non", "pas", "que", "qui", "quoi", "salut", "bonjour",
    "merci", "bonsoir", "ça", "ca", "c'est", "j'ai", "va", "bien", "d'accord", "voilà",
    "voila", "alors", "donc", "toi", "moi", "aujourd'hui", "être", "avec", "pour", "dans",
];
/// Mots-outils des langues de DÉRIVE de Parakeet sur les énoncés courts :
/// anglais (vécus 2026-07-04/05 : « So it's. », « Hello? », « The key song
/// there. ») puis portugais/espagnol/italien/roumain/allemand (vécus
/// 2026-07-05 : « Hello mon pote » → « Ele m'păr », « ça va et toi » →
/// « Sava aí por »). ⚠ JAMAIS de mot qui existe aussi en français
/// (exclus vérifiés : mais, ma, ce, du, des, si, est, eu, bien, nu, da).
const OFFLANG_WORDS: &[&str] = &[
    // anglais
    "the", "it's", "its", "so", "uh", "um", "what", "hello", "yeah", "yes", "you", "i'm",
    "don't", "that's", "this", "is", "are", "was", "were", "gonna", "okay", "well", "hey",
    "huh", "oh", "know", "like", "just", "and", "but", "have", "got", "get", "right",
    "there", "here", "some", "no", "me", "wow", "excuse", "sorry", "please", "thank", "thanks",
    // portugais / espagnol
    "ele", "ela", "por", "para", "isso", "sim", "com", "uma", "meu", "seu", "tudo",
    "muito", "pero", "esta", "eso", "hola", "gracias", "bueno", "ahora", "aqui", "yo",
    // italien
    "che", "cosa", "questo", "sono", "ciao", "grazie", "bene", "io", "della",
    // roumain / allemand
    "este", "asta", "acum", "ich", "das", "und", "nicht", "nein", "ist",
];

/// Lettres quasi impossibles en orthographe française mais fréquentes dans
/// les langues de dérive (pt/es/it/ro/de/nordiques). Un transcript COURT qui
/// en contient est une confusion de langue, pas du français : « Ele m'păr »
/// (ă roumain), « Sava aí por » (í portugais).
const OFFLANG_CHARS: &str = "ăąáíóúãõñìòșţțßěåøčšžýůđ";

/// Accents du français réel (ä/ö retirés : ils n'existent pas en français
/// et servaient de faux marqueur à une dérive allemande).
const FR_CHARS: &str = "àâçéèêëîïôùûüÿœ";

/// Vrai si le transcript ressemble à une HALLUCINATION HORS-LANGUE de
/// Parakeet : énoncé court (≤ ~2,5 s de parole) sans aucun marqueur français,
/// avec soit une lettre étrangère au français, soit une majorité de
/// mots-outils des langues de dérive. Dérive connue du modèle multilingue
/// (25 langues) sur les énoncés brefs — vécue les 2026-07-04/05
/// (« Salut » → « So it's. », « Hello mon pote » → « Ele m'păr »).
/// ⚠ À désactiver le jour du mode anglais.
pub fn looks_offlang_hallucination(transcript: &str, speech_secs: f32) -> bool {
    if speech_secs > 2.5 {
        return false; // les vrais énoncés longs se transcrivent bien
    }
    let lower = transcript.to_lowercase();
    let words: Vec<String> = lower
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'').to_string())
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return false;
    }
    // Un mot français courant = vrai énoncé (protège aussi les noms propres
    // étrangers dans une phrase française : « on va à São Paulo »).
    if words.iter().any(|w| FR_WORDS.contains(&w.as_str())) {
        return false;
    }
    // Lettre étrangère au français : signal fort, aucune phrase française
    // n'en contient (les bribes « Sava aí por » avaient passé la garde
    // anglaise parce que í/ă ne sont pas des accents français).
    if lower.chars().any(|c| OFFLANG_CHARS.contains(c)) {
        return true;
    }
    if lower.chars().any(|c| FR_CHARS.contains(c)) {
        return false;
    }
    let hits = words.iter().filter(|w| OFFLANG_WORDS.contains(&w.as_str())).count();
    hits * 2 >= words.len()
}

/// Compacte une réponse pour l'HISTORIQUE de conversation (pas pour
/// l'affichage ni le TTS). Mesuré le 2026-07-04 : FLM n'a AUCUN cache de
/// préfixe inter-requêtes — chaque token d'historique est re-préfillé à
/// chaque tour (TTFT 1,1 → 2,2 s en 15 tours). On garde les premières
/// phrases entières sous `max_chars`. Effet de bord vertueux : le modèle
/// voit ses réponses passées courtes et imite (moins de dérive verbeuse).
pub fn compact_reply(text: &str, max_chars: usize) -> String {
    let t = text.trim();
    if t.chars().count() <= max_chars {
        return t.to_string();
    }
    // Dernière fin de phrase avant la limite.
    let mut end = 0;
    for (n, (i, ch)) in t.char_indices().enumerate() {
        if n >= max_chars {
            break;
        }
        if matches!(ch, '.' | '!' | '?' | '…') {
            end = i + ch.len_utf8();
        }
    }
    if end > 0 {
        return t[..end].trim_end().to_string();
    }
    // Aucune fin de phrase : coupe dure sur une frontière de caractère.
    let cut = t
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(t.len());
    let mut s = t[..cut].trim_end().to_string();
    s.push('…');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hallucination_hors_langue_detectee() {
        // Cas vécus sur le terrain (2026-07-04)
        assert!(looks_offlang_hallucination("So it's.", 0.5));
        assert!(looks_offlang_hallucination("Hello?", 0.4));
        assert!(looks_offlang_hallucination("What?", 0.3));
        assert!(looks_offlang_hallucination("Yeah, okay.", 0.6));
        // Passees au travers en conversation reelle (2026-07-05)
        assert!(looks_offlang_hallucination("The key song there.", 0.9));
        assert!(looks_offlang_hallucination("No, some wow.", 0.7));
        assert!(looks_offlang_hallucination("Excuse me.", 0.5));
        // Dérives pt/ro vécues en session R2 (2026-07-05), passaient la
        // garde anglaise : « Hello mon pote » et « ça va et toi »
        assert!(looks_offlang_hallucination("Ele m'păr", 0.8));
        assert!(looks_offlang_hallucination("Sava aí por", 0.9));
        assert!(looks_offlang_hallucination("Okay, bitch.", 0.6));
        assert!(looks_offlang_hallucination("Não, isso.", 0.7));
        // Du vrai français court passe
        assert!(!looks_offlang_hallucination("Salut.", 0.4));
        assert!(!looks_offlang_hallucination("Oui.", 0.3));
        assert!(!looks_offlang_hallucination("Ça va et toi ?", 0.8));
        assert!(!looks_offlang_hallucination("Uh disons un thriller.", 1.2));
        assert!(!looks_offlang_hallucination("D'accord, merci.", 0.7));
        // Mots français HORS liste mais avec accent français : acceptés
        assert!(!looks_offlang_hallucination("Arrête.", 0.4));
        assert!(!looks_offlang_hallucination("Carrément génial.", 0.9));
        // Nom propre étranger dans une phrase française : accepté (mot FR)
        assert!(!looks_offlang_hallucination("On va à São Paulo", 1.5));
        // Bribe inconnue sans signal : on laisse passer (philosophie : ne
        // jamais avaler un vrai mot ; « Fabrico » reste pour la piste
        // re-transcription)
        assert!(!looks_offlang_hallucination("Fabrico", 0.5));
        // Un énoncé long n'est jamais filtré (même sans accents)
        assert!(!looks_offlang_hallucination(
            "The quick brown fox jumps over the lazy dog again",
            4.0
        ));
    }

    #[test]
    fn compact_reply_garde_les_phrases_entieres() {
        let long = "Première phrase courte. Deuxième phrase un peu plus longue pour le test. Troisième phrase qui dépasse largement la limite fixée et devrait disparaître de l'historique compacté.";
        let c = compact_reply(long, 100);
        assert_eq!(c, "Première phrase courte. Deuxième phrase un peu plus longue pour le test.");
        // Court : intact
        assert_eq!(compact_reply("Salut !", 100), "Salut !");
        // Sans ponctuation : coupe dure + ellipse
        let brut = "a".repeat(300);
        let c = compact_reply(&brut, 50);
        assert!(c.chars().count() == 51 && c.ends_with('…'));
    }

    #[test]
    fn garde_la_ponctuation_forte() {
        assert_eq!(clean_for_tts("Bonjour. Ça va ?"), "Bonjour. Ça va ?");
    }

    #[test]
    fn retire_emojis_et_markdown() {
        assert_eq!(clean_for_tts("**Salut** 😀 `code` # titre"), "Salut code titre");
    }

    #[test]
    fn epelle_les_symboles_monetaires() {
        assert_eq!(clean_for_tts("50 € & 10 %"), "50 euros et 10 pour cent");
    }

    #[test]
    fn retire_les_tool_calls() {
        assert_eq!(
            clean_for_tts("Je regarde. <tool_call>{\"name\":\"meteo\"}</tool_call> Voilà !"),
            "Je regarde. Voilà !"
        );
    }

    #[test]
    fn tool_call_non_ferme_coupe_la_suite() {
        assert_eq!(clean_for_tts("Attends <tool_call>{\"na"), "Attends");
    }

    #[test]
    fn apostrophe_typographique() {
        assert_eq!(clean_for_tts("c\u{2019}est"), "c'est");
    }

    #[test]
    fn verbalise_les_heures() {
        assert_eq!(clean_for_tts("Il est 01h09."), "Il est une heure neuf.");
        assert_eq!(clean_for_tts("Rendez-vous à 14h30 !"), "Rendez-vous à quatorze heures trente !");
        assert_eq!(clean_for_tts("Vers 12h00 ou 00h05 ?"), "Vers midi ou minuit cinq ?");
        assert_eq!(clean_for_tts("À 9:21 pile."), "À neuf heures vingt et une pile.");
    }

    #[test]
    fn verbalise_les_heures_sans_minutes() {
        assert_eq!(clean_for_tts("Rendez-vous à 8h."), "Rendez-vous à huit heures.");
        assert_eq!(clean_for_tts("Vers 20h pile."), "Vers vingt heures pile.");
        assert_eq!(clean_for_tts("À 0h tout ferme."), "À minuit tout ferme.");
        // Minutes à un chiffre.
        assert_eq!(clean_for_tts("Il est 14h5."), "Il est quatorze heures cinq.");
        // « 8: » n'est PAS une heure ; « 24heures » reste collé intact.
        assert_eq!(clean_for_tts("Note 8: rien."), "Note 8: rien.");
        assert_eq!(clean_for_tts("ouvert 24heures"), "ouvert 24heures");
    }

    #[test]
    fn liens_markdown_et_puces() {
        assert_eq!(
            clean_for_tts("Regarde [la doc](https://exemple.fr/x) demain."),
            "Regarde la doc demain."
        );
        assert_eq!(clean_for_tts("- premier point\n- second point"), "premier point second point");
        // Un tiret interne n'est pas une puce.
        assert_eq!(clean_for_tts("mon porte-monnaie"), "mon porte-monnaie");
    }

    #[test]
    fn ne_touche_pas_aux_autres_nombres() {
        assert_eq!(clean_for_tts("En 2026, ça vaut 3,14."), "En 2026, ça vaut 3,14.");
        assert_eq!(clean_for_tts("Le score est 25h999."), "Le score est 25h999.");
    }

    #[test]
    fn has_speech_filtre_le_vide() {
        assert!(!has_speech("  ... !! "));
        assert!(has_speech("ok."));
    }
}
