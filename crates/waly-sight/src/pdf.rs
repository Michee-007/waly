//! Rendu PDF natif Windows pour l'aperçu DANS l'app (UI 2026-09-14) :
//! `Windows.Data.Pdf` (WinRT système, signé Microsoft → SAC-safe, piège 3),
//! le même moteur que la preuve des documents générés (`lab/documents-banc`).
//! Chaque page devient un PNG en mémoire ; rien n'est écrit sur disque.

use std::path::Path;

/// Rend au plus `max` pages à `largeur` pixels. Retourne (PNG par page,
/// nombre total de pages du document).
#[cfg(windows)]
pub fn pages_png(chemin: &Path, max: u32, largeur: u32) -> Result<(Vec<Vec<u8>>, u32), String> {
    use windows::core::HSTRING;
    use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
    use windows::Storage::StorageFile;
    use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};

    let e = |x: windows::core::Error| format!("PDF illisible : {x}");
    // Un chemin canonicalisé porte le préfixe verbatim `\\?\` : WinRT le
    // refuse (« path too long », vécu 2026-09-14) → chemin Win32 classique.
    let brut = chemin.to_string_lossy();
    let classique = brut.strip_prefix(r"\\?\").unwrap_or(&brut);
    let fichier = StorageFile::GetFileFromPathAsync(&HSTRING::from(classique))
        .map_err(e)?
        .get()
        .map_err(e)?;
    let doc = PdfDocument::LoadFromFileAsync(&fichier).map_err(e)?.get().map_err(e)?;
    let total = doc.PageCount().map_err(e)?;
    let mut pages = Vec::new();
    for i in 0..total.min(max) {
        let page = doc.GetPage(i).map_err(e)?;
        let flux = InMemoryRandomAccessStream::new().map_err(e)?;
        let options = PdfPageRenderOptions::new().map_err(e)?;
        options.SetDestinationWidth(largeur).map_err(e)?;
        page.RenderWithOptionsToStreamAsync(&flux, &options).map_err(e)?.get().map_err(e)?;
        let taille = flux.Size().map_err(e)? as u32;
        let lecteur = DataReader::CreateDataReader(&flux.GetInputStreamAt(0).map_err(e)?).map_err(e)?;
        lecteur.LoadAsync(taille).map_err(e)?.get().map_err(e)?;
        let mut octets = vec![0u8; taille as usize];
        lecteur.ReadBytes(&mut octets).map_err(e)?;
        pages.push(octets);
    }
    Ok((pages, total))
}

#[cfg(not(windows))]
pub fn pages_png(_chemin: &Path, _max: u32, _largeur: u32) -> Result<(Vec<Vec<u8>>, u32), String> {
    Err("aperçu PDF : Windows seulement".into())
}

/// Décode une image (PNG/JPEG) en cliché RGB8 — pour l'OCR ou le VLM.
pub fn shot_depuis_octets(octets: &[u8]) -> Result<crate::screen::Shot, String> {
    let img = image::load_from_memory(octets)
        .map_err(|e| format!("image illisible : {e}"))?
        .to_rgb8();
    let (width, height) = img.dimensions();
    Ok(crate::screen::Shot { width, height, rgb: img.into_raw() })
}

/// Texte d'un PDF joint (lot 1, 2026-09-30) : `Windows.Data.Pdf` ne donne
/// pas le texte -> chaque page est rendue puis lue par l'OCR local (le même
/// qu'au mode Écran). Au plus `max` pages. Retourne (texte, pages lues,
/// pages totales). Rien n'est écrit sur disque.
pub fn texte_ocr(chemin: &Path, max: u32) -> Result<(String, u32, u32), String> {
    let (pages, total) = pages_png(chemin, max, 1400)?;
    let mut ocr = crate::ocr::Ocr::load_default()?;
    let mut texte = String::new();
    for (i, png) in pages.iter().enumerate() {
        let shot = shot_depuis_octets(png)?;
        let lu = ocr.read(&shot)?;
        if pages.len() > 1 {
            texte.push_str(&format!("--- page {} ---\n", i + 1));
        }
        texte.push_str(lu.text.trim());
        texte.push('\n');
    }
    Ok((texte, pages.len() as u32, total))
}
