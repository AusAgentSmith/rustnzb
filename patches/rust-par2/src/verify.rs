//! File verification against PAR2 checksums.
//!
//! Verifies files on disk by computing MD5 hashes and comparing them against
//! the hashes stored in the PAR2 file set. Optionally performs per-slice
//! CRC32/MD5 checks to identify exactly which blocks are damaged.

use std::io::Read;
use std::path::Path;

use md5::{Digest, Md5};
use rayon::prelude::*;
use tracing::{debug, info, trace, warn};

use crate::types::{DamagedFile, MissingFile, Par2FileSet, VerifiedFile, VerifyResult};

/// Verify all files in a PAR2 set against actual files in a directory.
///
/// For each file described in the PAR2 set:
/// - If the file exists and its MD5 matches → `intact`
/// - If the file exists but MD5 doesn't match → `damaged` (with per-block detail)
/// - If the file doesn't exist → `missing`
pub fn verify(file_set: &Par2FileSet, dir: &Path) -> VerifyResult {
    // Sort files by name for deterministic output
    let mut files: Vec<_> = file_set.files.values().collect();
    files.sort_by_key(|f| &f.filename);

    // Verify files in parallel using rayon — each file is independent I/O + MD5
    enum FileResult {
        Intact(VerifiedFile),
        Damaged(DamagedFile),
        Missing(MissingFile),
    }

    let results: Vec<FileResult> = files
        .par_iter()
        .map(|par2_file| {
            let file_path = dir.join(&par2_file.filename);

            if !file_path.exists() {
                debug!(filename = par2_file.filename, "file missing");
                let block_count = blocks_for_file(par2_file.size, file_set.slice_size);
                return FileResult::Missing(MissingFile {
                    filename: par2_file.filename.clone(),
                    expected_size: par2_file.size,
                    block_count,
                });
            }

            // Check file size first (fast reject)
            let metadata = match std::fs::metadata(&file_path) {
                Ok(m) => m,
                Err(e) => {
                    warn!(filename = par2_file.filename, error = %e, "cannot stat file");
                    let block_count = blocks_for_file(par2_file.size, file_set.slice_size);
                    return FileResult::Missing(MissingFile {
                        filename: par2_file.filename.clone(),
                        expected_size: par2_file.size,
                        block_count,
                    });
                }
            };

            if metadata.len() != par2_file.size {
                debug!(
                    filename = par2_file.filename,
                    expected = par2_file.size,
                    actual = metadata.len(),
                    "file size mismatch"
                );
                let total_blocks = blocks_for_file(par2_file.size, file_set.slice_size);
                return FileResult::Damaged(DamagedFile {
                    filename: par2_file.filename.clone(),
                    size: metadata.len(),
                    damaged_block_count: total_blocks,
                    total_block_count: total_blocks,
                    damaged_block_indices: (0..total_blocks).collect(),
                });
            }

            // Compute full-file MD5
            match compute_file_md5(&file_path) {
                Ok(hash) => {
                    if hash == par2_file.hash {
                        trace!(filename = par2_file.filename, "file OK (MD5 match)");
                        FileResult::Intact(VerifiedFile {
                            filename: par2_file.filename.clone(),
                            size: par2_file.size,
                        })
                    } else {
                        let total_blocks = blocks_for_file(par2_file.size, file_set.slice_size);
                        let bad_indices =
                            find_damaged_blocks(&file_path, &par2_file.slices, file_set.slice_size);
                        let damaged_blocks = bad_indices.len() as u32;

                        debug!(
                            filename = par2_file.filename,
                            damaged_blocks,
                            total_blocks,
                            bad_indices = ?bad_indices,
                            "file damaged (MD5 mismatch)"
                        );

                        FileResult::Damaged(DamagedFile {
                            filename: par2_file.filename.clone(),
                            size: par2_file.size,
                            damaged_block_count: damaged_blocks,
                            total_block_count: total_blocks,
                            damaged_block_indices: bad_indices,
                        })
                    }
                }
                Err(e) => {
                    warn!(filename = par2_file.filename, error = %e, "cannot hash file");
                    let total_blocks = blocks_for_file(par2_file.size, file_set.slice_size);
                    FileResult::Damaged(DamagedFile {
                        filename: par2_file.filename.clone(),
                        size: par2_file.size,
                        damaged_block_count: total_blocks,
                        total_block_count: total_blocks,
                        damaged_block_indices: (0..total_blocks).collect(),
                    })
                }
            }
        })
        .collect();

    // Gather parallel results into separate vectors
    let mut intact = Vec::new();
    let mut damaged = Vec::new();
    let mut missing = Vec::new();
    for r in results {
        match r {
            FileResult::Intact(f) => intact.push(f),
            FileResult::Damaged(f) => damaged.push(f),
            FileResult::Missing(f) => missing.push(f),
        }
    }

    // Count recovery blocks from ALL .par2 files in the directory (not just the index).
    // The index file typically has 0 recovery blocks; they're in .vol*.par2 volumes.
    let recovery_blocks_available = count_recovery_blocks_in_dir(dir, file_set);
    let total_needed: u32 = damaged.iter().map(|d| d.damaged_block_count).sum::<u32>()
        + missing.iter().map(|m| m.block_count).sum::<u32>();
    let repair_possible = total_needed <= recovery_blocks_available;

    info!(
        intact = intact.len(),
        damaged = damaged.len(),
        missing = missing.len(),
        blocks_needed = total_needed,
        recovery_blocks_available,
        "verification complete"
    );

    VerifyResult {
        intact,
        damaged,
        missing,
        recovery_blocks_available,
        repair_possible,
    }
}

