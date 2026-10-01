//! Capture d'écran native Windows — R5 GATE 1.
//!
//! Deux cibles (cadrage adaptatif du plan R5) :
//!   - `capture_screen()` : l'écran principal entier (« regarde tout mon écran »)
//!   - `capture_active_window()` : la fenêtre au premier plan (défaut, plus privé)
//!
//! Voie GDI (`BitBlt` / `PrintWindow`) via la crate `windows` : ce sont des
//! appels à des DLL système signées (gdi32/user32/dwmapi) chargées par
//! l'exe — JAMAIS un exe tiers (piège 3, la voie prouvée en R4 avec
//! nokhwa/ort). Aucun pixel n'est écrit sur disque ici : la persistance est
//! une décision de l'appelant, et le plan R5 l'interdit (vie privée écran).
//!
//! GDI est le PLANCHER SAC-safe le plus simple. Limite connue : certaines
//! fenêtres GPU/protégées (DRM) peuvent sortir noires — si le terrain le
//! montre, l'étage supérieur est Windows.Graphics.Capture / DXGI Duplication
//! (mêmes DLL, bordure système offerte), à adopter au chantier 0.

/// Un cliché décodé en RGB8 entrelacé (largeur × hauteur × 3).
pub struct Shot {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

impl Shot {
    /// Réduit à `max_w` (côté long, ratio gardé) et encode en JPEG q80 pour le
    /// VLM (≤ 720p = ~950 tok, GATE 3 R5). Même surface que le cliché caméra
    /// (`Percepteur::jpeg`). Retourne (octets JPEG, largeur, hauteur).
    pub fn jpeg(&self, max_w: u32) -> Result<(Vec<u8>, u32, u32), String> {
        let src = image::RgbImage::from_raw(self.width, self.height, self.rgb.clone())
            .ok_or("cliché RGB invalide")?;
        let long = self.width.max(self.height);
        let (w, h) = if long > max_w && long > 0 {
            let s = max_w as f32 / long as f32;
            ((self.width as f32 * s) as u32, (self.height as f32 * s) as u32)
        } else {
            (self.width, self.height)
        };
        let resized =
            image::imageops::resize(&src, w.max(1), h.max(1), image::imageops::FilterType::Triangle);
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 80)
            .encode_image(&resized)
            .map_err(|e| format!("encode JPEG: {e}"))?;
        Ok((jpeg, w.max(1), h.max(1)))
    }
}

