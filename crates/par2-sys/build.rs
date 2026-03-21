//! Build script for par2-sys.
//!
//! Strategy:
//! 1. Try to compile par2cmdline-turbo from source (autotools) for best performance.
//!    This produces a binary optimized for the build machine with OpenMP support.
//! 2. If autotools tools aren't available, fall back to downloading a pre-built
//!    release binary from GitHub (generic x86_64, statically linked).

use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

const VERSION: &str = "1.4.0";
const REPO_URL: &str = "https://github.com/animetosho/par2cmdline-turbo.git";
const RELEASE_BASE_URL: &str = "https://github.com/animetosho/par2cmdline-turbo/releases/download";

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let par2_bin = out_dir.join("par2");

    // Skip if already cached
    if par2_bin.exists() {
        eprintln!("par2-sys: using cached binary at {}", par2_bin.display());
        return;
    }

    // Try compiling from source first (produces optimized binary)
    if try_compile_from_source(&out_dir, &par2_bin) {
        eprintln!("par2-sys: compiled par2cmdline-turbo v{VERSION} from source");
        return;
    }

    // Fall back to pre-built release binary
    eprintln!("par2-sys: autotools not available, downloading pre-built binary");
    download_prebuilt(&par2_bin);
    eprintln!("par2-sys: installed pre-built par2cmdline-turbo v{VERSION}");
}

/// Try to compile par2cmdline-turbo from source using autotools.
/// Returns true on success, false if tools aren't available.
fn try_compile_from_source(out_dir: &Path, par2_bin: &Path) -> bool {
    // Check for required tools
    if !tool_exists("git") || !tool_exists("aclocal") || !tool_exists("autoconf")
        || !tool_exists("automake") || !tool_exists("make") || !tool_exists("g++")
    {
        eprintln!("par2-sys: missing build tools (need git, autotools, make, g++)");
        return false;
    }

    let src_dir = out_dir.join("par2cmdline-turbo-src");

    // Clone if not already present
    if !src_dir.join("configure.ac").exists() {
        eprintln!("par2-sys: cloning par2cmdline-turbo v{VERSION}...");
        let status = Command::new("git")
            .args(["clone", "--depth", "1", "--branch", &format!("v{VERSION}"), REPO_URL])
            .arg(&src_dir)
            .status();
        match status {
            Ok(s) if s.success() => {}
            _ => {
                eprintln!("par2-sys: git clone failed");
                let _ = fs::remove_dir_all(&src_dir);
                return false;
            }
        }
    }

    // Run autotools bootstrap
    if !src_dir.join("configure").exists() {
        for (cmd, args) in [
            ("aclocal", vec![]),
            ("automake", vec!["--add-missing"]),
            ("autoconf", vec![]),
        ] {
            if !run_in(&src_dir, cmd, &args) {
                eprintln!("par2-sys: {cmd} failed");
                return false;
            }
        }
    }

    // Configure (if not already done)
    if !src_dir.join("Makefile").exists() {
        if !run_in(&src_dir, "./configure", &[]) {
            eprintln!("par2-sys: configure failed");
            return false;
        }
    }

    // Build
    let num_jobs = env::var("NUM_JOBS").unwrap_or_else(|_| "4".to_string());
    if !run_in(&src_dir, "make", &[&format!("-j{num_jobs}")]) {
        eprintln!("par2-sys: make failed");
        return false;
    }

    // Copy the built binary
    let built = src_dir.join("par2");
    if !built.exists() {
        eprintln!("par2-sys: par2 binary not found after build");
        return false;
    }

    fs::copy(&built, par2_bin).expect("Failed to copy par2 binary");
    make_executable(par2_bin);
    true
}

/// Download pre-built release binary from GitHub.
fn download_prebuilt(par2_bin: &Path) {
    let asset_name = asset_for_target();
    let url = format!("{RELEASE_BASE_URL}/v{VERSION}/{asset_name}");

    eprintln!("par2-sys: downloading {url}");

    let response = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .expect("Failed to create HTTP client")
        .get(&url)
        .send()
        .unwrap_or_else(|e| panic!("Failed to download par2cmdline-turbo from {url}: {e}"));

    if !response.status().is_success() {
        panic!(
            "Failed to download par2cmdline-turbo: HTTP {}",
            response.status()
        );
    }

    let zip_bytes = response.bytes().expect("Failed to read response body");
    let cursor = std::io::Cursor::new(&zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).expect("Failed to open zip archive");

    let mut found = false;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).unwrap();
        let name = file.name().to_string();

        if name == "par2" || name == "par2.exe" {
            let mut contents = Vec::new();
            file.read_to_end(&mut contents)
                .expect("Failed to read par2 from zip");
            fs::write(par2_bin, &contents).expect("Failed to write par2 binary");
            make_executable(par2_bin);
            found = true;
            break;
        }
    }

    if !found {
        panic!("par2 binary not found in downloaded zip archive");
    }
}

fn tool_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run_in(dir: &Path, cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .current_dir(dir)
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
    }
}

fn asset_for_target() -> String {
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();

    let platform = match (os.as_str(), arch.as_str()) {
        ("linux", "x86_64") => "linux-amd64",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "arm") => "linux-armhf",
        ("macos", "x86_64") => "macos-amd64",
        ("macos", "aarch64") => "macos-arm64",
        ("windows", "x86_64") => "win-x64",
        ("windows", "aarch64") => "win-arm64",
        ("freebsd", "x86_64") => "freebsd-amd64",
        ("freebsd", "aarch64") => "freebsd-aarch64",
        _ => panic!("Unsupported platform: {os}-{arch}"),
    };

    format!("par2cmdline-turbo-{VERSION}-{platform}.zip")
}
