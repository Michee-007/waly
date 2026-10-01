//! PowerPoint (.pptx) : PresentationML minimal mais COMPLET — présentation,
//! masque, disposition vierge, thème complet (PowerPoint refuse un paquet
//! incomplet), une diapo 16:9 par section. Découpage : `---` ou un titre
//! quand la diapo a déjà du contenu → nouvelle diapo ; le 1ᵉʳ titre d'une
//! diapo en est le titre ; le reste va au corps (vraies puces, vraies
//! listes numérotées, lignes de tableau « A | B »). La taille du corps
//! s'adapte au nombre de lignes (pas d'autofit calculé hors PowerPoint).

use super::{xml, Bloc, Segment};
use crate::zipw::Zip;

const ENTETE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const NS: &str = "xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const CT: &str = "application/vnd.openxmlformats-officedocument";

const ARBRE_VIDE: &str = "<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/>\
<a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>";

const THEME: &str = "<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Waly\"><a:themeElements>\
<a:clrScheme name=\"Waly\"><a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1>\
<a:lt1><a:sysClr val=\"window\" lastClr=\"FFFFFF\"/></a:lt1><a:dk2><a:srgbClr val=\"1F3864\"/></a:dk2>\
<a:lt2><a:srgbClr val=\"E7E6E6\"/></a:lt2><a:accent1><a:srgbClr val=\"4472C4\"/></a:accent1>\
<a:accent2><a:srgbClr val=\"ED7D31\"/></a:accent2><a:accent3><a:srgbClr val=\"A5A5A5\"/></a:accent3>\
<a:accent4><a:srgbClr val=\"FFC000\"/></a:accent4><a:accent5><a:srgbClr val=\"5B9BD5\"/></a:accent5>\
<a:accent6><a:srgbClr val=\"70AD47\"/></a:accent6><a:hlink><a:srgbClr val=\"0563C1\"/></a:hlink>\
<a:folHlink><a:srgbClr val=\"954F72\"/></a:folHlink></a:clrScheme>\
<a:fontScheme name=\"Waly\"><a:majorFont><a:latin typeface=\"Calibri Light\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont>\
<a:minorFont><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme>\
<a:fmtScheme name=\"Waly\"><a:fillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>\
<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:fillStyleLst>\
<a:lnStyleLst><a:ln w=\"6350\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln>\
<a:ln w=\"12700\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln>\
<a:ln w=\"19050\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln></a:lnStyleLst>\
<a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle>\
<a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>\
<a:bgFillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>\
<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme>\
</a:themeElements></a:theme>";

/// Une ligne du corps d'une diapo.
enum Ligne {
    Texte(Vec<Segment>),
    Puce(Vec<Segment>),
    Numero(Vec<Segment>),
}

#[derive(Default)]
struct Diapo {
    titre: Option<String>,
    corps: Vec<Ligne>,
}

impl Diapo {
    fn vide(&self) -> bool {
        self.titre.is_none() && self.corps.is_empty()
    }
}

fn decouper(blocs: &[Bloc]) -> Vec<Diapo> {
    let mut diapos = vec![Diapo::default()];
    for b in blocs {
        let d = diapos.last_mut().unwrap();
        match b {
            Bloc::Separateur => {
                if !d.vide() {
                    diapos.push(Diapo::default());
                }
            }
            Bloc::Titre { texte, .. } => {
                if d.vide() {
                    d.titre = Some(texte.clone());
                } else {
                    diapos.push(Diapo { titre: Some(texte.clone()), corps: vec![] });
                }
            }
            Bloc::Paragraphe(s) => d.corps.push(Ligne::Texte(s.clone())),
            Bloc::Puce(s) => d.corps.push(Ligne::Puce(s.clone())),
            Bloc::Numero(s) => d.corps.push(Ligne::Numero(s.clone())),
            Bloc::Tableau(t) => {
                for (i, l) in t.iter().enumerate() {
                    let texte = l.join("  |  ");
                    d.corps.push(Ligne::Texte(vec![Segment { texte, gras: i == 0, italique: false }]));
                }
            }
        }
    }
    if diapos.len() > 1 && diapos.last().is_some_and(Diapo::vide) {
        diapos.pop();
    }
    diapos
}

fn runs(segs: &[Segment], taille: u32, gras_force: bool) -> String {
    let mut s = String::new();
    for g in segs {
        let b = if g.gras || gras_force { " b=\"1\"" } else { "" };
        let i = if g.italique { " i=\"1\"" } else { "" };
        s.push_str(&format!(
            "<a:r><a:rPr lang=\"fr-FR\" sz=\"{taille}\"{b}{i} dirty=\"0\"/><a:t>{}</a:t></a:r>",
            xml(&g.texte)
        ));
    }
    s
}

