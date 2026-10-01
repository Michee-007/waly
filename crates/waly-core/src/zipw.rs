//! Écrivain ZIP minimal (Mains v2 — documents Office : docx/xlsx/pptx sont
//! des archives ZIP de XML). Entrées DEFLATE, noms UTF-8, sans ZIP64 (des
//! documents de quelques Mo au plus). flate2 (backend Rust pur
//! miniz_oxide) + crc32fast, déjà dans le graphe du workspace : aucune
//! dépendance neuve, rien de natif (piège 3). Date fixe → sortie
//! déterministe (mêmes entrées = mêmes octets).

use std::io::Write;

/// Date DOS fixe : 2026-01-01 00:00.
const DATE_DOS: u16 = ((2026 - 1980) << 9) | (1 << 5) | 1;
const HEURE_DOS: u16 = 0;
/// Bit 11 : noms encodés en UTF-8.
const DRAPEAU_UTF8: u16 = 0x0800;
const DEFLATE: u16 = 8;

pub struct Zip {
    corps: Vec<u8>,
    central: Vec<u8>,
    entrees: u16,
}

fn p16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn p32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

impl Default for Zip {
    fn default() -> Self {
        Self::new()
    }
}

impl Zip {
    pub fn new() -> Self {
        Self { corps: Vec::new(), central: Vec::new(), entrees: 0 }
    }

    /// Ajoute un fichier (compressé DEFLATE).
    pub fn ajouter(&mut self, nom: &str, donnees: &[u8]) {
        let crc = crc32fast::hash(donnees);
        let mut enc =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(donnees).expect("deflate en memoire");
        let comp = enc.finish().expect("deflate en memoire");
        let offset = self.corps.len() as u32;
        let nom = nom.as_bytes();

        // En-tête local.
        let c = &mut self.corps;
        p32(c, 0x0403_4b50);
        p16(c, 20); // version requise
        p16(c, DRAPEAU_UTF8);
        p16(c, DEFLATE);
        p16(c, HEURE_DOS);
        p16(c, DATE_DOS);
        p32(c, crc);
        p32(c, comp.len() as u32);
        p32(c, donnees.len() as u32);
        p16(c, nom.len() as u16);
        p16(c, 0); // extra
        c.extend_from_slice(nom);
        c.extend_from_slice(&comp);

        // Entrée du répertoire central.
        let r = &mut self.central;
        p32(r, 0x0201_4b50);
        p16(r, 20); // version créatrice
        p16(r, 20); // version requise
        p16(r, DRAPEAU_UTF8);
        p16(r, DEFLATE);
        p16(r, HEURE_DOS);
        p16(r, DATE_DOS);
        p32(r, crc);
        p32(r, comp.len() as u32);
        p32(r, donnees.len() as u32);
        p16(r, nom.len() as u16);
        p16(r, 0); // extra
        p16(r, 0); // commentaire
        p16(r, 0); // disque
        p16(r, 0); // attributs internes
        p32(r, 0); // attributs externes
        p32(r, offset);
        r.extend_from_slice(nom);
        self.entrees += 1;
    }

    /// Ferme l'archive (répertoire central + fin de répertoire).
    pub fn finir(mut self) -> Vec<u8> {
        let debut = self.corps.len() as u32;
        let taille = self.central.len() as u32;
        let central = std::mem::take(&mut self.central);
        let c = &mut self.corps;
        c.extend_from_slice(&central);
        p32(c, 0x0605_4b50);
        p16(c, 0); // ce disque
        p16(c, 0); // disque du répertoire
        p16(c, self.entrees);
        p16(c, self.entrees);
        p32(c, taille);
        p32(c, debut);
        p16(c, 0); // commentaire
        self.corps
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Read;

    /// Relit une archive produite par [`Zip`] : (nom, contenu) par entrée,
    /// en suivant le répertoire central — sert aussi aux tests des formats.
    pub(crate) fn lire(zip: &[u8]) -> Vec<(String, Vec<u8>)> {
        let u16_at = |i: usize| u16::from_le_bytes([zip[i], zip[i + 1]]) as usize;
        let u32_at = |i: usize| u32::from_le_bytes(zip[i..i + 4].try_into().unwrap()) as usize;
        let fin = zip.len() - 22;
        assert_eq!(u32_at(fin), 0x0605_4b50, "fin de repertoire absente");
        let n = u16_at(fin + 10);
        let mut pos = u32_at(fin + 16);
        let mut out = Vec::new();
        for _ in 0..n {
            assert_eq!(u32_at(pos), 0x0201_4b50, "entree centrale invalide");
            let comp = u32_at(pos + 20);
            let taille = u32_at(pos + 24);
            let lnom = u16_at(pos + 28);
            let offset = u32_at(pos + 42);
            let nom = String::from_utf8(zip[pos + 46..pos + 46 + lnom].to_vec()).unwrap();
            assert_eq!(u32_at(offset), 0x0403_4b50, "en-tete local invalide");
            let debut = offset + 30 + u16_at(offset + 26) + u16_at(offset + 28);
            let mut d = flate2::read::DeflateDecoder::new(&zip[debut..debut + comp]);
            let mut contenu = Vec::new();
            d.read_to_end(&mut contenu).unwrap();
            assert_eq!(contenu.len(), taille);
            assert_eq!(crc32fast::hash(&contenu), u32_at(pos + 16) as u32);
            out.push((nom, contenu));
            pos += 46 + lnom + u16_at(pos + 30) + u16_at(pos + 32);
        }
        out
    }

    #[test]
    fn aller_retour_et_determinisme() {
        let fabriquer = || {
            let mut z = Zip::new();
            z.ajouter("[Content_Types].xml", b"<Types/>");
            z.ajouter("dossier/é.txt", "déjà vu".as_bytes());
            z.ajouter("vide.txt", b"");
            z.finir()
        };
        let a = fabriquer();
        assert_eq!(a, fabriquer(), "meme entree = memes octets");
        let e = lire(&a);
        assert_eq!(e.len(), 3);
        assert_eq!(e[0], ("[Content_Types].xml".to_string(), b"<Types/>".to_vec()));
        assert_eq!(e[1].0, "dossier/é.txt");
        assert_eq!(e[1].1, "déjà vu".as_bytes());
        assert!(e[2].1.is_empty());
    }
}
