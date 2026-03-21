//! Bundled par2cmdline-turbo binary.
//!
//! The binary is downloaded at build time and embedded into the Rust binary
//! via `include_bytes!`. On first use it is extracted to a temporary file
//! and made executable. This ensures it works in any deployment environment
//! (Docker, native, CI) without requiring a system-installed `par2`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The par2cmdline-turbo binary, embedded at compile time.
const PAR2_BINARY: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/par2"));

/// Version of the bundled par2cmdline-turbo binary.
pub const VERSION: &str = "1.4.0";

/// Path to the extracted par2cmdline-turbo binary.
///
/// On first call, extracts the embedded binary to a persistent location
/// and returns the path. Subsequent calls return the cached path.
pub fn par2_bin_path() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        extract_binary().expect("Failed to extract bundled par2 binary")
    })
}

fn extract_binary() -> std::io::Result<PathBuf> {
    // Use a stable location so we don't re-extract on every invocation.
    // data_dir/.par2-sys/par2-<version>
    let dir = std::env::var("PAR2_SYS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::temp_dir().join("par2-sys")
        });

    let bin_path = dir.join(format!("par2-{VERSION}"));

    // Already extracted and correct size?
    if bin_path.exists() {
        if let Ok(meta) = std::fs::metadata(&bin_path) {
            if meta.len() == PAR2_BINARY.len() as u64 {
                return Ok(bin_path);
            }
        }
    }

    std::fs::create_dir_all(&dir)?;
    std::fs::write(&bin_path, PAR2_BINARY)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin_path, std::fs::Permissions::from_mode(0o755))?;
    }

    Ok(bin_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_par2_bin_exists() {
        let path = par2_bin_path();
        assert!(path.exists(), "par2 binary should exist at {}", path.display());
    }

    #[test]
    fn test_par2_bin_is_executable() {
        let path = par2_bin_path();
        let output = std::process::Command::new(path)
            .arg("--help")
            .output()
            .expect("Failed to execute par2 binary");
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(
            combined.contains("par2") || combined.contains("PAR"),
            "par2 --help should mention par2: {combined}"
        );
    }

    #[test]
    fn test_par2_supports_threads() {
        let path = par2_bin_path();
        let output = std::process::Command::new(path)
            .arg("--help")
            .output()
            .expect("Failed to execute par2 binary");
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(
            combined.contains("-t") || combined.contains("thread"),
            "Bundled par2 should support -t (threads): {combined}"
        );
    }

    #[test]
    fn test_embedded_binary_size() {
        // par2cmdline-turbo is ~2.9MB statically linked
        assert!(
            PAR2_BINARY.len() > 1_000_000,
            "Embedded binary too small: {} bytes",
            PAR2_BINARY.len()
        );
    }
}
