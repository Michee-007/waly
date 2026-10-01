//! PDF : écrit à la main, sans dépendance (flux compressés via flate2).
//! Polices standard Helvetica (normal, gras, italique, gras-italique) en
//! WinAnsiEncoding — tous les accents français passent. Coupure des lignes
//! avec les VRAIES largeurs de glyphes (métriques AFM Helvetica), titres,
//! puces, listes numérotées, tableaux quadrillés, sauts de page, page A4.

use std::io::Write;

use super::{Bloc, Segment};

const LARGEUR_PAGE: f32 = 595.28;
const HAUTEUR_PAGE: f32 = 841.89;
const MARGE: f32 = 56.7; // 2 cm
const UTILE: f32 = LARGEUR_PAGE - 2.0 * MARGE;

/// Largeurs AFM (1/1000 em) des caractères ASCII 32..=126.
const HELV: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];
const HELV_GRAS: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667,
    611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556,
    278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];

/// Unicode → octet Windows-1252 (`?` hors table).
fn cp1252(c: char) -> u8 {
    let u = c as u32;
    if u < 0x80 || (0xA0..=0xFF).contains(&u) {
        return u as u8;
    }
    match c {
        '€' => 0x80, '‚' => 0x82, 'ƒ' => 0x83, '„' => 0x84, '…' => 0x85, '†' => 0x86, '‡' => 0x87,
        'ˆ' => 0x88, '‰' => 0x89, 'Š' => 0x8A, '‹' => 0x8B, 'Œ' => 0x8C, 'Ž' => 0x8E, '‘' => 0x91,
        '’' => 0x92, '“' => 0x93, '”' => 0x94, '•' => 0x95, '–' => 0x96, '—' => 0x97, '˜' => 0x98,
        '™' => 0x99, 'š' => 0x9A, '›' => 0x9B, 'œ' => 0x9C, 'ž' => 0x9E, 'Ÿ' => 0x9F,
        '\u{202F}' | '\u{2009}' => b' ', // espaces fines françaises
        _ => b'?',
    }
}

/// Largeur (1/1000 em) d'un caractère ; les lettres accentuées prennent
/// la largeur de leur lettre de base.
fn largeur_car(c: char, gras: bool) -> u16 {
    let table = if gras { &HELV_GRAS } else { &HELV };
    let base = match c {
        'à' | 'â' | 'ä' | 'á' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'î' | 'ï' | 'í' | 'ì' => 'i',
        'ô' | 'ö' | 'ó' | 'ò' | 'õ' => 'o',
        'ù' | 'û' | 'ü' | 'ú' => 'u',
        'ç' => 'c',
        'ÿ' => 'y',
        'À' | 'Â' | 'Ä' | 'Á' => 'A',
        'É' | 'È' | 'Ê' | 'Ë' => 'E',
        'Î' | 'Ï' => 'I',
        'Ô' | 'Ö' => 'O',
        'Ù' | 'Û' | 'Ü' => 'U',
        'Ç' => 'C',
        '\u{202F}' | '\u{2009}' => ' ',
        'œ' | 'Œ' | '—' | '…' | '‰' => return 1000,
        '’' | '‘' => return 222,
        '•' => return 350,
        '°' => return 400,
        c => c,
    };
    let u = base as u32;
    if (32..=126).contains(&u) {
        table[(u - 32) as usize]
    } else {
        556
    }
}

/// Un mot mis en forme, prêt à poser.
struct Mot {
    octets: Vec<u8>,
    police: usize, // 1..=4 → /F1../F4
    largeur: f32,  // en points, à la taille donnée
}

fn police(gras: bool, italique: bool) -> usize {
    match (gras, italique) {
        (false, false) => 1,
        (true, false) => 2,
        (false, true) => 3,
        (true, true) => 4,
    }
}

fn mesurer(texte: &str, gras: bool, taille: f32) -> f32 {
    texte.chars().map(|c| largeur_car(c, gras) as f32).sum::<f32>() * taille / 1000.0
}