fn forme(id: u32, nom: &str, (x, y, cx, cy): (u64, u64, u64, u64), paragraphes: &str) -> String {
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{nom}\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr>\
         <p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
         <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr>\
         <p:txBody><a:bodyPr wrap=\"square\" rtlCol=\"0\"/><a:lstStyle/>{paragraphes}</p:txBody></p:sp>"
    )
}

fn diapo_xml(d: &Diapo) -> String {
    let mut formes = String::new();
    if let Some(t) = &d.titre {
        let segs = vec![Segment { texte: t.clone(), gras: false, italique: false }];
        let p = format!("<a:p>{}</a:p>", runs(&segs, 3600, true));
        formes.push_str(&forme(2, "Titre", (457_200, 304_800, 11_277_600, 1_143_000), &p));
    }
    if !d.corps.is_empty() {
        let taille = match d.corps.len() {
            0..=6 => 2400,
            7..=10 => 2000,
            _ => 1600,
        };
        let mut ps = String::new();
        for l in &d.corps {
            let (ppr, segs) = match l {
                Ligne::Texte(s) => ("<a:pPr marL=\"0\" indent=\"0\"><a:buNone/></a:pPr>".to_string(), s),
                Ligne::Puce(s) => (
                    "<a:pPr marL=\"342900\" indent=\"-342900\"><a:buFont typeface=\"Arial\"/>\
                     <a:buChar char=\"•\"/></a:pPr>"
                        .to_string(),
                    s,
                ),
                Ligne::Numero(s) => (
                    "<a:pPr marL=\"457200\" indent=\"-457200\"><a:buAutoNum type=\"arabicPeriod\"/></a:pPr>"
                        .to_string(),
                    s,
                ),
            };
            ps.push_str(&format!("<a:p>{ppr}{}</a:p>", runs(segs, taille, false)));
        }
        let y = if d.titre.is_some() { 1_600_200 } else { 457_200 };
        formes.push_str(&forme(3, "Contenu", (457_200, y, 11_277_600, 6_400_800 - y), &ps));
    }
    format!(
        "{ENTETE}<p:sld {NS}><p:cSld><p:spTree>{ARBRE_VIDE}{formes}</p:spTree></p:cSld>\
         <p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
    )
}

fn rels(liens: &[(&str, &str, &str)]) -> String {
    let mut s = format!(
        "{ENTETE}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">"
    );
    for (id, typ, cible) in liens {
        s.push_str(&format!("<Relationship Id=\"{id}\" Type=\"{REL}/{typ}\" Target=\"{cible}\"/>"));
    }
    s.push_str("</Relationships>");
    s
}

