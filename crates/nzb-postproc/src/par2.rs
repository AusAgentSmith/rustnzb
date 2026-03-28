//! Par2 verification and repair via pure-Rust `rust-par2` library.
//!
//! No external binary or subprocess is needed — all PAR2 parsing, verification,
//! and Reed-Solomon repair runs natively in-process.

use std::path::Path;

use tracing::{info, warn};

/// Result of a par2 verify/repair operation.
#[derive(Debug)]
pub struct Par2Result {
    pub success: bool,
    pub blocks_needed: u32,
    pub blocks_available: u32,
    pub repaired: bool,
    pub message: String,
}

/// Repair damaged/missing files using native Rust PAR2 repair.
///
/// This runs verification internally and then repairs if needed.
/// Blocking — call from `spawn_blocking` in async contexts.
pub fn par2_repair_blocking(par2_file: &Path) -> anyhow::Result<Par2Result> {
    let dir = par2_file.parent().unwrap_or(Path::new("."));

    let file_set = rust_par2::parse(par2_file).map_err(|e| anyhow::anyhow!("PAR2 parse: {e}"))?;
    let verify_result = rust_par2::verify(&file_set, dir);

    if verify_result.all_correct() {
        return Ok(Par2Result {
            success: true,
            blocks_needed: 0,
            blocks_available: verify_result.recovery_blocks_available,
            repaired: false,
            message: "All files correct".to_string(),
        });
    }

    let blocks_needed = verify_result.blocks_needed();
    let blocks_available = verify_result.recovery_blocks_available;

    info!(
        blocks_needed,
        blocks_available,
        damaged = verify_result.damaged.len(),
        missing = verify_result.missing.len(),
        "Damage detected, attempting native repair"
    );

    match rust_par2::repair_from_verify(&file_set, dir, &verify_result, false) {
        Ok(repair_result) => {
            info!(
                blocks_repaired = repair_result.blocks_repaired,
                files_repaired = repair_result.files_repaired,
                "Native PAR2 repair complete"
            );
            Ok(Par2Result {
                success: repair_result.success,
                blocks_needed,
                blocks_available,
                repaired: repair_result.success,
                message: repair_result.message,
            })
        }
        Err(e) => {
            warn!(error = %e, "Native PAR2 repair failed");
            Ok(Par2Result {
                success: false,
                blocks_needed,
                blocks_available,
                repaired: false,
                message: format!("{e}"),
            })
        }
    }
}

/// Async wrapper around `par2_repair_blocking`.
pub async fn par2_repair(par2_file: &Path) -> anyhow::Result<Par2Result> {
    let par2_file = par2_file.to_path_buf();
    tokio::task::spawn_blocking(move || par2_repair_blocking(&par2_file)).await?
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_repair_nonexistent_file() {
        let result = par2_repair(Path::new("/nonexistent/file.par2")).await;
        assert!(result.is_err() || !result.unwrap().success);
    }
}
