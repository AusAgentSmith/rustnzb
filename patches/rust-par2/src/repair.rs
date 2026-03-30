//! PAR2 repair engine.
//!
//! Repairs damaged or missing files using Reed-Solomon recovery data.
//!
//! Algorithm:
//! 1. Verify to identify damaged/missing blocks
//! 2. Load recovery blocks from volume files
//! 3. Build and invert the decode matrix over GF(2^16)
//! 4. Apply the inverse to recovery data to reconstruct original blocks
//! 5. Write repaired blocks back to files

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use rayon::prelude::*;
use tracing::{debug, info};

use crate::gf_simd;
use crate::matrix::GfMatrix;
use crate::recovery::{load_recovery_blocks, RecoveryBlock};
use crate::types::{Par2FileSet, VerifyResult};
use crate::verify;

/// Result of a repair operation.
#[derive(Debug)]
pub struct RepairResult {
    /// Whether the repair succeeded (all files now intact).
    pub success: bool,
    /// Number of blocks repaired.
    pub blocks_repaired: u32,
    /// Number of files repaired.
    pub files_repaired: usize,
    /// Descriptive message.
    pub message: String,
}

impl fmt::Display for RepairResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.success {
            write!(
                f,
                "Repair complete: {} blocks repaired across {} files",
                self.blocks_repaired, self.files_repaired
            )
        } else {
            write!(f, "Repair failed: {}", self.message)
        }
    }
}

/// Errors that can occur during repair.
#[derive(Debug, thiserror::Error)]
pub enum RepairError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Insufficient recovery data: need {needed} blocks, have {available}")]
    InsufficientRecovery { needed: u32, available: u32 },
    #[error("Decode matrix is singular — cannot repair with these recovery blocks")]
    SingularMatrix,
    #[error("No damage detected — nothing to repair")]
    NoDamage,
    #[error("Verification after repair failed: {0}")]
    VerifyFailed(String),
}

/// Repair damaged/missing files in a PAR2 set.
///
/// This is a blocking operation. For async contexts, wrap in `spawn_blocking`.
///
/// Runs verification internally to identify damage. If you already have a
/// [`VerifyResult`] from a prior [`verify()`](crate::verify) call, use
/// [`repair_from_verify`] instead to skip the redundant verification pass.
pub fn repair(file_set: &Par2FileSet, dir: &Path) -> Result<RepairResult, RepairError> {
    let verify_result = verify::verify(file_set, dir);
    repair_from_verify(file_set, dir, &verify_result)
}

