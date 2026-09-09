//! Path validation shared by download and post-processing boundaries.

use std::path::{Path, PathBuf};

/// Join an untrusted archive or API supplied relative name beneath `root`.
/// Backslashes are treated as separators on every platform.
pub fn safe_join(root: &Path, name: &str) -> Option<PathBuf> {
    let normalized = name.replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.as_bytes().get(1) == Some(&b':')
        || normalized.chars().any(char::is_control)
    {
        return None;
    }

    let mut output = root.to_path_buf();
    for component in normalized.split('/') {
        match component {
            "" | "." => {}
            ".." => return None,
            component => output.push(component),
        }
    }
    output.starts_with(root).then_some(output)
}

/// Validate a user supplied directory or category component.
/// These values are intentionally a single path component.
pub fn safe_component(value: &str) -> Option<&str> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return None;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_join_rejects_cross_platform_escape_paths() {
        let root = Path::new("/tmp/job");
        assert!(safe_join(root, "folder/file.txt").is_some());
        for path in ["../outside", r"..\outside", "/etc/passwd", r"C:\temp"] {
            assert!(safe_join(root, path).is_none(), "{path}");
        }
        assert!(safe_component("movies").is_some());
        assert!(safe_component("../outside").is_none());
        assert!(safe_component(r"movies\tv").is_none());
    }
}
