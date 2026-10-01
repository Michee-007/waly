//! Excel (.xlsx) : SpreadsheetML minimal. Les tableaux markdown deviennent
//! des lignes (1ʳᵉ ligne de chaque tableau en gras, figée si le classeur
//! commence par un tableau) ; les nombres sont de VRAIS nombres (virgule
//! décimale française acceptée) pour que les formules marchent ; titres et
//! paragraphes restent en lignes d'une cellule. Sans tableau, un contenu
//! CSV (`;` ou `,`, guillemets gérés) est découpé en colonnes. Chaînes en
//! ligne (`inlineStr`) : pas de table partagée à maintenir.

use super::{plat, xml, Bloc, Segment};
use crate::zipw::Zip;

const ENTETE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

const TYPES: &str = "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
<Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>\
</Types>";

const RELS: &str = "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
</Relationships>";

const WB_RELS: &str = "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
</Relationships>";

/// Style 0 = normal, 1 = gras. Deux remplissages obligatoires (none, gray125).
const STYLES: &str = "<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
<fonts count=\"2\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font><font><b/><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>\
<fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills>\
<borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>\
<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>\
<cellXfs count=\"2\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>\
<xf numFmtId=\"0\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFont=\"1\"/></cellXfs>\
<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>\
</styleSheet>";

/// Une ligne du classeur : cellules + gras.
struct Ligne {
    cellules: Vec<String>,
    gras: bool,
}

/// 0 → A, 25 → Z, 26 → AA.
pub(crate) fn colonne(mut i: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (i % 26) as u8);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

/// Nombre au sens d'Excel : `-12`, `3.5`, `3,5` (virgule française).
fn nombre(s: &str) -> Option<f64> {
    let t = s.trim();
    let chiffres = t.strip_prefix('-').unwrap_or(t);
    let mut sep = 0;
    let valide = !chiffres.is_empty()
        && chiffres.chars().all(|c| {
            if c == '.' || c == ',' {
                sep += 1;
                true
            } else {
                c.is_ascii_digit()
            }
        })
        && sep <= 1
        && !chiffres.starts_with(['.', ','])
        && !chiffres.ends_with(['.', ',']);
    if !valide {
        return None;
    }
    t.replace(',', ".").parse().ok()
}

/// Découpe une ligne CSV (guillemets doublés gérés).
fn csv(ligne: &str, sep: char) -> Vec<String> {
    let (mut out, mut cur, mut guillemets) = (Vec::new(), String::new(), false);
    let mut cs = ligne.chars().peekable();
    while let Some(c) = cs.next() {
        match c {
            '"' if guillemets && cs.peek() == Some(&'"') => {
                cur.push('"');
                cs.next();
            }
            '"' => guillemets = !guillemets,
            c if c == sep && !guillemets => out.push(std::mem::take(&mut cur).trim().to_string()),
            c => cur.push(c),
        }
    }
    out.push(cur.trim().to_string());
    out
}

fn texte(segs: &[Segment]) -> String {
    segs.iter().map(|g| g.texte.as_str()).collect()
}

fn lignes(markdown: &str, blocs: &[Bloc]) -> (Vec<Ligne>, bool) {
    let a_tableau = blocs.iter().any(|b| matches!(b, Bloc::Tableau(_)));
    let brutes: Vec<&str> = markdown.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let sep = if brutes.first().is_some_and(|l| l.contains(';')) { ';' } else { ',' };
    if !a_tableau && brutes.len() >= 2 && brutes.iter().all(|l| l.contains(sep)) {
        // Contenu CSV : en-tête en gras, figé.
        let v = brutes
            .iter()
            .enumerate()
            .map(|(i, l)| Ligne { cellules: csv(l, sep), gras: i == 0 })
            .collect();
        return (v, true);
    }
    let figer = matches!(blocs.first(), Some(Bloc::Tableau(_)));
    let mut v = Vec::new();
    for b in blocs {
        match b {
            Bloc::Titre { texte: t, .. } => v.push(Ligne { cellules: vec![t.clone()], gras: true }),
            Bloc::Paragraphe(s) | Bloc::Puce(s) | Bloc::Numero(s) => {
                v.push(Ligne { cellules: vec![texte(s)], gras: false })
            }
            Bloc::Tableau(t) => {
                for (i, l) in t.iter().enumerate() {
                    v.push(Ligne { cellules: l.iter().map(|c| plat(c)).collect(), gras: i == 0 });
                }
            }
            Bloc::Separateur => v.push(Ligne { cellules: vec![], gras: false }),
        }
    }
    (v, figer)
}

