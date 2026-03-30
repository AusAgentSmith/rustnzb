//! Data types for PAR2 file sets.

use std::collections::HashMap;
use std::fmt;

/// 16-byte MD5 hash.
pub type Md5Hash = [u8; 16];

/// 16-byte identifier (Recovery Set ID, File ID, etc.).
pub type Id16 = [u8; 16];

/// A parsed PAR2 file set containing all metadata needed for verification.
#[derive(Debug, Clone)]
pub struct Par2FileSet {
    /// Recovery Set ID — all packets in a set share this.
    pub recovery_set_id: Id16,
    /// Slice (block) size in bytes.
    pub slice_size: u64,
    /// Files described in this PAR2 set, keyed by File ID.
    pub files: HashMap<Id16, Par2File>,
    /// Number of recovery slices available (counted from RecoverySlice packets).
    pub recovery_block_count: u32,
    /// Creator software string, if present.
    pub creator: Option<String>,
}

/// Metadata for a single file in the PAR2 set.
#[derive(Debug, Clone)]
pub struct Par2File {
    /// File ID (MD5 of hash16k + hash + file_id internal data).
    pub file_id: Id16,
    /// Full file MD5 hash.
    pub hash: Md5Hash,
    /// MD5 hash of the first 16 KiB of the file.
    pub hash_16k: Md5Hash,
    /// File size in bytes.
    pub size: u64,
    /// Filename (from the PAR2 packet, UTF-8 or best-effort decoded).
    pub filename: String,
    /// Per-slice checksums (MD5 + CRC32), in order. From IFSC packets.
    pub slices: Vec<SliceChecksum>,
}

/// Checksum data for a single slice (block) of a file.
#[derive(Debug, Clone, Copy)]
pub struct SliceChecksum {
    /// MD5 hash of this slice.
    pub md5: Md5Hash,
    /// CRC32 of this slice (the full slice, zero-padded if it's the last partial slice).
    pub crc32: u32,
}

/// Result of verifying a PAR2 file set against actual files on disk.
#[derive(Debug)]
pub struct VerifyResult {
    /// Files that are intact (MD5 matches).
    pub intact: Vec<VerifiedFile>,
    /// Files that are damaged (exist but MD5 doesn't match).
    pub damaged: Vec<DamagedFile>,
    /// Files that are missing entirely.
    pub missing: Vec<MissingFile>,
    /// Total number of recovery blocks available in the PAR2 set.
    pub recovery_blocks_available: u32,
    /// Whether repair is theoretically possible (enough recovery blocks).
    pub repair_possible: bool,
}

impl VerifyResult {
    /// Returns true if all files are intact.
    pub fn all_correct(&self) -> bool {
        self.damaged.is_empty() && self.missing.is_empty()
    }

    /// Total number of damaged/missing blocks that need repair.
    pub fn blocks_needed(&self) -> u32 {
        let damaged_blocks: u32 = self.damaged.iter().map(|d| d.damaged_block_count).sum();
        let missing_blocks: u32 = self.missing.iter().map(|m| m.block_count).sum();
        damaged_blocks + missing_blocks
    }
}

/// A file that passed verification.
#[derive(Debug)]
pub struct VerifiedFile {
    pub filename: String,
    pub size: u64,
}

/// A file that exists but has damage.
#[derive(Debug)]
pub struct DamagedFile {
    pub filename: String,
    pub size: u64,
    /// Number of blocks in this file that are damaged.
    pub damaged_block_count: u32,
    /// Total blocks in this file.
    pub total_block_count: u32,
    /// Indices of the specific damaged blocks within this file (0-based).
    pub damaged_block_indices: Vec<u32>,
}

/// A file that is missing entirely.
#[derive(Debug)]
pub struct MissingFile {
    pub filename: String,
    pub expected_size: u64,
    /// Number of blocks this file contributes to the repair requirement.
    pub block_count: u32,
}

impl fmt::Display for VerifyResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.all_correct() {
            write!(f, "All {} files correct", self.intact.len())
        } else {
            write!(
                f,
                "{} intact, {} damaged, {} missing — {} blocks needed, {} available",
                self.intact.len(),
                self.damaged.len(),
                self.missing.len(),
                self.blocks_needed(),
                self.recovery_blocks_available,
            )
        }
    }
}
