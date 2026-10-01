//! Aperçu DANS l'app des créations (UI 2026-09-14) : Waly ne doit pas
//! renvoyer vers l'Explorateur pour montrer ce qu'il a fait.
//!
//! Rust pur, aucune dépendance neuve : un LECTEUR ZIP minimal (le pendant de
//! `zipw.rs`, flate2 déjà dans le graphe) pour les documents Office, et une
//! extraction XML par balayage (les documents viennent de Waly ou d'Office,
//! on n'a besoin que du texte et de la structure). Les PDF sont rendus côté
//! desktop par le moteur Windows (`waly_sight::pdf`).
//!
//! Sortie JSON pour l'UI (`type` = texte | markdown | html | tableau |
//! document | diapos | image | inconnu), toujours bornée (`tronque`).

use std::io::Read;
use std::path::Path;

use serde_json::json;

const TAILLE_MAX: u64 = 50 * 1024 * 1024;
const TEXTE_MAX: usize = 400 * 1024;
const LIGNES_MAX: usize = 500;
const COLONNES_MAX: usize = 40;
const BLOCS_MAX: usize = 3000;

/// Aperçu d'un fichier (le contrôle « dossier autorisé » est fait par l'hôte).
pub fn apercu(chemin: &Path) -> Result<serde_json::Value, String> {
    let nom = chemin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let meta = std::fs::metadata(chemin).map_err(|_| "fichier introuvable".to_string())?;
    if meta.len() > TAILLE_MAX {
        return Ok(json!({"type": "inconnu", "nom": nom, "message": "Fichier trop volumineux pour l'aperçu."}));
    }
    let ext = chemin.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let octets = || std::fs::read(chemin).map_err(|e| format!("lecture impossible : {e}"));
    let mut v = match ext.as_str() {
        "md" | "markdown" => {
            let (t, tronque) = texte_borne(&octets()?);
            json!({"type": "markdown", "contenu": t, "tronque": tronque})
        }
        "html" | "htm" => {
            let (t, tronque) = texte_borne(&octets()?);
            json!({"type": "html", "contenu": t, "tronque": tronque})
        }
        "txt" | "log" | "json" | "xml" | "yaml" | "yml" | "toml" | "ini" | "rs" | "py" | "js" | "ts" | "css"
        | "sql" | "tex" | "sh" | "ps1" => {
            let (t, tronque) = texte_borne(&octets()?);
            json!({"type": "texte", "langage": ext, "contenu": t, "tronque": tronque})
        }
        "csv" | "tsv" => {
            let (t, _) = texte_borne(&octets()?);
            let (lignes, tronque) = csv(&t, if ext == "tsv" { '\t' } else { detecter_separateur(&t) });
            json!({"type": "tableau", "feuilles": [{"nom": nom, "lignes": lignes}], "tronque": tronque})
        }
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" => {
            let mime = match ext.as_str() {
                "jpg" | "jpeg" => "image/jpeg",
                "svg" => "image/svg+xml",
                autre => return Ok(image(&nom, &format!("image/{autre}"), &octets()?)),
            };
            image(&nom, mime, &octets()?)
        }
        "docx" => docx(&octets()?)?,
        "xlsx" => xlsx(&octets()?)?,
        "pptx" => pptx(&octets()?)?,
        autre => json!({"type": "inconnu", "message": format!("Pas d'aperçu pour les fichiers .{autre}.")}),
    };
    v["nom"] = json!(nom);
    Ok(v)
}