fn feuille(lignes: &[Ligne], figer: bool) -> String {
    let cols = lignes.iter().map(|l| l.cellules.len()).max().unwrap_or(0);
    let mut s = format!("{ENTETE}<worksheet xmlns=\"{MAIN}\">");
    if figer && lignes.len() > 1 {
        s.push_str(
            "<sheetViews><sheetView workbookViewId=\"0\"><pane ySplit=\"1\" topLeftCell=\"A2\" \
             activePane=\"bottomLeft\" state=\"frozen\"/></sheetView></sheetViews>",
        );
    }
    if cols > 0 {
        s.push_str("<cols>");
        for c in 0..cols {
            let max = lignes
                .iter()
                .filter_map(|l| l.cellules.get(c))
                .map(|x| x.chars().count())
                .max()
                .unwrap_or(0);
            let largeur = (max + 2).clamp(8, 60);
            s.push_str(&format!("<col min=\"{0}\" max=\"{0}\" width=\"{largeur}\" customWidth=\"1\"/>", c + 1));
        }
        s.push_str("</cols>");
    }
    s.push_str("<sheetData>");
    for (r, l) in lignes.iter().enumerate() {
        s.push_str(&format!("<row r=\"{}\">", r + 1));
        for (c, cellule) in l.cellules.iter().enumerate() {
            if cellule.is_empty() {
                continue;
            }
            let reference = format!("{}{}", colonne(c), r + 1);
            let style = if l.gras { " s=\"1\"" } else { "" };
            match nombre(cellule) {
                Some(n) => s.push_str(&format!("<c r=\"{reference}\"{style}><v>{n}</v></c>")),
                None => s.push_str(&format!(
                    "<c r=\"{reference}\"{style} t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
                    xml(cellule)
                )),
            }
        }
        s.push_str("</row>");
    }
    s.push_str("</sheetData></worksheet>");
    s
}

/// Rend le markdown (ou CSV) en fichier .xlsx.
pub fn generer(markdown: &str, blocs: &[Bloc]) -> Vec<u8> {
    let (lignes, figer) = lignes(markdown, blocs);
    let mut z = Zip::new();
    z.ajouter("[Content_Types].xml", format!("{ENTETE}{TYPES}").as_bytes());
    z.ajouter("_rels/.rels", format!("{ENTETE}{RELS}").as_bytes());
    z.ajouter(
        "xl/workbook.xml",
        format!(
            "{ENTETE}<workbook xmlns=\"{MAIN}\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
             <sheets><sheet name=\"Feuille1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"
        )
        .as_bytes(),
    );
    z.ajouter("xl/_rels/workbook.xml.rels", format!("{ENTETE}{WB_RELS}").as_bytes());
    z.ajouter("xl/styles.xml", format!("{ENTETE}{STYLES}").as_bytes());
    z.ajouter("xl/worksheets/sheet1.xml", feuille(&lignes, figer).as_bytes());
    z.finir()
}

#[cfg(test)]
mod tests {
    use super::super::analyser;
    use super::*;

    fn feuille_de(md: &str) -> String {
        let z = generer(md, &analyser(md));
        let e = crate::zipw::tests::lire(&z);
        let (_, c) = e.iter().find(|(n, _)| n == "xl/worksheets/sheet1.xml").unwrap();
        String::from_utf8(c.clone()).unwrap()
    }

    #[test]
    fn colonnes_et_nombres() {
        assert_eq!(colonne(0), "A");
        assert_eq!(colonne(25), "Z");
        assert_eq!(colonne(26), "AA");
        assert_eq!(colonne(701), "ZZ");
        assert_eq!(nombre("12,5"), Some(12.5));
        assert_eq!(nombre("-3"), Some(-3.0));
        assert_eq!(nombre("2 h"), None);
        assert_eq!(nombre("1.2.3"), None);
        assert_eq!(nombre(","), None);
    }

    #[test]
    fn tableau_markdown_entete_gras_fige_et_nombres() {
        let f = feuille_de("| Poste | Montant |\n|---|---|\n| Loyer & charges | 850,50 |");
        assert!(f.contains("state=\"frozen\""));
        assert!(f.contains("<c r=\"A1\" s=\"1\" t=\"inlineStr\">"), "{f}");
        assert!(f.contains("Loyer &amp; charges"));
        assert!(f.contains("<c r=\"B2\"><v>850.5</v></c>"), "{f}");
    }

    #[test]
    fn csv_point_virgule_et_guillemets() {
        let f = feuille_de("Nom;Ville;Âge\n\"Dupont; Jean\";Lyon;42");
        assert!(f.contains("Dupont; Jean"), "{f}");
        assert!(f.contains("<c r=\"C2\"><v>42</v></c>"), "{f}");
        assert!(f.contains("<c r=\"A1\" s=\"1\""));
    }

    #[test]
    fn titres_et_paragraphes_gardes_sans_figer() {
        let f = feuille_de("# Budget\nAvant le tableau.\n| A | B |\n|---|---|\n| 1 | 2 |");
        assert!(!f.contains("frozen"));
        assert!(f.contains("<c r=\"A1\" s=\"1\" t=\"inlineStr\"><is><t xml:space=\"preserve\">Budget"));
        assert!(f.contains("<c r=\"A4\"><v>1</v></c>"), "{f}");
    }
}