/// Repair using a pre-computed [`VerifyResult`].
///
/// This skips the initial verification pass, saving significant time when the
/// caller has already called [`verify()`](crate::verify). The `verify_result`
/// must have been computed against the same `file_set` and `dir`.
///
/// This is a blocking operation. For async contexts, wrap in `spawn_blocking`.
pub fn repair_from_verify(
    file_set: &Par2FileSet,
    dir: &Path,
    verify_result: &VerifyResult,
) -> Result<RepairResult, RepairError> {
    if verify_result.all_correct() {
        return Err(RepairError::NoDamage);
    }

    let blocks_needed = verify_result.blocks_needed();
    info!(
        blocks_needed,
        damaged = verify_result.damaged.len(),
        missing = verify_result.missing.len(),
        "Repair: damage detected"
    );

    // Step 1: Load recovery blocks
    let recovery_blocks =
        load_recovery_blocks(dir, &file_set.recovery_set_id, file_set.slice_size);

    if (recovery_blocks.len() as u32) < blocks_needed {
        return Err(RepairError::InsufficientRecovery {
            needed: blocks_needed,
            available: recovery_blocks.len() as u32,
        });
    }

    // Step 3: Map files to a global block index
    // Each file's blocks are numbered sequentially: file0_block0, file0_block1, ..., file1_block0, ...
    let block_map = build_block_map(file_set);
    let total_input_blocks = block_map.total_blocks as usize;

    // Identify which global block indices are damaged/missing
    let damaged_indices = find_damaged_block_indices(&verify_result, &block_map);
    info!(
        damaged_block_count = damaged_indices.len(),
        total_input_blocks,
        "Mapped damaged blocks to global indices"
    );

    // Step 4: Build the decode matrix
    // Select recovery blocks to use (we need exactly damaged_indices.len())
    let recovery_to_use: Vec<&RecoveryBlock> = recovery_blocks
        .iter()
        .take(damaged_indices.len())
        .collect();

    let recovery_exponents: Vec<u32> = recovery_to_use.iter().map(|b| b.exponent).collect();

    // Build the full encoding matrix, then select the rows we have
    // (intact data rows + selected recovery rows)
    let encoding_matrix =
        GfMatrix::par2_encoding_matrix(total_input_blocks, &recovery_exponents);

    // Build the "available" row selection:
    // For each output position (0..total_input_blocks):
    //   - If the block is intact, use the identity row (row = block_index)
    //   - If the block is damaged, use a recovery row
    let mut available_rows: Vec<usize> = Vec::with_capacity(total_input_blocks);
    let mut recovery_idx = 0;
    for block_idx in 0..total_input_blocks {
        if damaged_indices.contains(&block_idx) {
            // Use recovery row: these are at index total_input_blocks + recovery_idx
            available_rows.push(total_input_blocks + recovery_idx);
            recovery_idx += 1;
        } else {
            // Use identity row (data is intact)
            available_rows.push(block_idx);
        }
    }

    let decode_submatrix = encoding_matrix.select_rows(&available_rows);
    let inverse = decode_submatrix
        .invert()
        .ok_or(RepairError::SingularMatrix)?;

    info!("Decode matrix inverted successfully");

    // Step 5: Reconstruct damaged blocks using streaming I/O + parallel compute.
    //
    // Source-major approach: stream source blocks one at a time through a reader
    // thread, applying each source to ALL damaged-block outputs in parallel.
    // This overlaps I/O with compute (double-buffered via sync_channel) and
    // reduces memory from O(total_blocks × slice) to O(damaged × slice).

    let slice_size = file_set.slice_size as usize;
    let num_damaged = damaged_indices.len();
    let num_sources = total_input_blocks;

    // Coefficient matrix: coeffs[dmg_i][src_idx]
    let coeffs: Vec<Vec<u16>> = (0..num_damaged)
        .map(|dmg_i| {
            (0..num_sources)
                .map(|src_idx| inverse.get(damaged_indices[dmg_i], src_idx))
                .collect()
        })
        .collect();

    // Pre-allocate output buffers (one per damaged block)
    let mut outputs: Vec<Vec<u8>> = (0..num_damaged)
        .map(|_| vec![0u8; slice_size])
        .collect();

    // Stream source blocks via a reader thread, overlap I/O with compute.
    // sync_channel(2) gives double-buffering: reader can be 2 blocks ahead.
    let read_error: Option<std::io::Error> = std::thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::sync_channel::<(usize, Vec<u8>)>(2);

        let reader = scope.spawn({
            let damaged_indices = &damaged_indices;
            let recovery_to_use = &recovery_to_use;
            let block_map = &block_map;
            move || -> Result<(), std::io::Error> {
                let mut recovery_idx = 0usize;
                let mut file_handles: HashMap<String, std::fs::File> = HashMap::new();

                for src_idx in 0..num_sources {
                    let data = if damaged_indices.contains(&src_idx) {
                        let d = recovery_to_use[recovery_idx].data.clone();
                        recovery_idx += 1;
                        d
                    } else {
                        read_source_block(
                            dir,
                            block_map,
                            src_idx,
                            slice_size,
                            &mut file_handles,
                        )?
                    };

                    if tx.send((src_idx, data)).is_err() {
                        break; // Receiver dropped (e.g. panic on compute side)
                    }
                }
                Ok(())
            }
        });

        // Compute: apply each source to all damaged outputs in parallel.
        // The source block (~768KB) stays hot in L3 as rayon threads share it.
        for (src_idx, src_data) in rx {
            outputs
                .par_iter_mut()
                .enumerate()
                .for_each(|(dmg_i, dst)| {
                    let coeff = coeffs[dmg_i][src_idx];
                    if coeff != 0 {
                        gf_simd::mul_add_buffer(dst, &src_data, coeff);
                    }
                });
        }

        // Collect reader result
        match reader.join().unwrap() {
            Ok(()) => None,
            Err(e) => Some(e),
        }
    });

    if let Some(e) = read_error {
        return Err(RepairError::Io(e));
    }

    let repaired_blocks: Vec<(usize, Vec<u8>)> = damaged_indices
        .iter()
        .copied()
        .zip(outputs)
        .collect();

    // Step 6: Write repaired blocks back to files
    let mut files_touched = std::collections::HashSet::new();

    for (global_idx, data) in &repaired_blocks {
        let (filename, file_offset, write_len) =
            block_map.global_to_file(*global_idx, slice_size);

        let file_path = dir.join(&filename);
        debug!(
            filename,
            global_block = global_idx,
            offset = file_offset,
            len = write_len,
            "Writing repaired block"
        );

        // Create file if missing, open for write if exists
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&file_path)?;

        // Ensure file is the right size (for missing files)
        let expected_size = block_map
            .files
            .iter()
            .find(|bf| bf.filename == filename)
            .map(|bf| bf.file_size)
            .unwrap_or(0);
        let current_size = f.metadata()?.len();
        if current_size < expected_size {
            f.set_len(expected_size)?;
        }

        f.seek(SeekFrom::Start(file_offset as u64))?;
        f.write_all(&data[..write_len])?;
        files_touched.insert(filename.clone());
    }

    // Step 7: Re-verify
    let re_verify = verify::verify(file_set, dir);
    if re_verify.all_correct() {
        info!(
            blocks = repaired_blocks.len(),
            files = files_touched.len(),
            "Repair successful — all files verified"
        );
        Ok(RepairResult {
            success: true,
            blocks_repaired: repaired_blocks.len() as u32,
            files_repaired: files_touched.len(),
            message: "All files repaired and verified".to_string(),
        })
    } else {
        Err(RepairError::VerifyFailed(format!("{re_verify}")))
    }
}

