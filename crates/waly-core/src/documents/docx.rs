//! Word (.docx) : WordprocessingML minimal mais complet — styles intégrés
//! (`heading 1..3` → volet de navigation), puces et listes numérotées
//! (chaque liste repart à 1), tableaux quadrillés (1ʳᵉ ligne en gras),
//! sauts de page, page A4. L'ORDRE des éléments suit le schéma OOXML :
//! Word refuse un fichier qui ne le respecte pas.

use super::{xml, Bloc, Segment};
use crate::zipw::Zip;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const ENTETE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

const TYPES: &str = "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
<Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/>\
</Types>";

const RELS: &str = "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
</Relationships>";

const DOC_RELS: &str = "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering\" Target=\"numbering.xml\"/>\
</Relationships>";

fn styles() -> String {
    let titre = |n: u8, taille: u32| {
        format!(
            "<w:style w:type=\"paragraph\" w:styleId=\"Heading{n}\"><w:name w:val=\"heading {n}\"/>\
             <w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/>\
             <w:pPr><w:keepNext/><w:spacing w:before=\"{}\" w:after=\"120\"/><w:outlineLvl w:val=\"{}\"/></w:pPr>\
             <w:rPr><w:b/><w:color w:val=\"1F3864\"/><w:sz w:val=\"{taille}\"/><w:szCs w:val=\"{taille}\"/></w:rPr></w:style>",
            if n == 1 { 360 } else { 240 },
            n - 1
        )
    };
    let bord = |cote: &str| format!("<w:{cote} w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>");
    format!(
        "{ENTETE}<w:styles xmlns:w=\"{W}\">\
         <w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:eastAsia=\"Calibri\" w:cs=\"Calibri\"/>\
         <w:sz w:val=\"22\"/><w:szCs w:val=\"22\"/><w:lang w:val=\"fr-FR\"/></w:rPr></w:rPrDefault>\
         <w:pPrDefault><w:pPr><w:spacing w:after=\"160\" w:line=\"276\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>\
         <w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>\
         {}{}{}\
         <w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:basedOn w:val=\"Normal\"/>\
         <w:qFormat/><w:pPr><w:spacing w:after=\"60\"/><w:ind w:left=\"720\"/></w:pPr></w:style>\
         <w:style w:type=\"table\" w:styleId=\"TableGrid\"><w:name w:val=\"Table Grid\"/><w:tblPr><w:tblBorders>{}{}{}{}{}{}</w:tblBorders>\
         <w:tblCellMar><w:left w:w=\"108\" w:type=\"dxa\"/><w:right w:w=\"108\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr></w:style>\
         </w:styles>",
        titre(1, 36),
        titre(2, 28),
        titre(3, 24),
        bord("top"),
        bord("left"),
        bord("bottom"),
        bord("right"),
        bord("insideH"),
        bord("insideV"),
    )
}

/// numId 1 = puces ; numId 2.. = une liste numérotée CHACUNE (repart à 1).
fn numbering(listes_numerotees: usize) -> String {
    let abs = |id: u8, fmt: &str, texte: &str| {
        format!(
            "<w:abstractNum w:abstractNumId=\"{id}\"><w:multiLevelType w:val=\"hybridMultilevel\"/>\
             <w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{fmt}\"/><w:lvlText w:val=\"{texte}\"/>\
             <w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"720\" w:hanging=\"360\"/></w:pPr></w:lvl></w:abstractNum>"
        )
    };
    let mut s = format!(
        "{ENTETE}<w:numbering xmlns:w=\"{W}\">{}{}<w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num>",
        abs(0, "bullet", "•"),
        abs(1, "decimal", "%1.")
    );
    for k in 0..listes_numerotees {
        s.push_str(&format!(
            "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"1\"/>\
             <w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"1\"/></w:lvlOverride></w:num>",
            2 + k
        ));
    }
    s.push_str("</w:numbering>");
    s
}

fn runs(segs: &[Segment], forcer_gras: bool) -> String {
    let mut s = String::new();
    for g in segs {
        let mut rpr = String::new();
        if g.gras || forcer_gras {
            rpr.push_str("<w:b/>");
        }
        if g.italique {
            rpr.push_str("<w:i/>");
        }
        if !rpr.is_empty() {
            rpr = format!("<w:rPr>{rpr}</w:rPr>");
        }
        s.push_str(&format!("<w:r>{rpr}<w:t xml:space=\"preserve\">{}</w:t></w:r>", xml(&g.texte)));
    }
    s
}

fn texte(t: &str) -> Vec<Segment> {
    vec![Segment { texte: t.to_string(), gras: false, italique: false }]
}

fn tableau(lignes: &[Vec<String>]) -> String {
    let cols = lignes.iter().map(Vec::len).max().unwrap_or(1).max(1);
    // Largeur utile A4 (marges 2,5 cm) ≈ 9 072 twips, répartie également.
    let largeur = 9072 / cols;
    let mut s = String::from(
        "<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"0\" w:type=\"auto\"/></w:tblPr><w:tblGrid>",
    );
    for _ in 0..cols {
        s.push_str(&format!("<w:gridCol w:w=\"{largeur}\"/>"));
    }
    s.push_str("</w:tblGrid>");
    for (i, ligne) in lignes.iter().enumerate() {
        s.push_str("<w:tr>");
        for c in 0..cols {
            let cellule = ligne.get(c).map(String::as_str).unwrap_or("");
            s.push_str(&format!(
                "<w:tc><w:tcPr><w:tcW w:w=\"{largeur}\" w:type=\"dxa\"/></w:tcPr><w:p>{}</w:p></w:tc>",
                runs(&texte(cellule), i == 0)
            ));
        }
        s.push_str("</w:tr>");
    }
    s.push_str("</w:tbl>");
    s
}

