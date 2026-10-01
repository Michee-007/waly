//! Capture caméra native Windows (Media Foundation via nokhwa) — R4 GATE C.
//!
//! Une frame = RGB8 entrelacé (largeur × hauteur × 3). Le décodage
//! YUY2/NV12/MJPG → RGB est fait par nokhwa. Aucune frame n'est écrite sur
//! disque ici : la persistance est une décision de l'appelant (règle vie
//! privée du plan R4).

use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{
    ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType,
    Resolution,
};
use nokhwa::Camera;

/// Une frame décodée en RGB8.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// Énumère les caméras vues par Media Foundation.
pub fn list() -> Result<Vec<String>, String> {
    let cams = nokhwa::query(ApiBackend::MediaFoundation)
        .map_err(|e| format!("énumération caméras: {e}"))?;
    Ok(cams
        .iter()
        .map(|c| format!("{} — {} ({})", c.index(), c.human_name(), c.description()))
        .collect())
}

pub struct Cam {
    inner: Camera,
}

impl Cam {
    /// Ouvre la caméra `index` en ~640×480@30 (la résolution de la boucle de
    /// perception : décoder du full-HD coûte ~6× plus cher pour rien).
    /// Essaie NV12 puis MJPG puis YUY2 — `Closest` de nokhwa exige le FourCC
    /// exact et les caméras n'exposent pas toutes les mêmes.
    pub fn open(index: u32) -> Result<Self, String> {
        Self::open_at(index, 640, 480, 30)
    }

    pub fn open_at(index: u32, w: u32, h: u32, fps: u32) -> Result<Self, String> {
        let mut last_err = String::new();
        for four_cc in [FrameFormat::NV12, FrameFormat::MJPEG, FrameFormat::YUYV] {
            let requested = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(
                CameraFormat::new(Resolution::new(w, h), four_cc, fps),
            ));
            match Camera::new(CameraIndex::Index(index), requested) {
                Ok(mut inner) => match inner.open_stream() {
                    Ok(()) => return Ok(Self { inner }),
                    Err(e) => last_err = format!("flux {four_cc:?}: {e}"),
                },
                Err(e) => last_err = format!("ouverture {four_cc:?}: {e}"),
            }
        }
        Err(format!("caméra {index}: aucun format accepté ({last_err})"))
    }

    /// Format négocié (résolution, fps, FourCC source).
    pub fn format(&self) -> String {
        format!("{}", self.inner.camera_format())
    }

    /// Capture et décode une frame.
    pub fn grab(&mut self) -> Result<Frame, String> {
        let buf = self.inner.frame().map_err(|e| format!("frame: {e}"))?;
        let img = buf
            .decode_image::<RgbFormat>()
            .map_err(|e| format!("décodage frame: {e}"))?;
        Ok(Frame { width: img.width(), height: img.height(), rgb: img.into_raw() })
    }
}