// ---------------------------------------------------------------------------
// Block mapping
// ---------------------------------------------------------------------------

/// Maps between global block indices and per-file block positions.
struct BlockMap {
    files: Vec<BlockFile>,
    total_blocks: u32,
}

struct BlockFile {
    filename: String,
    file_size: u64,
    block_count: u32,
    /// First global block index for this file.
    start_block: u32,
}

fn build_block_map(file_set: &Par2FileSet) -> BlockMap {
    let slice_size = file_set.slice_size;
    let mut files = Vec::new();
    let mut block_offset = 0u32;

    // Sort files by file ID for deterministic ordering (same as par2cmdline)
    let mut sorted_files: Vec<_> = file_set.files.values().collect();
    sorted_files.sort_by_key(|f| f.file_id);

    for f in sorted_files {
        let block_count = if slice_size == 0 {
            0
        } else {
            ((f.size + slice_size - 1) / slice_size) as u32
        };
        files.push(BlockFile {
            filename: f.filename.clone(),
            file_size: f.size,
            block_count,
            start_block: block_offset,
        });
        block_offset += block_count;
    }

    BlockMap {
        files,
        total_blocks: block_offset,
    }
}

impl BlockMap {
    /// Convert a global block index to (filename, file_byte_offset, bytes_to_write).
    fn global_to_file(&self, global_idx: usize, slice_size: usize) -> (String, usize, usize) {
        let global = global_idx as u32;
        for f in &self.files {
            if global >= f.start_block && global < f.start_block + f.block_count {
                let local_block = (global - f.start_block) as usize;
                let file_offset = local_block * slice_size;
                // Last block may be shorter than slice_size
                let remaining = f.file_size as usize - file_offset;
                let write_len = remaining.min(slice_size);
                return (f.filename.clone(), file_offset, write_len);
            }
        }
        panic!("Global block index {global_idx} out of range");
    }
}

fn find_damaged_block_indices(verify_result: &VerifyResult, block_map: &BlockMap) -> Vec<usize> {
    let mut indices = Vec::new();

    for damaged in &verify_result.damaged {
        if let Some(bf) = block_map.files.iter().find(|f| f.filename == damaged.filename) {
            if damaged.damaged_block_indices.is_empty() {
                // No per-block info — assume all blocks damaged
                for i in 0..bf.block_count {
                    indices.push((bf.start_block + i) as usize);
                }
            } else {
                // Use precise per-block damage info
                for &local_idx in &damaged.damaged_block_indices {
                    indices.push((bf.start_block + local_idx) as usize);
                }
            }
        }
    }

    for missing in &verify_result.missing {
        if let Some(bf) = block_map.files.iter().find(|f| f.filename == missing.filename) {
            for i in 0..bf.block_count {
                indices.push((bf.start_block + i) as usize);
            }
        }
    }

    indices.sort();
    indices.dedup();
    indices
}

