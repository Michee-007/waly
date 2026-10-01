//! waly-sight — l'œil de Waly : caméra (R4, mode appel) puis écran (R5).
//!
//! Deux étages (plan R4) : la boucle rapide (petits ONNX CPU, < 500 ms,
//! jamais le VLM) vit ici ; le moment VLM ponctuel passe par waly-core.
//! Règle vie privée : aucune frame persistée hors demande explicite.

#[cfg(windows)]
pub mod camera;
pub mod demo;
pub mod face;
#[cfg(feature = "mains")]
pub mod mains;
pub mod ocr;
pub mod pdf;
pub mod perception;
pub mod screen;
pub mod uia;