fn document(blocs: &[Bloc]) -> (String, usize) {
    let mut corps = String::new();
    let mut listes = 0usize;
    let mut dans_liste_num = false;
    for b in blocs {
        if !matches!(b, Bloc::Numero(_)) {
            dans_liste_num = false;
        }
        match b {
            Bloc::Titre { niveau, texte: t } => corps.push_str(&format!(
                "<w:p><w:pPr><w:pStyle w:val=\"Heading{niveau}\"/></w:pPr>{}</w:p>",
                runs(&texte(t), false)
            )),
            Bloc::Paragraphe(segs) => corps.push_str(&format!("<w:p>{}</w:p>", runs(segs, false))),
            Bloc::Puce(segs) => corps.push_str(&format!(
                "<w:p><w:pPr><w:pStyle w:val=\"ListParagraph\"/><w:numPr><w:ilvl w:val=\"0\"/>\
                 <w:numId w:val=\"1\"/></w:numPr></w:pPr>{}</w:p>",
                runs(segs, false)
            )),
            Bloc::Numero(segs) => {
                if !dans_liste_num {
                    listes += 1;
                    dans_liste_num = true;
                }
                corps.push_str(&format!(
                    "<w:p><w:pPr><w:pStyle w:val=\"ListParagraph\"/><w:numPr><w:ilvl w:val=\"0\"/>\
                     <w:numId w:val=\"{}\"/></w:numPr></w:pPr>{}</w:p>",
                    1 + listes,
                    runs(segs, false)
                ));
            }
            Bloc::Tableau(lignes) => corps.push_str(&tableau(lignes)),
            Bloc::Separateur => corps.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>"),
        }
    }
    // Word veut un paragraphe après un tableau final (et un corps non vide).
    if blocs.is_empty() || matches!(blocs.last(), Some(Bloc::Tableau(_))) {
        corps.push_str("<w:p/>");
    }
    let doc = format!(
        "{ENTETE}<w:document xmlns:w=\"{W}\"><w:body>{corps}\
         <w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/>\
         <w:pgMar w:top=\"1417\" w:right=\"1417\" w:bottom=\"1417\" w:left=\"1417\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/>\
         </w:sectPr></w:body></w:document>"
    );
    (doc, listes)
}

/// Rend les blocs en fichier .docx.
pub fn generer(blocs: &[Bloc]) -> Vec<u8> {
    let (doc, listes) = document(blocs);
    let mut z = Zip::new();
    z.ajouter("[Content_Types].xml", format!("{ENTETE}{TYPES}").as_bytes());
    z.ajouter("_rels/.rels", format!("{ENTETE}{RELS}").as_bytes());
    z.ajouter("word/_rels/document.xml.rels", format!("{ENTETE}{DOC_RELS}").as_bytes());
    z.ajouter("word/document.xml", doc.as_bytes());
    z.ajouter("word/styles.xml", styles().as_bytes());
    z.ajouter("word/numbering.xml", numbering(listes).as_bytes());
    z.finir()
}

#[cfg(test)]
mod tests {
    use super::super::analyser;
    use super::*;

    fn partie(zip: &[u8], nom: &str) -> String {
        let e = crate::zipw::tests::lire(zip);
        let (_, c) = e.iter().find(|(n, _)| n == nom).unwrap_or_else(|| panic!("{nom} absent"));
        String::from_utf8(c.clone()).unwrap()
    }

    #[test]
    fn paquet_complet_et_contenu() {
        let md = "# Plan & budget\n- Réunion **client**\n1. Un\n2. Deux\nPause\n1. Encore\n\
                  ---\n| A | B |\n|---|---|\n| x |";
        let z = generer(&analyser(md));
        let noms: Vec<String> = crate::zipw::tests::lire(&z).into_iter().map(|(n, _)| n).collect();
        for n in ["[Content_Types].xml", "_rels/.rels", "word/_rels/document.xml.rels",
                  "word/document.xml", "word/styles.xml", "word/numbering.xml"] {
            assert!(noms.iter().any(|x| x == n), "{n} absent");
        }
        let doc = partie(&z, "word/document.xml");
        assert!(doc.contains("Heading1"));
        assert!(doc.contains("Plan &amp; budget"), "texte echappe");
        assert!(doc.contains("<w:b/>"));
        assert!(doc.contains("w:type=\"page\""));
        // Deux listes numérotées distinctes → numId 2 puis 3 (chacune repart à 1).
        assert!(doc.contains("<w:numId w:val=\"2\"/>") && doc.contains("<w:numId w:val=\"3\"/>"));
        assert!(partie(&z, "word/numbering.xml").contains("w:numId=\"3\""));
        // Ligne de tableau incomplète complétée ; paragraphe après tableau final.
        assert_eq!(doc.matches("<w:tc>").count(), 4);
        assert!(doc.contains("</w:tbl><w:p/>"));
    }

    #[test]
    fn document_vide_reste_valide() {
        let doc = partie(&generer(&[]), "word/document.xml");
        assert!(doc.contains("<w:body><w:p/><w:sectPr>"));
    }
}