/// Read a single source block from disk, reusing file handles.
fn read_source_block(
    dir: &Path,
    block_map: &BlockMap,
    global_idx: usize,
    slice_size: usize,
    file_handles: &mut HashMap<String, std::fs::File>,
) -> std::io::Result<Vec<u8>> {
    let (filename, file_offset, _) = block_map.global_to_file(global_idx, slice_size);

    let handle = match file_handles.entry(filename.clone()) {
        Entry::Occupied(e) => e.into_mut(),
        Entry::Vacant(e) => {
            let path = dir.join(&filename);
            e.insert(std::fs::File::open(&path)?)
        }
    };
    handle.seek(SeekFrom::Start(file_offset as u64))?;

    let mut buf = vec![0u8; slice_size]; // zero-initialized for last-block padding
    let mut total = 0;
    while total < slice_size {
        match handle.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gf;

    /// Test that the basic RS encode→decode round-trip works.
    /// 2 data blocks, 2 recovery blocks, lose both data blocks, recover.
    #[test]
    fn test_rs_roundtrip_simple() {
        // 2 input "blocks" of 4 bytes each (2 u16 values)
        let input0: Vec<u8> = vec![0x01, 0x00, 0x02, 0x00]; // [1, 2] as u16 LE
        let input1: Vec<u8> = vec![0x03, 0x00, 0x04, 0x00]; // [3, 4] as u16 LE

        let input_count = 2;
        let recovery_exponents = vec![0u32, 1u32];

        // Build encoding matrix
        let enc = GfMatrix::par2_encoding_matrix(input_count, &recovery_exponents);
        // enc is 4x2:
        // Row 0: [1, 0]  (identity for input 0)
        // Row 1: [0, 1]  (identity for input 1)
        // Row 2: [2^(0*0), 2^(0*1)] = [1, 1]  (recovery exp=0)
        // Row 3: [2^(1*0), 2^(1*1)] = [1, 2]  (recovery exp=1)

        println!("Encoding matrix:");
        for r in 0..enc.rows {
            for c in 0..enc.cols {
                print!("{:5} ", enc.get(r, c));
            }
            println!();
        }

        // Compute recovery blocks: for each u16 position, recovery[e] = Σ input[i] * enc[e+2][i]
        let slice_size = 4;
        let u16_per_slice = slice_size / 2;
        let inputs = [&input0, &input1];

        let mut recovery0 = vec![0u8; slice_size];
        let mut recovery1 = vec![0u8; slice_size];

        for pos in 0..u16_per_slice {
            let off = pos * 2;
            let mut r0: u16 = 0;
            let mut r1: u16 = 0;
            for (i, inp) in inputs.iter().enumerate() {
                let val = u16::from_le_bytes([inp[off], inp[off + 1]]);
                r0 = gf::add(r0, gf::mul(enc.get(2, i), val));
                r1 = gf::add(r1, gf::mul(enc.get(3, i), val));
            }
            recovery0[off] = r0 as u8;
            recovery0[off + 1] = (r0 >> 8) as u8;
            recovery1[off] = r1 as u8;
            recovery1[off + 1] = (r1 >> 8) as u8;
        }

        println!("Recovery 0: {:?}", recovery0);
        println!("Recovery 1: {:?}", recovery1);

        // Now "lose" both input blocks. We have recovery rows 2 and 3.
        // Select those rows and invert to decode.
        let decode_sub = enc.select_rows(&[2, 3]);
        println!("Decode submatrix:");
        for r in 0..decode_sub.rows {
            for c in 0..decode_sub.cols {
                print!("{:5} ", decode_sub.get(r, c));
            }
            println!();
        }

        let inv = decode_sub.invert().expect("Should be invertible");
        println!("Inverse:");
        for r in 0..inv.rows {
            for c in 0..inv.cols {
                print!("{:5} ", inv.get(r, c));
            }
            println!();
        }

        // Reconstruct: for each u16 position
        let available = [&recovery0, &recovery1];
        let mut result0 = vec![0u8; slice_size];
        let mut result1 = vec![0u8; slice_size];

        for pos in 0..u16_per_slice {
            let off = pos * 2;
            let mut out0: u16 = 0;
            let mut out1: u16 = 0;
            for (src_idx, src) in available.iter().enumerate() {
                let val = u16::from_le_bytes([src[off], src[off + 1]]);
                out0 = gf::add(out0, gf::mul(inv.get(0, src_idx), val));
                out1 = gf::add(out1, gf::mul(inv.get(1, src_idx), val));
            }
            result0[off] = out0 as u8;
            result0[off + 1] = (out0 >> 8) as u8;
            result1[off] = out1 as u8;
            result1[off + 1] = (out1 >> 8) as u8;
        }

        assert_eq!(result0, input0, "Recovered block 0 should match original");
        assert_eq!(result1, input1, "Recovered block 1 should match original");
    }
}