/// Texte brut d'un fichier joint au message (lot 1, 2026-09-30) — réutilise
/// l'aperçu et l'aplatit. Images et PDF sont traités par l'hôte (vision,
/// OCR) : ici ils sont refusés, comme les types inconnus.
pub fn texte(chemin: &Path) -> Result<String, String> {
    let v = apercu(chemin)?;
    let s = |x: &serde_json::Value| x.as_str().unwrap_or("").to_string();
    let t = match v["type"].as_str().unwrap_or("") {
        "markdown" | "html" | "texte" => s(&v["contenu"]),
        "tableau" => {
            let mut out = String::new();
            for f in v["feuilles"].as_array().into_iter().flatten() {
                out.push_str(&format!("Feuille {} :\n", s(&f["nom"])));
                for l in f["lignes"].as_array().into_iter().flatten() {
                    let cells: Vec<String> = l.as_array().into_iter().flatten().map(s).collect();
                    out.push_str(&cells.join("\t"));
                    out.push('\n');
                }
            }
            out
        }
        "document" => v["blocs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|b| {
                if b["genre"] == "tableau" {
                    b["lignes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|l| l.as_array().into_iter().flatten().map(s).collect::<Vec<_>>().join("\t"))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else if b["genre"] == "puce" {
                    format!("- {}", s(&b["texte"]))
                } else {
                    s(&b["texte"])
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        "diapos" => v["diapos"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| {
                let t: Vec<String> = d["textes"].as_array().into_iter().flatten().map(s).collect();
                format!("Diapo {} : {}", d["numero"], t.join(" / "))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => {
            return Err(v["message"]
                .as_str()
                .unwrap_or("Ce type de fichier ne se lit pas encore.")
                .to_string())
        }
    };
    Ok(t)
}

fn image(nom: &str, mime: &str, octets: &[u8]) -> serde_json::Value {
    json!({"type": "image", "nom": nom, "data_url": format!("data:{mime};base64,{}", crate::native_tools::base64(octets))})
}

fn texte_borne(octets: &[u8]) -> (String, bool) {
    let tronque = octets.len() > TEXTE_MAX;
    let t = String::from_utf8_lossy(&octets[..octets.len().min(TEXTE_MAX)]).into_owned();
    (t.trim_start_matches('\u{feff}').to_string(), tronque)
}

// ── CSV ─────────────────────────────────────────────────────────────────────

fn detecter_separateur(t: &str) -> char {
    let premiere = t.lines().next().unwrap_or("");
    if premiere.matches(';').count() > premiere.matches(',').count() {
        ';'
    } else {
        ','
    }
}

/// CSV avec guillemets (`"a,b"`, `""` échappé) ; borné.
pub fn csv(t: &str, sep: char) -> (Vec<Vec<String>>, bool) {
    let mut lignes: Vec<Vec<String>> = Vec::new();
    let mut ligne: Vec<String> = Vec::new();
    let mut champ = String::new();
    let mut guillemets = false;
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if guillemets {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    champ.push('"');
                    chars.next();
                } else {
                    guillemets = false;
                }
            } else {
                champ.push(c);
            }
            continue;
        }
        match c {
            '"' => guillemets = true,
            '\r' => {}
            '\n' => {
                ligne.push(std::mem::take(&mut champ));
                lignes.push(std::mem::take(&mut ligne));
                if lignes.len() >= LIGNES_MAX {
                    return (lignes, true);
                }
            }
            c if c == sep => ligne.push(std::mem::take(&mut champ)),
            c => champ.push(c),
        }
    }
    if !champ.is_empty() || !ligne.is_empty() {
        ligne.push(champ);
        lignes.push(ligne);
    }
    (lignes, false)
}

// ── ZIP (lecture) ───────────────────────────────────────────────────────────

fn u16le(b: &[u8], i: usize) -> Option<usize> {
    Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]) as usize)
}
fn u32le(b: &[u8], i: usize) -> Option<usize> {
    Some(u32::from_le_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, *b.get(i + 3)?]) as usize)
}

/// Lit toutes les entrées d'une archive ZIP (stockées ou DEFLATE, sans ZIP64).
pub fn lire_zip(b: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let err = || "archive illisible".to_string();
    let debut = b.len().saturating_sub(66_000);
    let fin = (debut..b.len().saturating_sub(21))
        .rev()
        .find(|&i| b[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or_else(err)?;
    let n = u16le(b, fin + 10).ok_or_else(err)?;
    let mut p = u32le(b, fin + 16).ok_or_else(err)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        if b.get(p..p + 4) != Some(&[0x50, 0x4b, 0x01, 0x02]) {
            return Err(err());
        }
        let methode = u16le(b, p + 10).ok_or_else(err)?;
        let taille_c = u32le(b, p + 20).ok_or_else(err)?;
        let taille_u = u32le(b, p + 24).ok_or_else(err)?;
        let (ln, le, lc) = (u16le(b, p + 28).ok_or_else(err)?, u16le(b, p + 30).ok_or_else(err)?, u16le(b, p + 32).ok_or_else(err)?);
        let local = u32le(b, p + 42).ok_or_else(err)?;
        let nom = String::from_utf8_lossy(b.get(p + 46..p + 46 + ln).ok_or_else(err)?).into_owned();
        p += 46 + ln + le + lc;
        if b.get(local..local + 4) != Some(&[0x50, 0x4b, 0x03, 0x04]) || taille_u > TAILLE_MAX as usize {
            return Err(err());
        }
        let d = local + 30 + u16le(b, local + 26).ok_or_else(err)? + u16le(b, local + 28).ok_or_else(err)?;
        let brut = b.get(d..d + taille_c).ok_or_else(err)?;
        let donnees = match methode {
            0 => brut.to_vec(),
            8 => {
                let mut v = Vec::with_capacity(taille_u);
                flate2::read::DeflateDecoder::new(brut)
                    .take(TAILLE_MAX)
                    .read_to_end(&mut v)
                    .map_err(|_| err())?;
                v
            }
            _ => continue, // méthode exotique : entrée ignorée
        };
        out.push((nom, donnees));
    }
    Ok(out)
}

fn entree<'a>(z: &'a [(String, Vec<u8>)], nom: &str) -> Option<String> {
    z.iter().find(|(n, _)| n == nom).map(|(_, d)| String::from_utf8_lossy(d).into_owned())
}