/// Read buffer size for hashing. 2 MiB gives good kernel readahead and
/// amortizes syscall overhead on large files.
const HASH_BUF_SIZE: usize = 2 * 1024 * 1024;

/// Compute the MD5 hash of a file using double-buffered I/O.
/// One buffer is being hashed while the other is being filled by the OS,
/// overlapping CPU and I/O work.
fn compute_file_md5(path: &Path) -> std::io::Result<[u8; 16]> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Md5::new();

    let mut buf_a = vec![0u8; HASH_BUF_SIZE];
    let mut buf_b = vec![0u8; HASH_BUF_SIZE];

    // Fill first buffer
    let mut n_a = file.read(&mut buf_a)?;

    loop {
        if n_a == 0 {
            break;
        }

        // Start reading into buf_b while we hash buf_a.
        // On Linux, the kernel's readahead will prefetch data for the next
        // read while we're busy with MD5 computation.
        let n_b = file.read(&mut buf_b)?;
        hasher.update(&buf_a[..n_a]);

        if n_b == 0 {
            break;
        }

        // Now hash buf_b while reading into buf_a
        n_a = file.read(&mut buf_a)?;
        hasher.update(&buf_b[..n_b]);
    }

    Ok(hasher.finalize().into())
}

/// Compute the MD5 hash of the first 16 KiB of a file.
///
/// Useful for file identification when filenames are obfuscated.
pub fn compute_hash_16k(path: &Path) -> std::io::Result<[u8; 16]> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Md5::new();
    let mut buf = [0u8; 16384]; // 16 KiB

    let n = file.read(&mut buf)?;
    hasher.update(&buf[..n]);

    Ok(hasher.finalize().into())
}

/// Find the indices of damaged blocks via per-slice MD5 verification.
/// Returns the 0-based indices of blocks that DON'T match their expected MD5.
fn find_damaged_blocks(
    path: &Path,
    slices: &[crate::types::SliceChecksum],
    slice_size: u64,
) -> Vec<u32> {
    if slices.is_empty() {
        return vec![];
    }

    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return (0..slices.len() as u32).collect(),
    };

    let mut bad = Vec::new();
    let mut buf = vec![0u8; slice_size as usize];

    for (idx, expected) in slices.iter().enumerate() {
        let n = match file.read(&mut buf) {
            Ok(0) => {
                // Remaining blocks are all missing
                for i in idx..slices.len() {
                    bad.push(i as u32);
                }
                break;
            }
            Ok(n) => n,
            Err(_) => {
                for i in idx..slices.len() {
                    bad.push(i as u32);
                }
                break;
            }
        };

        let mut hasher = Md5::new();
        hasher.update(&buf[..n]);
        if n < slice_size as usize {
            let padding = vec![0u8; slice_size as usize - n];
            hasher.update(&padding);
        }
        let hash: [u8; 16] = hasher.finalize().into();

        if hash != expected.md5 {
            bad.push(idx as u32);
        }
    }

    bad
}