#[cfg(windows)]
mod imp {
    use super::Shot;
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        HBITMAP, HDC, HGDIOBJ, SRCCOPY,
    };
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GetForegroundWindow, GetSystemMetrics, GetWindowRect,
        GetWindowTextLengthW, GetWindowTextW, IsWindowVisible, SetForegroundWindow,
        SetWindowDisplayAffinity, SM_CXSCREEN, SM_CYSCREEN, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
    };

    /// Exclut une fenêtre des CAPTURES d'écran (elle reste visible à l'écran mais
    /// n'apparaît pas dans les captures — comme les overlays Copilot/OBS). Sert
    /// à ce que Waly ne se voie pas lui-même : pop-up, cadre ET fenêtre
    /// principale pendant le partage → il voit le travail de Michée.
    pub fn exclude_from_capture(raw: isize) {
        unsafe {
            let hwnd = HWND(raw as *mut core::ffi::c_void);
            let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
        }
    }

    /// Ré-inclut une fenêtre dans les captures (fin du partage).
    pub fn include_in_capture(raw: isize) {
        unsafe {
            let hwnd = HWND(raw as *mut core::ffi::c_void);
            let _ = SetWindowDisplayAffinity(hwnd, WDA_NONE);
        }
    }

    /// À appeler une fois au démarrage du processus : sans elle, sur un écran
    /// mis à l'échelle (125/150 %), GDI capture la résolution LOGIQUE (floue,
    /// plus petite). On veut les pixels réels. Best-effort (ignore l'échec).
    pub fn rendre_conscient_dpi() {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    }

    /// Lit un bitmap GDI (déjà sélectionné dans `mem`) en RGB8, en imposant un
    /// DIB top-down 32 bits (BGRA), puis convertit BGRA → RGB.
    unsafe fn dib_vers_rgb(mem: HDC, hbmp: HBITMAP, w: i32, h: i32) -> Result<Shot, String> {
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // négatif = lignes de haut en bas
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bgra = vec![0u8; (w as usize) * (h as usize) * 4];
        let lignes = GetDIBits(
            mem,
            hbmp,
            0,
            h as u32,
            Some(bgra.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        if lignes == 0 {
            return Err("GetDIBits a échoué".into());
        }
        let mut rgb = vec![0u8; (w as usize) * (h as usize) * 3];
        for (i, px) in bgra.chunks_exact(4).enumerate() {
            rgb[i * 3] = px[2]; // R
            rgb[i * 3 + 1] = px[1]; // G
            rgb[i * 3 + 2] = px[0]; // B
        }
        Ok(Shot { width: w as u32, height: h as u32, rgb })
    }

    /// Capture l'écran principal entier via BitBlt depuis le DC écran.
    pub fn capture_screen() -> Result<Shot, String> {
        unsafe {
            let w = GetSystemMetrics(SM_CXSCREEN);
            let h = GetSystemMetrics(SM_CYSCREEN);
            if w <= 0 || h <= 0 {
                return Err("dimensions écran nulles".into());
            }
            let screen = GetDC(None);
            if screen.is_invalid() {
                return Err("GetDC(écran) a échoué".into());
            }
            let mem = CreateCompatibleDC(Some(screen));
            let hbmp = CreateCompatibleBitmap(screen, w, h);
            let ancien = SelectObject(mem, HGDIOBJ(hbmp.0));
            let res = (|| {
                BitBlt(mem, 0, 0, w, h, Some(screen), 0, 0, SRCCOPY)
                    .map_err(|e| format!("BitBlt écran: {e}"))?;
                dib_vers_rgb(mem, hbmp, w, h)
            })();
            SelectObject(mem, ancien);
            let _ = DeleteObject(HGDIOBJ(hbmp.0));
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            res
        }
    }

    /// Rectangle « cadre étendu » DWM (exclut l'ombre portée) sinon GetWindowRect.
    unsafe fn rect_fenetre(hwnd: HWND) -> RECT {
        let mut r = RECT::default();
        let ok = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut _ as *mut _,
            std::mem::size_of::<RECT>() as u32,
        );
        if ok.is_ok() && r.right > r.left && r.bottom > r.top {
            return r;
        }
        let _ = GetWindowRect(hwnd, &mut r);
        r
    }

    /// Capture la région écran d'une fenêtre (BitBlt — elle doit être visible ;
    /// une fenêtre masquée capturerait ce qui est au-dessus). Cœur partagé.
    unsafe fn capture_hwnd(hwnd: HWND) -> Result<Shot, String> {
        let r = rect_fenetre(hwnd);
        let w = r.right - r.left;
        let h = r.bottom - r.top;
        if w <= 0 || h <= 0 {
            return Err("fenêtre de taille nulle".into());
        }
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("GetDC(écran) a échoué".into());
        }
        let mem = CreateCompatibleDC(Some(screen));
        let hbmp = CreateCompatibleBitmap(screen, w, h);
        let ancien = SelectObject(mem, HGDIOBJ(hbmp.0));
        let res = (|| {
            BitBlt(mem, 0, 0, w, h, Some(screen), r.left, r.top, SRCCOPY)
                .map_err(|e| format!("BitBlt fenêtre: {e}"))?;
            dib_vers_rgb(mem, hbmp, w, h)
        })();
        SelectObject(mem, ancien);
        let _ = DeleteObject(HGDIOBJ(hbmp.0));
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        res
    }

    /// Capture la fenêtre au premier plan.
    pub fn capture_active_window() -> Result<Shot, String> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_invalid() {
                return Err("aucune fenêtre au premier plan".into());
            }
            capture_hwnd(hwnd)
        }
    }

    /// Capture une fenêtre CHOISIE (par son handle). On l'amène au premier plan
    /// d'abord (BitBlt lit l'écran ; une fenêtre derrière donnerait ce qui la
    /// masque). Petit délai pour laisser le compositeur la remonter.
    pub fn capture_window(raw: isize) -> Result<Shot, String> {
        unsafe {
            let hwnd = HWND(raw as *mut core::ffi::c_void);
            let _ = SetForegroundWindow(hwnd);
            let _ = BringWindowToTop(hwnd);
            std::thread::sleep(std::time::Duration::from_millis(180));
            capture_hwnd(hwnd)
        }
    }

    /// Liste les fenêtres de haut niveau VISIBLES qui ont un titre (hors Waly) :
    /// (handle, titre). Pour le sélecteur « quelle fenêtre regarder ? ».
    pub fn list_windows() -> Vec<(isize, String)> {
        let mut out: Vec<(isize, String)> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(enum_cb), LPARAM(&mut out as *mut _ as isize));
        }
        out
    }

    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let out = &mut *(lparam.0 as *mut Vec<(isize, String)>);
        if IsWindowVisible(hwnd).as_bool() {
            let len = GetWindowTextLengthW(hwnd);
            if len > 0 {
                let mut buf = vec![0u16; (len as usize) + 1];
                let n = GetWindowTextW(hwnd, &mut buf);
                if n > 0 {
                    let title = String::from_utf16_lossy(&buf[..n as usize]);
                    let t = title.trim();
                    if !t.is_empty() && t != "Waly" && t != "Waly présence" {
                        out.push((hwnd.0 as isize, t.to_string()));
                    }
                }
            }
        }
        BOOL(1) // continuer l'énumération
    }
}

#[cfg(windows)]
pub use imp::{
    capture_active_window, capture_screen, capture_window, exclude_from_capture,
    include_in_capture, list_windows, rendre_conscient_dpi,
};

// Stub non-Windows : garde `cargo check --workspace` vert sur l'hôte WSL.
#[cfg(not(windows))]
pub fn capture_screen() -> Result<Shot, String> {
    Err("capture d'écran : Windows seulement".into())
}
#[cfg(not(windows))]
pub fn capture_active_window() -> Result<Shot, String> {
    Err("capture d'écran : Windows seulement".into())
}
#[cfg(not(windows))]
pub fn capture_window(_raw: isize) -> Result<Shot, String> {
    Err("capture d'écran : Windows seulement".into())
}
#[cfg(not(windows))]
pub fn list_windows() -> Vec<(isize, String)> {
    Vec::new()
}
#[cfg(not(windows))]
pub fn exclude_from_capture(_raw: isize) {}
#[cfg(not(windows))]
pub fn include_in_capture(_raw: isize) {}
#[cfg(not(windows))]
pub fn rendre_conscient_dpi() {}
