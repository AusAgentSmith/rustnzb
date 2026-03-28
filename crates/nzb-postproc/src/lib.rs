//! Post-processing pipeline: par2 verify/repair, RAR/7z/ZIP extraction, cleanup.
//!
//! This crate contains:
//! - `detect` — File detection helpers (par2, RAR, 7z, ZIP, cleanup candidates)
//! - `par2` — Native PAR2 verify/repair via `rust-par2`
//! - `unpack` — RAR extraction (unrar), 7z (7z binary), ZIP (zip crate)
//! - `pipeline` — Orchestrate: verify -> repair -> extract -> cleanup

pub mod detect;
pub mod par2;
pub mod pipeline;
pub mod unpack;

pub use detect::ArchiveType;
pub use pipeline::{PostProcConfig, PostProcResult, run_pipeline};
