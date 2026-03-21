//! Par2 verification and repair via bundled par2cmdline-turbo binary.
//!
//! The binary is downloaded at build time by the `par2-sys` crate and is
//! guaranteed to be par2cmdline-turbo with SIMD and multi-threading support.
//! No system-installed `par2` binary is required.

use std::path::Path;
use std::process::Stdio;

use regex::Regex;
use tokio::process::Command;
use tracing::{debug, info, warn};

/// Result of a par2 verify/repair operation.
#[derive(Debug)]
pub struct Par2Result {
    pub success: bool,
    pub blocks_needed: u32,
    pub blocks_available: u32,
    pub repaired: bool,
    pub output: String,
}

/// Parsed status from par2 stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Par2Status {
    /// All files are intact — no repair needed.
    AllCorrect,
    /// Repair is possible and was (or can be) completed.
    RepairPossible,
    /// Repair completed successfully.
    RepairComplete,
    /// Not enough recovery data to repair.
    RepairNotPossible,
    /// Could not determine status from output.
    Unknown,
}

impl std::fmt::Display for Par2Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AllCorrect => write!(f, "All files correct"),
            Self::RepairPossible => write!(f, "Repair possible"),
            Self::RepairComplete => write!(f, "Repair complete"),
            Self::RepairNotPossible => write!(f, "Repair not possible"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Parse par2 output to extract status information.
pub fn parse_par2_output(stdout: &str) -> (Par2Status, u32, u32) {
    let status = if stdout.contains("All files are correct") {
        Par2Status::AllCorrect
    } else if stdout.contains("Repair complete") {
        Par2Status::RepairComplete
    } else if stdout.contains("Repair is not possible") {
        Par2Status::RepairNotPossible
    } else if stdout.contains("Repair is required") || stdout.contains("repair is possible") {
        Par2Status::RepairPossible
    } else {
        Par2Status::Unknown
    };

    let blocks_needed = parse_blocks_needed(stdout);
    let blocks_available = parse_blocks_available(stdout);

    (status, blocks_needed, blocks_available)
}

/// Extract "You need N more recovery blocks" from par2 output.
fn parse_blocks_needed(stdout: &str) -> u32 {
    let re = Regex::new(r"You need (\d+) more recovery block").unwrap();
    re.captures(stdout)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0)
}

/// Extract available recovery block count from par2 output.
fn parse_blocks_available(stdout: &str) -> u32 {
    let re = Regex::new(r"(\d+) recovery blocks? available").unwrap();
    re.captures(stdout)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0)
}

/// Verify par2 integrity of files in a directory.
pub async fn par2_verify(par2_file: &Path) -> anyhow::Result<Par2Result> {
    let par2_bin = par2_sys::par2_bin_path();
    let basepath = par2_file.parent().unwrap_or(Path::new("."));
    let wildcard = format!("{}/*", basepath.display());

    info!(file = %par2_file.display(), "Running par2 verify");

    let output = Command::new(par2_bin)
        .arg("verify")
        .arg("-B")
        .arg(basepath)
        .arg(par2_file)
        .arg(&wildcard)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let success = output.status.success();

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    debug!(
        exit_code = ?output.status.code(),
        stdout_len = stdout.len(),
        stderr = %stderr,
        "par2 verify completed"
    );

    let (status, blocks_needed, blocks_available) = parse_par2_output(&stdout);
    info!(%status, blocks_needed, blocks_available, "par2 verify result");

    let effective_success = success || status == Par2Status::AllCorrect;

    Ok(Par2Result {
        success: effective_success,
        blocks_needed,
        blocks_available,
        repaired: false,
        output: stdout,
    })
}

/// Repair files using par2 recovery blocks.
///
/// The `repair` command verifies first, then repairs if needed — this
/// is a single-pass alternative to calling verify + repair separately.
pub async fn par2_repair(par2_file: &Path) -> anyhow::Result<Par2Result> {
    let par2_bin = par2_sys::par2_bin_path();
    let basepath = par2_file.parent().unwrap_or(Path::new("."));
    let wildcard = format!("{}/*", basepath.display());

    info!(file = %par2_file.display(), "Running par2 repair");

    let output = Command::new(par2_bin)
        .arg("repair")
        .arg("-B")
        .arg(basepath)
        .arg(par2_file)
        .arg(&wildcard)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let success = output.status.success();

    let (status, blocks_needed, blocks_available) = parse_par2_output(&stdout);

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let repaired = success || status == Par2Status::RepairComplete;

    if repaired {
        info!("par2 repair successful");
    } else {
        warn!(
            %status,
            blocks_needed,
            blocks_available,
            exit_code = ?output.status.code(),
            stderr = %stderr,
            stdout_len = stdout.len(),
            "par2 repair failed"
        );
    }

    Ok(Par2Result {
        success: repaired,
        blocks_needed,
        blocks_available,
        repaired,
        output: stdout,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_all_correct() {
        let output = "Loading packets...\nAll files are correct, repair is not required.\n";
        let (status, needed, _available) = parse_par2_output(output);
        assert_eq!(status, Par2Status::AllCorrect);
        assert_eq!(needed, 0);
    }

    #[test]
    fn test_parse_repair_complete() {
        let output = "Verifying repaired files...\nRepair complete.\n";
        let (status, _, _) = parse_par2_output(output);
        assert_eq!(status, Par2Status::RepairComplete);
    }

    #[test]
    fn test_parse_repair_not_possible() {
        let output =
            "You need 5 more recovery blocks to be able to repair.\nRepair is not possible.\n";
        let (status, needed, _) = parse_par2_output(output);
        assert_eq!(status, Par2Status::RepairNotPossible);
        assert_eq!(needed, 5);
    }

    #[test]
    fn test_parse_blocks_available() {
        let output = "There are 42 recovery blocks available.\n";
        let (_, _, available) = parse_par2_output(output);
        assert_eq!(available, 42);
    }

    #[test]
    fn test_parse_repair_possible() {
        let output = "Repair is required.\nYou have 10 out of 10 recovery blocks available.\nrepair is possible\n";
        let (status, _, available) = parse_par2_output(output);
        assert_eq!(status, Par2Status::RepairPossible);
        assert_eq!(available, 10);
    }

    #[test]
    fn test_parse_unknown_output() {
        let output = "Some unexpected output from par2\n";
        let (status, _, _) = parse_par2_output(output);
        assert_eq!(status, Par2Status::Unknown);
    }

    #[test]
    fn test_bundled_binary_exists() {
        assert!(
            par2_sys::par2_bin_path().exists(),
            "Bundled par2 binary should exist"
        );
    }
}
