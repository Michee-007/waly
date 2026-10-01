//! Documents Office et PDF natifs (Mains v2). Waly écrit en MARKDOWN ;
//! l'extension choisit le format : .docx (Word), .xlsx (Excel), .pptx
//! (PowerPoint), .pdf. Tout est généré ici, en Rust pur, sans dépendance
//! native (archives OOXML via `zipw`).
//!
//! Ce module porte le MODÈLE commun : le markdown est analysé une fois en
//! blocs, que chaque format rend à sa manière.

pub mod docx;
pub mod pdf;
pub mod pptx;
pub mod xlsx;

/// Morceau de texte avec sa mise en forme.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub texte: String,
    pub gras: bool,
    pub italique: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Bloc {
    /// `#` à `######` — niveaux ramenés à 1..=3.
    Titre { niveau: u8, texte: String },
    Paragraphe(Vec<Segment>),
    /// `- `, `* ` ou `• `.
    Puce(Vec<Segment>),
    /// `1. ` ou `1) `.
    Numero(Vec<Segment>),
    /// Lignes `| a | b |` (la ligne de règle `|---|` est ignorée).
    Tableau(Vec<Vec<String>>),
    /// `---` : saut de page (docx/pdf), nouvelle diapo (pptx).
    Separateur,
}

/// Octets du document pour cette extension (sans point, insensible à la
/// casse), ou `None` si ce n'est pas un format généré ici — l'appelant
/// écrit alors le markdown tel quel (md, txt, html…).
pub fn generer(extension: &str, markdown: &str) -> Option<Vec<u8>> {
    let blocs = analyser(markdown);
    match extension.to_ascii_lowercase().as_str() {
        "docx" => Some(docx::generer(&blocs)),
        "xlsx" => Some(xlsx::generer(markdown, &blocs)),
        "pptx" => Some(pptx::generer(&blocs)),
        "pdf" => Some(pdf::generer(&blocs)),
        _ => None,
    }
}

/// Analyse le markdown de Waly en blocs.
pub fn analyser(md: &str) -> Vec<Bloc> {
    let mut blocs = Vec::new();
    let mut tableau: Vec<Vec<String>> = Vec::new();
    let mut code = false;
    for brute in md.lines() {
        let l = brute.trim_end();
        let t = l.trim_start();
        if !code && t.starts_with('|') {
            let cellules: Vec<String> =
                t.trim_matches('|').split('|').map(|c| plat(c.trim())).collect();
            let regle = cellules
                .iter()
                .all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':'));
            if !regle {
                tableau.push(cellules);
            }
            continue;
        }
        if !tableau.is_empty() {
            blocs.push(Bloc::Tableau(std::mem::take(&mut tableau)));
        }
        if t.starts_with("```") {
            code = !code;
            continue;
        }
        if code {
            // Code : gardé tel quel, ligne par ligne.
            if !t.is_empty() {
                blocs.push(Bloc::Paragraphe(vec![Segment {
                    texte: l.to_string(),
                    gras: false,
                    italique: false,
                }]));
            }
            continue;
        }
        if t.is_empty() {
            continue;
        }
        if t == "---" || t == "***" || t == "___" {
            blocs.push(Bloc::Separateur);
            continue;
        }
        let dieses = t.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&dieses) && t[dieses..].starts_with(' ') {
            blocs.push(Bloc::Titre { niveau: dieses.min(3) as u8, texte: plat(t[dieses..].trim()) });
            continue;
        }
        if let Some(r) =
            t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("• "))
        {
            blocs.push(Bloc::Puce(segments(r.trim())));
            continue;
        }
        if let Some(r) = numero(t) {
            blocs.push(Bloc::Numero(segments(r)));
            continue;
        }
        blocs.push(Bloc::Paragraphe(segments(t)));
    }
    if !tableau.is_empty() {
        blocs.push(Bloc::Tableau(tableau));
    }
    blocs
}

/// `12. texte` / `3) texte` → `texte`.
fn numero(t: &str) -> Option<&str> {
    let n = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if n == 0 || n > 3 {
        return None;
    }
    let r = &t[n..];
    r.strip_prefix(". ").or_else(|| r.strip_prefix(") ")).map(str::trim)
}

fn pousser(out: &mut Vec<Segment>, cur: &mut String, gras: bool, italique: bool) {
    if !cur.is_empty() {
        out.push(Segment { texte: std::mem::take(cur), gras, italique });
    }
}

