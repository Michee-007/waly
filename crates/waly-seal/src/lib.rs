//! `waly-seal` — R6a « Huis clos ». Le service scelleur (SYSTEM) et son
//! protocole IPC. Détail : `docs/PLAN-2026-07-20-R6a-huis-clos.md`.
//!
//! - `wfp` : le moteur WFP (porté du banc, par session, fail-closed).
//! - `ipc` : le protocole named-pipe étroit (partagé conceptuellement avec le
//!   client `waly-core::sceau`, qui le ré-implémente en léger côté user).
//! - `service` : SCM + boucle de service + serveur de pipe.
//!
//! Ce crate ne remonte PAS dans waly-core (windows-sys lourd). Le client parle
//! au pipe via `std::fs`.

pub mod enclos;
pub mod ipc;
pub mod tri;

#[cfg(windows)]
pub mod wfp;
#[cfg(windows)]
pub mod service;
#[cfg(windows)]
pub mod regard;

/// Version du service (affichée dans l'état, utile au diagnostic terrain).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