// ── XML par balayage ────────────────────────────────────────────────────────

/// Éléments `<tag …>…</tag>` (et `<tag …/>`, contenu vide) : (attributs, contenu).
fn elements<'a>(xml: &'a str, tag: &str) -> Vec<(&'a str, &'a str)> {
    let ouvre = format!("<{tag}");
    let ferme = format!("</{tag}>");
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(k) = xml[i..].find(&ouvre) {
        let s = i + k;
        let apres = s + ouvre.len();
        match xml[apres..].chars().next() {
            Some('>') | Some(' ') | Some('/') | Some('\t') | Some('\n') | Some('\r') => {}
            _ => {
                i = apres;
                continue;
            }
        }
        let Some(g) = xml[apres..].find('>') else { break };
        let fin_ouvrant = apres + g;
        let attrs = &xml[apres..fin_ouvrant];
        if attrs.ends_with('/') {
            out.push((attrs, ""));
            i = fin_ouvrant + 1;
            continue;
        }
        let Some(f) = xml[fin_ouvrant + 1..].find(&ferme) else { break };
        out.push((attrs, &xml[fin_ouvrant + 1..fin_ouvrant + 1 + f]));
        i = fin_ouvrant + 1 + f + ferme.len();
    }
    out
}

fn textes(xml: &str, tag: &str) -> String {
    elements(xml, tag).iter().map(|(_, t)| desechapper(t)).collect()
}

fn attr(attrs: &str, nom: &str) -> Option<String> {
    let cle = format!("{nom}=\"");
    let d = attrs.find(&cle)? + cle.len();
    let f = attrs[d..].find('"')?;
    Some(desechapper(&attrs[d..d + f]))
}

fn desechapper(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut reste = s;
    while let Some(i) = reste.find('&') {
        out.push_str(&reste[..i]);
        let apres = &reste[i..];
        let Some(fin) = apres.find(';') else {
            out.push_str(apres);
            return out;
        };
        let ent = &apres[1..fin];
        let c = match ent {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => out.push(c),
            None => out.push_str(&apres[..=fin]),
        }
        reste = &apres[fin + 1..];
    }
    out.push_str(reste);
    out
}

// ── Word ────────────────────────────────────────────────────────────────────

/// Début de la prochaine balise `<tag` (frontière de nom vérifiée).
fn prochain(xml: &str, tag: &str) -> Option<usize> {
    let ouvre = format!("<{tag}");
    let mut i = 0;
    while let Some(k) = xml[i..].find(&ouvre) {
        let s = i + k;
        match xml[s + ouvre.len()..].chars().next() {
            Some('>') | Some(' ') | Some('/') | Some('\t') | Some('\n') | Some('\r') => return Some(s),
            _ => i = s + ouvre.len(),
        }
    }
    None
}

