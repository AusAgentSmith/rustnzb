//! File detection helpers for post-processing.
//!
//! Scans a completed download directory to find par2 files, RAR archives,
//! 7z archives, ZIP archives, and cleanup candidates.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// The type of archive detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveType {
    Rar,
    SevenZip,
    Zip,
}

impl std::fmt::Display for ArchiveType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rar => write!(f, "RAR"),
            Self::SevenZip => write!(f, "7z"),
            Self::Zip => write!(f, "ZIP"),
        }
    }
}

/// Find all `.par2` files in a directory. The index par2 file (without
/// `.volNNN+NNN.par2` suffix) is returned first so callers can use it
/// as the primary verification target.
pub fn find_par2_files(dir: &Path) -> Vec<PathBuf> {
    let mut index_files: Vec<PathBuf> = Vec::new();
    let mut volume_files: Vec<PathBuf> = Vec::new();

    for entry in WalkDir::new(dir).into_iter().flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_lowercase(),
            None => continue,
        };
        if !name.ends_with(".par2") {
            continue;
        }
        // Index par2 files do NOT contain ".vol" before ".par2"
        if is_par2_volume(&name) {
            volume_files.push(path.to_path_buf());
        } else {
            index_files.push(path.to_path_buf());
        }
    }

    index_files.sort();
    volume_files.sort();

    // Index files first, then volumes
    index_files.extend(volume_files);
    index_files
}

/// Returns true if a filename looks like a par2 volume file (e.g.
/// `foo.vol00+01.par2`) rather than the index file (`foo.par2`).
fn is_par2_volume(name_lower: &str) -> bool {
    // Typical pattern: .vol000+000.par2
    // We check for ".vol" anywhere before the final ".par2"
    let without_ext = name_lower.trim_end_matches(".par2");
    // Look for ".vol" followed by digits, a '+', and more digits
    if let Some(vol_pos) = without_ext.rfind(".vol") {
        let after_vol = &without_ext[vol_pos + 4..];
        // Check pattern: digits + '+' + digits
        if let Some(plus_pos) = after_vol.find('+') {
            let before_plus = &after_vol[..plus_pos];
            let after_plus = &after_vol[plus_pos + 1..];
            return !before_plus.is_empty()
                && before_plus.chars().all(|c| c.is_ascii_digit())
                && !after_plus.is_empty()
                && after_plus.chars().all(|c| c.is_ascii_digit());
        }
    }
    false
}

/// Find the first RAR volume(s) in a directory. Handles both old-style naming
/// (.rar, .r00, .r01, ...) and new-style (.part001.rar, .part002.rar, ...).
///
/// Returns only the *first* volume of each archive set (the one you pass to
/// `unrar x`).
pub fn find_rar_files(dir: &Path) -> Vec<PathBuf> {
    let mut first_volumes: Vec<PathBuf> = Vec::new();

    for entry in WalkDir::new(dir).into_iter().flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        let name_lower = name.to_lowercase();

        // New-style: .part001.rar is the first volume
        if name_lower.ends_with(".rar") {
            if let Some(stem) = name_lower.strip_suffix(".rar") {
                // Check for .partNNN pattern
                if let Some(dot_pos) = stem.rfind(".part") {
                    let part_num_str = &stem[dot_pos + 5..];
                    if !part_num_str.is_empty() && part_num_str.chars().all(|c| c.is_ascii_digit()) {
                        if let Ok(part_num) = part_num_str.parse::<u32>() {
                            if part_num == 1 {
                                first_volumes.push(path.to_path_buf());
                            }
                            // part > 1 is not a first volume
                            continue;
                        }
                    }
                }
                // Plain .rar with no .partNNN — this is the first volume in old-style
                first_volumes.push(path.to_path_buf());
            }
        }
        // Old-style: .r00, .r01, etc. — we do NOT add these; the .rar file
        // is the first volume in old-style sets.
    }

    first_volumes.sort();
    first_volumes
}

/// Detect all archives in a directory. Returns (ArchiveType, path) pairs.
/// For multi-volume RAR sets, only the first volume is returned.
pub fn find_archives(dir: &Path) -> Vec<(ArchiveType, PathBuf)> {
    let mut archives: Vec<(ArchiveType, PathBuf)> = Vec::new();

    // RAR first volumes
    for path in find_rar_files(dir) {
        archives.push((ArchiveType::Rar, path));
    }

    // 7z and ZIP
    for entry in WalkDir::new(dir).into_iter().flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_lowercase(),
            None => continue,
        };

        if name.ends_with(".7z") {
            archives.push((ArchiveType::SevenZip, path.to_path_buf()));
        } else if name.ends_with(".zip") {
            archives.push((ArchiveType::Zip, path.to_path_buf()));
        }
    }

    archives.sort_by(|a, b| a.1.cmp(&b.1));
    archives
}

/// Find files that are safe to delete after successful extraction.
/// This includes par2 files, RAR volumes (old-style and new-style), and
/// other recovery/split files.
pub fn find_cleanup_files(dir: &Path) -> Vec<PathBuf> {
    let mut cleanup: Vec<PathBuf> = Vec::new();

    for entry in WalkDir::new(dir).into_iter().flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_lowercase(),
            None => continue,
        };

        if is_cleanup_candidate(&name) {
            cleanup.push(path.to_path_buf());
        }
    }

    cleanup.sort();
    cleanup
}