/// Mise en forme en ligne : `**gras**`, `*italique*` ; les accents graves
/// de code sont retirés (le texte reste).
pub fn segments(s: &str) -> Vec<Segment> {
    let cs: Vec<char> = s.chars().collect();
    let (mut out, mut cur) = (Vec::new(), String::new());
    let (mut gras, mut italique) = (false, false);
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '*' if cs.get(i + 1) == Some(&'*') => {
                pousser(&mut out, &mut cur, gras, italique);
                gras = !gras;
                i += 2;
            }
            '*' => {
                pousser(&mut out, &mut cur, gras, italique);
                italique = !italique;
                i += 1;
            }
            '`' => i += 1,
            c => {
                cur.push(c);
                i += 1;
            }
        }
    }
    pousser(&mut out, &mut cur, gras, italique);
    out
}

/// Texte sans marqueurs de mise en forme.
pub fn plat(s: &str) -> String {
    segments(s).into_iter().map(|g| g.texte).collect()
}

/// Échappement XML (et retrait des caractères de contrôle, interdits en
/// XML 1.0 — ils rendraient le fichier illisible pour Office).
pub(crate) fn xml(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => o.push(c),
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(t: &str, gras: bool, italique: bool) -> Segment {
        Segment { texte: t.into(), gras, italique }
    }

    #[test]
    fn blocs_du_markdown_de_waly() {
        let md = "# Plan de la semaine\n\n## Lundi\n- Réunion **client**\n1. Appeler *Paul*\n\
                  Texte simple.\n\n| Jour | Tâche |\n|---|:---:|\n| Lundi | Budget |\n\n---\n### Fin";
        let b = analyser(md);
        assert_eq!(b[0], Bloc::Titre { niveau: 1, texte: "Plan de la semaine".into() });
        assert_eq!(b[1], Bloc::Titre { niveau: 2, texte: "Lundi".into() });
        assert_eq!(b[2], Bloc::Puce(vec![seg("Réunion ", false, false), seg("client", true, false)]));
        assert_eq!(b[3], Bloc::Numero(vec![seg("Appeler ", false, false), seg("Paul", false, true)]));
        assert_eq!(b[4], Bloc::Paragraphe(vec![seg("Texte simple.", false, false)]));
        assert_eq!(
            b[5],
            Bloc::Tableau(vec![
                vec!["Jour".into(), "Tâche".into()],
                vec!["Lundi".into(), "Budget".into()],
            ])
        );
        assert_eq!(b[6], Bloc::Separateur);
        assert_eq!(b[7], Bloc::Titre { niveau: 3, texte: "Fin".into() });
        assert_eq!(b.len(), 8);
    }

    #[test]
    fn code_garde_tel_quel_et_diese_sans_espace() {
        let b = analyser("```\n# pas un titre\n```\n#hashtag");
        assert_eq!(b[0], Bloc::Paragraphe(vec![seg("# pas un titre", false, false)]));
        assert_eq!(b[1], Bloc::Paragraphe(vec![seg("#hashtag", false, false)]));
    }

    /// Exemple riche, proche de ce que Waly produit.
    pub(crate) const EXEMPLE: &str = "# Plan de la semaine\n\n\
Semaine du **14 septembre** : priorités *client* & équipe.\n\n\
## Lundi\n- Réunion d'équipe à 9 h\n- Point **budget** avec Élodie\n\n\
## Objectifs\n1. Finaliser le devis\n2. Envoyer la facture\n\n\
| Jour | Tâche | Durée |\n|---|---|---|\n| Lundi | Devis | 2 h |\n| Mardi | Facture | 1 h |\n\n\
---\n## Annexe\nNotes : caractères spéciaux < > & \" ' conservés.";

    #[test]
    fn formats_reconnus() {
        assert!(generer("DOCX", "# x").is_some());
        assert!(generer("md", "# x").is_none());
    }

    /// Écrit de VRAIS fichiers à ouvrir dans Office :
    /// `WALY_DOC_OUT=<dossier> cargo test -p waly-core ecrire_exemples -- --ignored`
    #[test]
    #[ignore = "ecrit des exemples reels (WALY_DOC_OUT) pour verification dans Office"]
    fn ecrire_exemples() {
        let dossier = std::env::var("WALY_DOC_OUT").expect("WALY_DOC_OUT");
        for ext in ["docx", "xlsx", "pptx", "pdf"] {
            let octets = generer(ext, EXEMPLE).expect("format genere");
            std::fs::write(format!("{dossier}/exemple.{ext}"), octets).unwrap();
        }
    }

    #[test]
    fn echappement_xml_et_controles() {
        assert_eq!(xml("a<b>&\"c'\u{1}"), "a&lt;b&gt;&amp;&quot;c&apos;");
    }
}