fn docx(b: &[u8]) -> Result<serde_json::Value, String> {
    let z = lire_zip(b)?;
    let xml = entree(&z, "word/document.xml").ok_or("document Word illisible")?;
    let mut blocs = Vec::new();
    let mut tronque = false;
    // Balayage DANS L'ORDRE du corps : paragraphes et tableaux (un tableau
    // devient un bloc « tableau », ses cellules ne sont pas des paragraphes).
    let mut i = 0;
    loop {
        if blocs.len() >= BLOCS_MAX {
            tronque = true;
            break;
        }
        let (p, t) = (prochain(&xml[i..], "w:p"), prochain(&xml[i..], "w:tbl"));
        let (k, table) = match (p, t) {
            (None, None) => break,
            (Some(a), None) => (a, false),
            (None, Some(b)) => (b, true),
            (Some(a), Some(b)) => if b < a { (b, true) } else { (a, false) },
        };
        let s = i + k;
        if table {
            let fin = xml[s..].find("</w:tbl>").map(|f| s + f + "</w:tbl>".len()).unwrap_or(xml.len());
            let lignes: Vec<Vec<String>> = elements(&xml[s..fin], "w:tr")
                .iter()
                .map(|(_, tr)| elements(tr, "w:tc").iter().map(|(_, tc)| textes(tc, "w:t")).collect())
                .collect();
            blocs.push(json!({"genre": "tableau", "lignes": lignes}));
            i = fin;
            continue;
        }
        let g = xml[s..].find('>').map(|g| s + g).unwrap_or(xml.len() - 1);
        if xml[..=g].ends_with("/>") {
            i = g + 1;
            continue;
        }
        let fin = xml[g..].find("</w:p>").map(|f| g + f + "</w:p>".len()).unwrap_or(xml.len());
        let p = &xml[g + 1..fin.saturating_sub("</w:p>".len()).max(g + 1)];
        i = fin;
        let texte = textes(p, "w:t");
        if texte.trim().is_empty() {
            continue;
        }
        let style = elements(p, "w:pStyle")
            .first()
            .and_then(|(a, _)| attr(a, "w:val"))
            .unwrap_or_default()
            .to_lowercase();
        let genre = if style.contains("title") || style.ends_with("heading1") || style.ends_with("titre1") || style == "heading1" {
            "titre1"
        } else if style.contains("heading2") || style.contains("titre2") {
            "titre2"
        } else if style.contains("heading3") || style.contains("titre3") {
            "titre3"
        } else if p.contains("<w:numPr") || style.contains("list") || style.contains("liste") {
            "puce"
        } else {
            "para"
        };
        blocs.push(json!({"genre": genre, "texte": texte}));
    }
    Ok(json!({"type": "document", "blocs": blocs, "tronque": tronque}))
}

// ── Excel ───────────────────────────────────────────────────────────────────

fn colonne(reference: &str) -> usize {
    let mut n = 0usize;
    for c in reference.chars().take_while(|c| c.is_ascii_alphabetic()) {
        n = n * 26 + (c.to_ascii_uppercase() as usize - 'A' as usize + 1);
    }
    n.saturating_sub(1)
}

fn xlsx(b: &[u8]) -> Result<serde_json::Value, String> {
    let z = lire_zip(b)?;
    let partages: Vec<String> = entree(&z, "xl/sharedStrings.xml")
        .map(|x| elements(&x, "si").iter().map(|(_, si)| textes(si, "t")).collect())
        .unwrap_or_default();
    let rels = entree(&z, "xl/_rels/workbook.xml.rels").unwrap_or_default();
    let classeur = entree(&z, "xl/workbook.xml").unwrap_or_default();
    let mut feuilles = Vec::new();
    let mut tronque = false;
    for (i, (a, _)) in elements(&classeur, "sheet").iter().enumerate() {
        let nom = attr(a, "name").unwrap_or_else(|| format!("Feuille {}", i + 1));
        let cible = attr(a, "r:id")
            .and_then(|rid| {
                elements(&rels, "Relationship")
                    .into_iter()
                    .find(|(ra, _)| attr(ra, "Id").as_deref() == Some(rid.as_str()))
                    .and_then(|(ra, _)| attr(ra, "Target"))
            })
            .map(|t| if let Some(s) = t.strip_prefix('/') { s.to_string() } else { format!("xl/{t}") })
            .unwrap_or_else(|| format!("xl/worksheets/sheet{}.xml", i + 1));
        let Some(xml) = entree(&z, &cible) else { continue };
        let mut lignes: Vec<Vec<String>> = Vec::new();
        for (_, row) in elements(&xml, "row") {
            if lignes.len() >= LIGNES_MAX {
                tronque = true;
                break;
            }
            let mut ligne: Vec<String> = Vec::new();
            for (ca, c) in elements(row, "c") {
                let col = attr(ca, "r").map(|r| colonne(&r)).unwrap_or(ligne.len());
                if col >= COLONNES_MAX {
                    tronque = true;
                    continue;
                }
                let v = textes(c, "v");
                let valeur = match attr(ca, "t").as_deref() {
                    Some("s") => v.trim().parse::<usize>().ok().and_then(|k| partages.get(k).cloned()).unwrap_or_default(),
                    Some("inlineStr") => textes(c, "t"),
                    Some("b") => if v.trim() == "1" { "VRAI".into() } else { "FAUX".into() },
                    _ => v,
                };
                if ligne.len() <= col {
                    ligne.resize(col + 1, String::new());
                }
                ligne[col] = valeur;
            }
            lignes.push(ligne);
        }
        feuilles.push(json!({"nom": nom, "lignes": lignes}));
    }
    Ok(json!({"type": "tableau", "feuilles": feuilles, "tronque": tronque}))
}