/// Rend les blocs en fichier .pptx.
pub fn generer(blocs: &[Bloc]) -> Vec<u8> {
    let diapos = decouper(blocs);
    let n = diapos.len();
    let mut z = Zip::new();

    let mut types = format!(
        "{ENTETE}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Override PartName=\"/ppt/presentation.xml\" ContentType=\"{CT}.presentationml.presentation.main+xml\"/>\
         <Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"{CT}.presentationml.slideMaster+xml\"/>\
         <Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"{CT}.presentationml.slideLayout+xml\"/>\
         <Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"{CT}.theme+xml\"/>"
    );
    for i in 1..=n {
        types.push_str(&format!(
            "<Override PartName=\"/ppt/slides/slide{i}.xml\" ContentType=\"{CT}.presentationml.slide+xml\"/>"
        ));
    }
    types.push_str("</Types>");
    z.ajouter("[Content_Types].xml", types.as_bytes());
    z.ajouter("_rels/.rels", rels(&[("rId1", "officeDocument", "ppt/presentation.xml")]).as_bytes());

    // Présentation : masque (rId1), diapos (rId2..), thème (dernier).
    let mut ids = String::new();
    let mut liens_p: Vec<(String, &str, String)> =
        vec![("rId1".into(), "slideMaster", "slideMasters/slideMaster1.xml".into())];
    for i in 1..=n {
        ids.push_str(&format!("<p:sldId id=\"{}\" r:id=\"rId{}\"/>", 255 + i, i + 1));
        liens_p.push((format!("rId{}", i + 1), "slide", format!("slides/slide{i}.xml")));
    }
    liens_p.push((format!("rId{}", n + 2), "theme", "theme/theme1.xml".into()));
    z.ajouter(
        "ppt/presentation.xml",
        format!(
            "{ENTETE}<p:presentation {NS} saveSubsetFonts=\"1\">\
             <p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst>\
             <p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx=\"12192000\" cy=\"6858000\"/>\
             <p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"
        )
        .as_bytes(),
    );
    let liens: Vec<(&str, &str, &str)> =
        liens_p.iter().map(|(a, b, c)| (a.as_str(), *b, c.as_str())).collect();
    z.ajouter("ppt/_rels/presentation.xml.rels", rels(&liens).as_bytes());

    z.ajouter(
        "ppt/slideMasters/slideMaster1.xml",
        format!(
            "{ENTETE}<p:sldMaster {NS}><p:cSld><p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg>\
             <p:spTree>{ARBRE_VIDE}</p:spTree></p:cSld>\
             <p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" \
             accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>\
             <p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst>\
             <p:txStyles><p:titleStyle><a:lvl1pPr><a:defRPr sz=\"3600\"/></a:lvl1pPr></p:titleStyle>\
             <p:bodyStyle><a:lvl1pPr><a:defRPr sz=\"2400\"/></a:lvl1pPr></p:bodyStyle>\
             <p:otherStyle><a:lvl1pPr><a:defRPr/></a:lvl1pPr></p:otherStyle></p:txStyles></p:sldMaster>"
        )
        .as_bytes(),
    );
    z.ajouter(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"), ("rId2", "theme", "../theme/theme1.xml")])
            .as_bytes(),
    );
    z.ajouter(
        "ppt/slideLayouts/slideLayout1.xml",
        format!(
            "{ENTETE}<p:sldLayout {NS} type=\"blank\" preserve=\"1\"><p:cSld name=\"Vide\"><p:spTree>{ARBRE_VIDE}</p:spTree></p:cSld>\
             <p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
        )
        .as_bytes(),
    );
    z.ajouter(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")]).as_bytes(),
    );
    z.ajouter("ppt/theme/theme1.xml", format!("{ENTETE}{THEME}").as_bytes());
    for (i, d) in diapos.iter().enumerate() {
        z.ajouter(&format!("ppt/slides/slide{}.xml", i + 1), diapo_xml(d).as_bytes());
        z.ajouter(
            &format!("ppt/slides/_rels/slide{}.xml.rels", i + 1),
            rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml")]).as_bytes(),
        );
    }
    z.finir()
}

#[cfg(test)]
mod tests {
    use super::super::analyser;
    use super::*;

    #[test]
    fn decoupage_en_diapos() {
        let d = decouper(&analyser("# Titre\nIntro\n## Partie 1\n- a\n---\nSans titre\n---\n"));
        assert_eq!(d.len(), 3, "titre avec contenu = nouvelle diapo ; --- final ignore");
        assert_eq!(d[0].titre.as_deref(), Some("Titre"));
        assert_eq!(d[1].titre.as_deref(), Some("Partie 1"));
        assert!(d[2].titre.is_none() && d[2].corps.len() == 1);
    }

    #[test]
    fn paquet_complet_puces_et_numeros() {
        let z = generer(&analyser("# Plan & suite\n- Réunion\n1. Un\n---\n# Deux"));
        let e = crate::zipw::tests::lire(&z);
        let lire = |nom: &str| {
            String::from_utf8(e.iter().find(|(n, _)| n == nom).unwrap().1.clone()).unwrap()
        };
        let types = lire("[Content_Types].xml");
        assert!(types.contains("/ppt/slides/slide2.xml"));
        let s1 = lire("ppt/slides/slide1.xml");
        assert!(s1.contains("Plan &amp; suite"));
        assert!(s1.contains("<a:buChar char=\"•\"/>"));
        assert!(s1.contains("arabicPeriod"));
        let rels = lire("ppt/_rels/presentation.xml.rels");
        assert!(rels.contains("slides/slide2.xml") && rels.contains("theme/theme1.xml"));
        assert!(lire("ppt/presentation.xml").contains("<p:sldId id=\"257\" r:id=\"rId3\"/>"));
    }

    #[test]
    fn document_vide_une_diapo() {
        let z = generer(&[]);
        let e = crate::zipw::tests::lire(&z);
        assert!(e.iter().any(|(n, _)| n == "ppt/slides/slide1.xml"));
        assert!(!e.iter().any(|(n, _)| n == "ppt/slides/slide2.xml"));
    }
}
