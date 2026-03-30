//! Recovery block reading from PAR2 volume files.
//!
//! Scans all `.par2` files in a directory and extracts RecoverySlice packets,
//! which contain the actual Reed-Solomon recovery data needed for repair.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use tracing::{debug, info, warn};

use crate::packets::{HEADER_SIZE, MAGIC};

/// RecoverySlice packet type identifier.
const TYPE_RECOVERY: &[u8; 16] = b"PAR 2.0\x00RecvSlic";

/// A recovery block extracted from a RecoverySlice packet.
#[derive(Debug)]
pub struct RecoveryBlock {
    /// The exponent (recovery block index) used in the Vandermonde matrix.
    pub exponent: u32,
    /// The recovery data. Length = slice_size.
    pub data: Vec<u8>,
}

/// Load all recovery blocks from PAR2 files in a directory.
///
/// Scans all `.par2` files (index and volumes) for RecoverySlice packets
/// matching the given recovery set ID. Returns blocks sorted by exponent.
pub fn load_recovery_blocks(
    dir: &Path,
    set_id: &[u8; 16],
    slice_size: u64,
) -> Vec<RecoveryBlock> {
    let mut blocks = Vec::new();

    // Find all .par2 files in the directory
    let mut par2_files: Vec<_> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .map_or(false, |ext| ext.eq_ignore_ascii_case("par2"))
            })
            .collect(),
        Err(e) => {
            warn!(error = %e, "Failed to read directory for recovery files");
            return blocks;
        }
    };
    par2_files.sort();

    for par2_path in &par2_files {
        match read_recovery_packets(par2_path, set_id, slice_size) {
            Ok(mut file_blocks) => {
                debug!(
                    file = %par2_path.display(),
                    count = file_blocks.len(),
                    "Loaded recovery blocks"
                );
                blocks.append(&mut file_blocks);
            }
            Err(e) => {
                debug!(
                    file = %par2_path.display(),
                    error = %e,
                    "Skipping file (no recovery blocks or read error)"
                );
            }
        }
    }

    blocks.sort_by_key(|b| b.exponent);

    info!(
        total_blocks = blocks.len(),
        "Recovery blocks loaded"
    );

    blocks
}

/// Read RecoverySlice packets from a single PAR2 file.
fn read_recovery_packets(
    path: &Path,
    set_id: &[u8; 16],
    slice_size: u64,
) -> std::io::Result<Vec<RecoveryBlock>> {
    let mut file = std::fs::File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut blocks = Vec::new();
    let mut pos: u64 = 0;

    let mut header_buf = [0u8; HEADER_SIZE];

    while pos + HEADER_SIZE as u64 <= file_size {
        file.seek(SeekFrom::Start(pos))?;

        // Read packet header
        if file.read_exact(&mut header_buf).is_err() {
            break;
        }

        // Check magic
        if &header_buf[0..8] != MAGIC {
            pos += 4; // Scan forward
            continue;
        }

        // Parse packet length
        let packet_len = u64::from_le_bytes(header_buf[8..16].try_into().unwrap());
        if packet_len < HEADER_SIZE as u64 || packet_len % 4 != 0 {
            pos += 4;
            continue;
        }

        // Check recovery set ID
        let pkt_set_id: [u8; 16] = header_buf[32..48].try_into().unwrap();
        if &pkt_set_id != set_id {
            pos += packet_len;
            continue;
        }

        // Check packet type
        let pkt_type = &header_buf[48..64];
        if pkt_type == TYPE_RECOVERY {
            // RecoverySlice packet body layout:
            // Offset 0..4 (within body): exponent (u32 LE)
            // Offset 4..4+slice_size: recovery data
            let body_len = packet_len - HEADER_SIZE as u64;
            let expected_body = 4 + slice_size;

            if body_len >= expected_body {
                let mut body = vec![0u8; expected_body as usize];
                file.seek(SeekFrom::Start(pos + HEADER_SIZE as u64))?;
                file.read_exact(&mut body)?;

                let exponent = u32::from_le_bytes(body[0..4].try_into().unwrap());
                let data = body[4..4 + slice_size as usize].to_vec();

                blocks.push(RecoveryBlock { exponent, data });
            }
        }

        pos += packet_len;
    }

    Ok(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let set_id = [0u8; 16];
        let blocks = load_recovery_blocks(dir.path(), &set_id, 768000);
        assert!(blocks.is_empty());
    }
}