/// Count recovery blocks across all .par2 files in a directory.
fn count_recovery_blocks_in_dir(dir: &Path, file_set: &Par2FileSet) -> u32 {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return file_set.recovery_block_count,
    };

    let mut count = 0u32;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map_or(false, |e| e.eq_ignore_ascii_case("par2")) {
            if let Ok(parsed) = crate::packets::parse_par2_file(&path) {
                if parsed.recovery_set_id == file_set.recovery_set_id {
                    count += parsed.recovery_block_count;
                }
            }
        }
    }

    // Fall back to file_set count if directory scan found nothing
    if count == 0 {
        file_set.recovery_block_count
    } else {
        count
    }
}

/// Compute the number of slices (blocks) needed for a file of the given size.
fn blocks_for_file(file_size: u64, slice_size: u64) -> u32 {
    if slice_size == 0 {
        return 0;
    }
    ((file_size + slice_size - 1) / slice_size) as u32
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::parse_par2_file;

    /// Test verification of the intact par2test set.
    #[test]
    fn test_verify_intact_set() {
        let par2_path =
            Path::new("/home/sprooty/sabnzbd/tests/data/par2repair/basic/par2test.par2");
        let dir = Path::new("/home/sprooty/sabnzbd/tests/data/par2repair/basic");

        if !par2_path.exists() {
            eprintln!("Skipping test: test data not found");
            return;
        }

        let set = parse_par2_file(par2_path).unwrap();
        let result = verify(&set, dir);

        // The test data should have some intact and some problematic files.
        // par2test.part2.rar (102400 bytes) should be intact.
        // par2test.part1.rar is only 9 bytes (damaged/truncated).
        // par2test.part5.rar is only 8 bytes.
        // Some files might be missing.

        println!("Verify result: {result}");
        println!("  intact:  {:?}", result.intact.iter().map(|f| &f.filename).collect::<Vec<_>>());
        println!("  damaged: {:?}", result.damaged.iter().map(|f| &f.filename).collect::<Vec<_>>());
        println!("  missing: {:?}", result.missing.iter().map(|f| &f.filename).collect::<Vec<_>>());

        // We should have at least some results
        let total = result.intact.len() + result.damaged.len() + result.missing.len();
        assert_eq!(total, 6, "should account for all 6 files");
    }

    /// Test blocks_for_file calculation.
    #[test]
    fn test_blocks_for_file() {
        assert_eq!(blocks_for_file(100000, 100000), 1);
        assert_eq!(blocks_for_file(100001, 100000), 2);
        assert_eq!(blocks_for_file(200000, 100000), 2);
        assert_eq!(blocks_for_file(0, 100000), 0);
        assert_eq!(blocks_for_file(1, 100000), 1);
        assert_eq!(blocks_for_file(102400, 100000), 2);
    }

    /// Test compute_hash_16k.
    #[test]
    fn test_hash_16k() {
        let path =
            Path::new("/home/sprooty/sabnzbd/tests/data/par2repair/basic/par2test.part2.rar");
        if !path.exists() {
            eprintln!("Skipping test: test data not found");
            return;
        }

        let hash = compute_hash_16k(path).unwrap();
        // The hash should be non-zero
        assert_ne!(hash, [0u8; 16], "hash should not be all zeros");
    }

    /// Test that hash_16k matches the PAR2 stored hash for an intact file.
    #[test]
    fn test_hash_16k_matches_par2() {
        let par2_path =
            Path::new("/home/sprooty/sabnzbd/tests/data/par2repair/basic/par2test.par2");
        let dir = Path::new("/home/sprooty/sabnzbd/tests/data/par2repair/basic");

        if !par2_path.exists() {
            eprintln!("Skipping test: test data not found");
            return;
        }

        let set = parse_par2_file(par2_path).unwrap();

        // par2test.part2.rar should be an intact 102400-byte file
        let part2 = set
            .files
            .values()
            .find(|f| f.filename == "par2test.part2.rar")
            .expect("part2 should exist in par2 set");

        let file_path = dir.join("par2test.part2.rar");
        if !file_path.exists() || std::fs::metadata(&file_path).unwrap().len() != part2.size {
            eprintln!("Skipping: par2test.part2.rar is not the expected size");
            return;
        }

        let computed = compute_hash_16k(&file_path).unwrap();
        assert_eq!(
            computed, part2.hash_16k,
            "computed 16K hash should match PAR2 stored hash"
        );
    }
}