// ── PowerPoint ──────────────────────────────────────────────────────────────

fn pptx(b: &[u8]) -> Result<serde_json::Value, String> {
    let z = lire_zip(b)?;
    let mut diapos: Vec<(usize, &str)> = z
        .iter()
        .filter_map(|(n, _)| {
            let num = n.strip_prefix("ppt/slides/slide")?.strip_suffix(".xml")?.parse().ok()?;
            Some((num, n.as_str()))
        })
        .collect();
    diapos.sort_by_key(|d| d.0);
    let diapos: Vec<serde_json::Value> = diapos
        .iter()
        .filter_map(|(num, nom)| {
            let xml = entree(&z, nom)?;
            let textes: Vec<String> = elements(&xml, "a:p")
                .iter()
                .map(|(_, p)| textes(p, "a:t"))
                .filter(|t| !t.trim().is_empty())
                .collect();
            Some(json!({"numero": num, "textes": textes}))
        })
        .collect();
    Ok(json!({"type": "diapos", "diapos": diapos, "tronque": false}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fichier(nom: &str, octets: &[u8]) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("waly-apercu-{}-{nom}", std::process::id()));
        std::fs::write(&p, octets).unwrap();
        p
    }

    fn tout_le_texte(v: &serde_json::Value) -> String {
        v.to_string()
    }

    #[test]
    fn zip_aller_retour_avec_notre_ecrivain() {
        let mut z = crate::zipw::Zip::new();
        z.ajouter("a.txt", "bonjour".repeat(50).as_bytes());
        z.ajouter("dossier/b.xml", b"<x>&amp;</x>");
        let e = lire_zip(&z.finir()).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].1, "bonjour".repeat(50).as_bytes());
        assert_eq!(e[1].0, "dossier/b.xml");
    }

    #[test]
    fn word_excel_powerpoint_generes_par_waly_se_relisent() {
        let md = "# Bilan T3\n\nLe total est juste.\n\n- loyer\n- charges\n\n| Poste | Montant |\n|---|---|\n| Loyer | 900 |\n";
        for (ext, genre, attendu) in [("docx", "document", "Le total est juste."), ("xlsx", "tableau", "Loyer"), ("pptx", "diapos", "Bilan T3")] {
            let octets = crate::documents::generer(ext, md).unwrap();
            let p = fichier(&format!("t.{ext}"), &octets);
            let v = apercu(&p).unwrap();
            std::fs::remove_file(&p).ok();
            assert_eq!(v["type"], genre, "{ext} : {v}");
            assert!(tout_le_texte(&v).contains(attendu), "{ext} : {attendu} absent de {v}");
            if ext == "docx" {
                // Le tableau markdown devient UN bloc tableau, pas des paragraphes.
                let t = v["blocs"].as_array().unwrap().iter().find(|b| b["genre"] == "tableau").expect("bloc tableau");
                assert_eq!(t["lignes"][1][0], "Loyer", "{t}");
                assert!(!v["blocs"].as_array().unwrap().iter().any(|b| b["texte"] == "Loyer"), "{v}");
            }
        }
    }

    #[test]
    fn csv_guillemets_et_separateur() {
        let (l, t) = csv("nom;montant\n\"Dupont; Jean\";\"12,5\"\n", ';');
        assert!(!t);
        assert_eq!(l, vec![vec!["nom", "montant"], vec!["Dupont; Jean", "12,5"]]);
        assert_eq!(detecter_separateur("a;b;c\n"), ';');
        let (l, _) = csv("a,\"dit \"\"oui\"\"\"\n", ',');
        assert_eq!(l[0][1], "dit \"oui\"");
    }

    #[test]
    fn types_simples_et_inconnu() {
        let p = fichier("n.md", "# Titre\n".as_bytes());
        assert_eq!(apercu(&p).unwrap()["type"], "markdown");
        std::fs::remove_file(&p).ok();
        let p = fichier("x.bin", &[0, 1, 2]);
        let v = apercu(&p).unwrap();
        std::fs::remove_file(&p).ok();
        assert_eq!(v["type"], "inconnu");
        assert_eq!(desechapper("a &lt;b&gt; &amp; &#233; &#x41;"), "a <b> & é A");
        assert_eq!(colonne("AB12"), 27);
    }
}
