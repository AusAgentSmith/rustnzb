//! Pure Rust PAR2 verify and repair.
//!
//! The first Rust crate with full PAR2 repair support. Implements GF(2^16)
//! arithmetic with the PAR2-mandated polynomial `0x1100B` and Reed-Solomon
//! decoding for block-level repair.
//!
//! # Usage
//!
//! ```no_run
//! use std::path::Path;
//!
//! let par2_path = Path::new("/downloads/movie/movie.par2");
//! let job_dir = Path::new("/downloads/movie");
//!
//! // Parse the PAR2 index file
//! let file_set = rust_par2::parse(par2_path).unwrap();
//!
//! // Verify all files
//! let result = rust_par2::verify(&file_set, job_dir);
//!
//! if result.all_correct() {
//!     println!("All files intact");
//! } else if result.repair_possible {
//!     // Repair damaged/missing files
//!     let repair = rust_par2::repair(&file_set, job_dir).unwrap();
//!     println!("{repair}");
//! }
//! ```

pub mod gf;
pub mod gf_simd;
pub mod matrix;
mod packets;
pub mod recovery;
pub mod repair;
pub mod types;
mod verify;

pub use packets::{parse_par2_file as parse, parse_par2_reader, ParseError};
pub use types::{
    DamagedFile, Md5Hash, MissingFile, Par2File, Par2FileSet, SliceChecksum, VerifiedFile,
    VerifyResult,
};
pub use repair::{repair, repair_from_verify, RepairError, RepairResult};
pub use verify::{compute_hash_16k, verify};

/// Re-export SIMD functions for benchmarks.
pub mod gf_simd_public {
    pub use crate::gf_simd::{mul_add_buffer, mul_add_multi, xor_buffers};
}