/// Determine whether a file (by its lowercased name) is safe to clean up
/// after successful extraction.
fn is_cleanup_candidate(name: &str) -> bool {
    // Par2 files: .par2
    if name.ends_with(".par2") {
        return true;
    }

    // RAR volumes (new-style): .part001.rar, .part002.rar, ...
    // and plain .rar files
    if name.ends_with(".rar") {
        return true;
    }

    // Old-style RAR split volumes: .r00, .r01, ..., .r99, .s00, ...
    // Pattern: ends with .rNN or .sNN (or any letter + two digits)
    if name.len() > 4 {
        let last4 = &name[name.len() - 4..];
        if last4.starts_with('.')
            && last4.as_bytes()[1].is_ascii_lowercase()
            && last4.as_bytes()[2].is_ascii_digit()
            && last4.as_bytes()[3].is_ascii_digit()
        {
            return true;
        }
    }

    // Extended old-style volumes beyond .r99: .s00, .t00, etc. are caught above.
    // Also handle three-digit extensions: .part001.rar already covered.

    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a temporary directory with the given filenames (empty files).
    fn make_test_dir(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for name in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, b"").unwrap();
        }
        dir
    }

    #[test]
    fn test_find_par2_index_first() {
        let dir = make_test_dir(&[
            "movie.vol00+01.par2",
            "movie.vol01+02.par2",
            "movie.par2",
        ]);
        let results = find_par2_files(dir.path());
        assert_eq!(results.len(), 3);
        // Index file should come first
        assert!(
            results[0].file_name().unwrap().to_str().unwrap() == "movie.par2",
            "Index par2 file should be first, got {:?}",
            results[0]
        );
    }

    #[test]
    fn test_find_par2_empty_dir() {
        let dir = make_test_dir(&["readme.txt", "movie.mkv"]);
        let results = find_par2_files(dir.path());
        assert!(results.is_empty());
    }

    #[test]
    fn test_find_rar_new_style() {
        let dir = make_test_dir(&[
            "archive.part001.rar",
            "archive.part002.rar",
            "archive.part003.rar",
        ]);
        let results = find_rar_files(dir.path());
        // Only the first volume should be returned
        assert_eq!(results.len(), 1);
        assert!(
            results[0]
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .contains("part001"),
        );
    }

    #[test]
    fn test_find_rar_old_style() {
        let dir = make_test_dir(&[
            "archive.rar",
            "archive.r00",
            "archive.r01",
            "archive.r02",
        ]);
        let results = find_rar_files(dir.path());
        // Only .rar (the first volume) should be returned
        assert_eq!(results.len(), 1);
        assert!(results[0]
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with(".rar"));
    }

    #[test]
    fn test_find_archives_mixed() {
        let dir = make_test_dir(&[
            "movie.part001.rar",
            "movie.part002.rar",
            "subs.zip",
            "extras.7z",
        ]);
        let results = find_archives(dir.path());
        let types: Vec<ArchiveType> = results.iter().map(|(t, _)| *t).collect();
        assert!(types.contains(&ArchiveType::Rar));
        assert!(types.contains(&ArchiveType::Zip));
        assert!(types.contains(&ArchiveType::SevenZip));
        // RAR should only have 1 entry (first volume)
        assert_eq!(types.iter().filter(|&&t| t == ArchiveType::Rar).count(), 1);
    }

    #[test]
    fn test_find_cleanup_files() {
        let dir = make_test_dir(&[
            "movie.par2",
            "movie.vol00+01.par2",
            "movie.part001.rar",
            "movie.part002.rar",
            "movie.r00",
            "movie.r01",
            "movie.mkv",           // should NOT be cleaned up
            "readme.txt",          // should NOT be cleaned up
        ]);
        let results = find_cleanup_files(dir.path());
        // par2 (2) + rar (2) + r00 + r01 = 6
        assert_eq!(results.len(), 6, "Expected 6 cleanup files, got: {:?}", results);
        // .mkv and .txt should NOT be present
        for path in &results {
            let name = path.file_name().unwrap().to_str().unwrap();
            assert!(!name.ends_with(".mkv"));
            assert!(!name.ends_with(".txt"));
        }
    }

    #[test]
    fn test_is_par2_volume() {
        assert!(is_par2_volume("file.vol00+01.par2"));
        assert!(is_par2_volume("file.vol123+456.par2"));
        assert!(!is_par2_volume("file.par2"));
        assert!(!is_par2_volume("file.volume.par2"));
    }

    #[test]
    fn test_cleanup_old_style_volumes() {
        assert!(is_cleanup_candidate("archive.r00"));
        assert!(is_cleanup_candidate("archive.r99"));
        assert!(is_cleanup_candidate("archive.s00"));
        assert!(!is_cleanup_candidate("readme.txt"));
        assert!(!is_cleanup_candidate("movie.mkv"));
    }
}
