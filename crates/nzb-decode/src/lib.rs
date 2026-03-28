//! Article decoding (yEnc) and file assembly.
//!
//! This crate contains:
//! - `yenc` — yEnc decoder
//! - `cache` — Bounded article cache with disk spill
//! - `assembler` — Write decoded articles into final files

pub mod assembler;
pub mod cache;
pub mod yenc;

pub use assembler::FileAssembler;
pub use cache::ArticleCache;
pub use yenc::{YencDecodeResult, decode_yenc};