/// Découpe des segments en mots ; un mot plus large que `max` est coupé.
fn mots(segs: &[Segment], taille: f32, gras_force: bool, max: f32) -> Vec<Mot> {
    let mut out = Vec::new();
    for g in segs {
        let gras = g.gras || gras_force;
        for m in g.texte.split_whitespace() {
            let mut cur = String::new();
            for c in m.chars() {
                cur.push(c);
                if mesurer(&cur, gras, taille) > max && cur.chars().count() > 1 {
                    let dernier = cur.pop().unwrap();
                    out.push(Mot {
                        octets: cur.chars().map(cp1252).collect(),
                        police: police(gras, g.italique),
                        largeur: mesurer(&cur, gras, taille),
                    });
                    cur = dernier.to_string();
                }
            }
            out.push(Mot {
                octets: cur.chars().map(cp1252).collect(),
                police: police(gras, g.italique),
                largeur: mesurer(&cur, gras, taille),
            });
        }
    }
    out
}

/// Répartit les mots en lignes de largeur ≤ `max`.
fn lignes(mots: Vec<Mot>, taille: f32, max: f32) -> Vec<Vec<Mot>> {
    let espace = 278.0 * taille / 1000.0;
    let (mut out, mut cur, mut l): (Vec<Vec<Mot>>, Vec<Mot>, f32) = (Vec::new(), Vec::new(), 0.0);
    for m in mots {
        let ajout = if cur.is_empty() { m.largeur } else { espace + m.largeur };
        if !cur.is_empty() && l + ajout > max {
            out.push(std::mem::take(&mut cur));
            l = 0.0;
        }
        l += if cur.is_empty() { m.largeur } else { espace + m.largeur };
        cur.push(m);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn echapper(octets: &[u8]) -> Vec<u8> {
    let mut o = Vec::with_capacity(octets.len());
    for &b in octets {
        if matches!(b, b'(' | b')' | b'\\') {
            o.push(b'\\');
        }
        o.push(b);
    }
    o
}

struct Pages {
    finies: Vec<Vec<u8>>,
    cur: Vec<u8>,
    y: f32,
}

impl Pages {
    fn new() -> Self {
        Self { finies: Vec::new(), cur: Vec::new(), y: HAUTEUR_PAGE - MARGE }
    }
    fn en_haut(&self) -> bool {
        self.y >= HAUTEUR_PAGE - MARGE - 0.1
    }
    fn saut(&mut self) {
        let p = std::mem::take(&mut self.cur);
        self.finies.push(p);
        self.y = HAUTEUR_PAGE - MARGE;
    }
    fn besoin(&mut self, h: f32) {
        if self.y - h < MARGE && !self.en_haut() {
            self.saut();
        }
    }
    fn poser(&mut self, x: f32, base: f32, taille: f32, police: usize, octets: &[u8]) {
        let _ = write!(self.cur, "BT /F{police} {taille:.1} Tf {x:.2} {base:.2} Td (");
        self.cur.extend_from_slice(&echapper(octets));
        self.cur.extend_from_slice(b") Tj ET\n");
    }
    /// Pose un paragraphe coupé en lignes ; `puce` = marqueur à gauche.
    fn paragraphe(&mut self, segs: &[Segment], taille: f32, x: f32, gras: bool, puce: Option<(&str, f32)>) {
        let max = LARGEUR_PAGE - MARGE - x;
        let interligne = taille * 1.35;
        let espace = 278.0 * taille / 1000.0;
        for (i, ligne) in lignes(mots(segs, taille, gras, max), taille, max).into_iter().enumerate() {
            self.besoin(interligne);
            let base = self.y - taille;
            if let (0, Some((marque, mx))) = (i, puce) {
                let o: Vec<u8> = marque.chars().map(cp1252).collect();
                self.poser(mx, base, taille, 1, &o);
            }
            let mut cx = x;
            for m in &ligne {
                self.poser(cx, base, taille, m.police, &m.octets);
                cx += m.largeur + espace;
            }
            self.y -= interligne;
        }
    }
    fn tableau(&mut self, t: &[Vec<String>]) {
        let cols = t.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let (taille, pad) = (9.5, 4.0);
        let lc = UTILE / cols as f32;
        let interligne = taille * 1.3;
        for (r, ligne) in t.iter().enumerate() {
            let cellules: Vec<Vec<Vec<Mot>>> = (0..cols)
                .map(|c| {
                    let s = [Segment { texte: ligne.get(c).cloned().unwrap_or_default(), gras: r == 0, italique: false }];
                    lignes(mots(&s, taille, false, lc - 2.0 * pad), taille, lc - 2.0 * pad)
                })
                .collect();
            let n = cellules.iter().map(Vec::len).max().unwrap_or(1).max(1);
            let h = n as f32 * interligne + 2.0 * pad;
            self.besoin(h);
            let haut = self.y;
            let espace = 278.0 * taille / 1000.0;
            for (c, cel) in cellules.iter().enumerate() {
                let x0 = MARGE + c as f32 * lc;
                let _ = writeln!(self.cur, "0.5 w {x0:.2} {:.2} {lc:.2} {h:.2} re S", haut - h);
                for (i, l) in cel.iter().enumerate() {
                    let base = haut - pad - taille - i as f32 * interligne;
                    let mut cx = x0 + pad;
                    for m in l {
                        self.poser(cx, base, taille, m.police, &m.octets);
                        cx += m.largeur + espace;
                    }
                }
            }
            self.y -= h;
        }
        self.y -= 8.0;
    }
}

fn texte_simple(t: &str) -> Vec<Segment> {
    vec![Segment { texte: t.to_string(), gras: false, italique: false }]
}

/// Rend les blocs en fichier PDF.
pub fn generer(blocs: &[Bloc]) -> Vec<u8> {
    let mut p = Pages::new();
    let mut numero = 0;
    for b in blocs {
        if !matches!(b, Bloc::Numero(_)) {
            numero = 0;
        }
        match b {
            Bloc::Titre { niveau, texte } => {
                let taille = match niveau { 1 => 20.0, 2 => 15.0, _ => 12.5 };
                if !p.en_haut() {
                    p.y -= 10.0;
                }
                p.besoin(taille * 3.0); // un titre ne reste pas seul en bas de page
                p.paragraphe(&texte_simple(texte), taille, MARGE, true, None);
                p.y -= 4.0;
            }
            Bloc::Paragraphe(s) => {
                p.paragraphe(s, 11.0, MARGE, false, None);
                p.y -= 6.0;
            }
            Bloc::Puce(s) => {
                p.paragraphe(s, 11.0, MARGE + 18.0, false, Some(("•", MARGE + 6.0)));
                p.y -= 2.0;
            }
            Bloc::Numero(s) => {
                numero += 1;
                let marque = format!("{numero}.");
                p.paragraphe(s, 11.0, MARGE + 20.0, false, Some((&marque, MARGE + 4.0)));
                p.y -= 2.0;
            }
            Bloc::Tableau(t) => {
                // Respiration avant un tableau (vu au rendu : il collait à
                // la liste précédente).
                if !p.en_haut() {
                    p.y -= 6.0;
                }
                p.tableau(t);
            }
            Bloc::Separateur => {
                if !p.en_haut() {
                    p.saut();
                }
            }
        }
    }
    if !p.cur.is_empty() || p.finies.is_empty() {
        p.saut();
    }
    let titre = blocs.iter().find_map(|b| match b {
        Bloc::Titre { texte, .. } => Some(texte.clone()),
        _ => None,
    });
    assembler(&p.finies, titre.as_deref())
}

/// Objets : 1 catalogue, 2 pages, 3-6 polices, 7 infos, puis par page
/// k : 8+2k (page) et 9+2k (contenu compressé).
fn assembler(pages: &[Vec<u8>], titre: Option<&str>) -> Vec<u8> {
    let mut objets: Vec<Vec<u8>> = Vec::new();
    let kids: Vec<String> = (0..pages.len()).map(|k| format!("{} 0 R", 8 + 2 * k)).collect();
    objets.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    objets.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into_bytes());
    for nom in ["Helvetica", "Helvetica-Bold", "Helvetica-Oblique", "Helvetica-BoldOblique"] {
        objets.push(
            format!("<< /Type /Font /Subtype /Type1 /BaseFont /{nom} /Encoding /WinAnsiEncoding >>").into_bytes(),
        );
    }
    let mut infos = b"<< /Producer (Waly)".to_vec();
    if let Some(t) = titre {
        infos.extend_from_slice(b" /Title (");
        infos.extend_from_slice(&echapper(&t.chars().map(cp1252).collect::<Vec<u8>>()));
        infos.extend_from_slice(b")");
    }
    infos.extend_from_slice(b" >>");
    objets.push(infos);
    for (k, contenu) in pages.iter().enumerate() {
        objets.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {LARGEUR_PAGE} {HAUTEUR_PAGE}] \
                 /Resources << /Font << /F1 3 0 R /F2 4 0 R /F3 5 0 R /F4 6 0 R >> >> /Contents {} 0 R >>",
                9 + 2 * k
            )
            .into_bytes(),
        );
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(contenu).expect("zlib en memoire");
        let comp = enc.finish().expect("zlib en memoire");
        let mut o = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", comp.len()).into_bytes();
        o.extend_from_slice(&comp);
        o.extend_from_slice(b"\nendstream");
        objets.push(o);
    }
    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objets.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objets.len() + 1).as_bytes());
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R /Info 7 0 R >>\nstartxref\n{xref}\n%%EOF\n", objets.len() + 1)
            .as_bytes(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::super::analyser;
    use super::*;
    use std::io::Read;

    fn flux(pdf: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut i = 0;
        // « >>\nstream\n » : ne PAS confondre avec la fin de « endstream ».
        while let Some(p) = pdf[i..].windows(10).position(|w| w == b">>\nstream\n") {
            let debut = i + p + 10;
            let fin = debut + pdf[debut..].windows(10).position(|w| w == b"\nendstream").unwrap();
            let mut d = flate2::read::ZlibDecoder::new(&pdf[debut..fin]);
            let mut v = Vec::new();
            d.read_to_end(&mut v).unwrap();
            out.push(v);
            i = fin;
        }
        out
    }

    #[test]
    fn structure_xref_et_pages() {
        let pdf = generer(&analyser("# Plan\nUn\n---\n## Deux"));
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.contains("/Count 2"), "--- = saut de page");
        // Chaque décalage de la table xref pointe sur « N 0 obj ».
        let xref = s.find("xref\n").unwrap();
        for (n, l) in s[xref..].lines().skip(3).take_while(|l| l.ends_with(" n ")).enumerate() {
            let off: usize = l[..10].parse().unwrap();
            assert!(pdf[off..].starts_with(format!("{} 0 obj", n + 1).as_bytes()), "objet {}", n + 1);
        }
    }

    #[test]
    fn accents_en_winansi_et_titre() {
        let pdf = generer(&analyser("# Réunion d'équipe\nÉlodie & Noël : « budget » (2 €)"));
        let c = flux(&pdf).concat();
        // Chaque mot est posé par son propre Tj.
        assert!(c.windows(9).any(|w| w == b"(R\xE9union)"), "é en cp1252");
        assert!(c.windows(4).any(|w| w == b"(\\(2"), "parenthese echappee");
        assert!(c.windows(4).any(|w| w == b"\x80\\))"), "euro 0x80");
        assert!(String::from_utf8_lossy(&pdf).contains("/Title (R"));
    }

    #[test]
    fn long_paragraphe_coupe_dans_la_marge() {
        let long = "mot ".repeat(200);
        let pdf = generer(&analyser(&long));
        let c = String::from_utf8_lossy(&flux(&pdf).concat()).to_string();
        assert_eq!(c.matches(" Tj").count(), 200);
        // Aucun mot ne commence au-delà de la marge droite.
        for l in c.lines().filter(|l| l.starts_with("BT ")) {
            let x: f32 = l.split_whitespace().nth(4).unwrap().parse().unwrap();
            assert!(x < LARGEUR_PAGE - MARGE, "x={x}");
        }
    }
}
