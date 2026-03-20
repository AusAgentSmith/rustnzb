//! File assembler — writes decoded articles into final output files.
//!
//! Articles can arrive out of order from multiple NNTP connections.
//! Uses `pwrite` (`write_at`) for lock-free concurrent writes — multiple
//! threads can write to the same file at different offsets without
//! serialization. File handles are opened once at registration and
//! reused for all segments.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io;
#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(windows)]
use std::os::windows::fs::FileExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use parking_lot::RwLock;
use thiserror::Error;
use tracing::{debug, info};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Error, Debug)]
pub enum AssemblerError {
    #[error("I/O error writing file: {0}")]
    Io(#[from] io::Error),
    #[error("File not registered: job={job_id}, file={file_id}")]
    FileNotRegistered { job_id: String, file_id: String },
    #[error("Segment number {segment} out of range (1..={total})")]
    SegmentOutOfRange { segment: u32, total: u32 },
}

pub type AssemblerResult<T> = std::result::Result<T, AssemblerError>;

// ---------------------------------------------------------------------------
// Per-file tracking
// ---------------------------------------------------------------------------

/// Tracks assembly progress for a single output file.
struct FileState {
    /// Path to the output file on disk (retained for diagnostics).
    #[allow(dead_code)]
    output_path: PathBuf,
    /// Persistent file handle — opened once, reused for all segment writes.
    file: File,
    /// Total number of segments expected.
    total_segments: u32,
    /// Which segments have been written (indexed by segment_number - 1).
    /// Uses a Vec<AtomicU8> as atomic bitflags so bitmap updates don't
    /// need a write-lock on the outer HashMap.
    written: Vec<std::sync::atomic::AtomicU8>,
    /// How many segments have been written so far.
    written_count: AtomicU32,
}

impl FileState {
    fn new(output_path: PathBuf, file: File, total_segments: u32) -> Self {
        let written: Vec<std::sync::atomic::AtomicU8> = (0..total_segments)
            .map(|_| std::sync::atomic::AtomicU8::new(0))
            .collect();
        Self {
            output_path,
            file,
            total_segments,
            written,
            written_count: AtomicU32::new(0),
        }
    }

    fn is_complete(&self) -> bool {
        self.written_count.load(Ordering::Acquire) == self.total_segments
    }

    /// Return (segments_written, total_segments).
    fn progress(&self) -> (u32, u32) {
        (self.written_count.load(Ordering::Relaxed), self.total_segments)
    }

    /// Mark a segment as written. Returns `true` if the file just became complete.
    fn mark_written(&self, segment_number: u32) -> bool {
        let idx = (segment_number - 1) as usize;
        if idx < self.written.len() {
            // CAS: only increment counter if this is the first time marking this segment
            let prev = self.written[idx].swap(1, Ordering::AcqRel);
            if prev == 0 {
                self.written_count.fetch_add(1, Ordering::AcqRel);
            }
        }
        self.is_complete()
    }

    /// Return a list of missing segment numbers (1-based).
    fn missing_segments(&self) -> Vec<u32> {
        self.written
            .iter()
            .enumerate()
            .filter(|(_, w)| w.load(Ordering::Relaxed) == 0)
            .map(|(i, _)| (i + 1) as u32)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Composite key for the file state map
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FileKey {
    job_id: String,
    file_id: String,
}

// ---------------------------------------------------------------------------
// FileAssembler
// ---------------------------------------------------------------------------

/// Assembles decoded articles into final output files.
///
/// Thread-safe: multiple NNTP connections can call `assemble_article`
/// concurrently. Uses `pwrite` (write_at) for lock-free writes at
/// arbitrary offsets — no per-file mutex needed.
pub struct FileAssembler {
    /// Per-file state, behind a RwLock for safe concurrent registration and lookup.
    files: RwLock<HashMap<FileKey, FileState>>,
}

impl FileAssembler {
    /// Create a new file assembler.
    pub fn new() -> Self {
        Self {
            files: RwLock::new(HashMap::new()),
        }
    }

    /// Register a file for assembly.
    ///
    /// Must be called before any articles for this file are assembled.
    /// `output_path` is the full path where the final file will be written.
    /// `total_segments` is the total number of article segments for this file.
    pub fn register_file(
        &self,
        job_id: &str,
        file_id: &str,
        output_path: PathBuf,
        total_segments: u32,
    ) -> AssemblerResult<()> {
        // Ensure parent directory exists.
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Open file handle once — kept open for all segment writes.
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&output_path)?;

        let key = FileKey {
            job_id: job_id.to_string(),
            file_id: file_id.to_string(),
        };

        let mut files = self.files.write();
        files.insert(key, FileState::new(output_path, file, total_segments));
        Ok(())
    }

    /// Write a decoded article directly to the output file at the given offset.
    ///
    /// Uses `pwrite` (write_at) for lock-free concurrent writes — multiple
    /// threads can write to different offsets of the same file simultaneously
    /// without any mutex. The kernel handles synchronization.
    ///
    /// Returns `true` if the file is now complete (all segments written).
    pub fn assemble_article(
        &self,
        job_id: &str,
        file_id: &str,
        segment_number: u32,
        data_begin: u64,
        data: &[u8],
    ) -> AssemblerResult<bool> {
        let key = FileKey {
            job_id: job_id.to_string(),
            file_id: file_id.to_string(),
        };

        // Read-lock only — pwrite + atomic bitmap need no write-lock.
        let files = self.files.read();
        let state = files.get(&key).ok_or_else(|| AssemblerError::FileNotRegistered {
            job_id: job_id.to_string(),
            file_id: file_id.to_string(),
        })?;

        if segment_number == 0 || segment_number > state.total_segments {
            return Err(AssemblerError::SegmentOutOfRange {
                segment: segment_number,
                total: state.total_segments,
            });
        }

        // pwrite: write at offset without seeking — concurrent-safe on the same fd.
        let io_start = Instant::now();
        #[cfg(unix)]
        state.file.write_all_at(data, data_begin)?;
        #[cfg(windows)]
        {
            let mut offset = data_begin;
            let mut remaining = data;
            while !remaining.is_empty() {
                let n = state.file.seek_write(remaining, offset)?;
                offset += n as u64;
                remaining = &remaining[n..];
            }
        }
        let io_us = io_start.elapsed().as_micros();

        debug!(
            job_id,
            file_id,
            segment = segment_number,
            offset = data_begin,
            len = data.len(),
            io_us,
            "Wrote article segment to file"
        );

        // Atomic bitmap update — no write-lock needed.
        let complete = state.mark_written(segment_number);
        if complete {
            info!(
                job_id,
                file_id,
                total_segments = state.total_segments,
                "File assembly complete"
            );
        }

        Ok(complete)
    }

    /// Check whether all segments for a file have been written.
    pub fn is_file_complete(&self, job_id: &str, file_id: &str) -> bool {
        let key = FileKey {
            job_id: job_id.to_string(),
            file_id: file_id.to_string(),
        };
        let files = self.files.read();
        files.get(&key).is_some_and(|s| s.is_complete())
    }

    /// Get assembly progress: (segments_written, total_segments).
    ///
    /// Returns `(0, 0)` if the file is not registered.
    pub fn get_file_progress(&self, job_id: &str, file_id: &str) -> (u32, u32) {
        let key = FileKey {
            job_id: job_id.to_string(),
            file_id: file_id.to_string(),
        };
        let files = self.files.read();
        files.get(&key).map(|s| s.progress()).unwrap_or((0, 0))
    }

    /// Get the list of missing segment numbers for a file (1-based).
    ///
    /// Returns an empty vec if the file is complete or not registered.
    pub fn missing_segments(&self, job_id: &str, file_id: &str) -> Vec<u32> {
        let key = FileKey {
            job_id: job_id.to_string(),
            file_id: file_id.to_string(),
        };
        let files = self.files.read();
        files
            .get(&key)
            .map(|s| s.missing_segments())
            .unwrap_or_default()
    }

    /// Remove tracking state for all files belonging to a job.
    pub fn clear_job(&self, job_id: &str) {
        let mut files = self.files.write();
        files.retain(|k, _| k.job_id != job_id);
    }
}

impl Default for FileAssembler {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    /// Helper: register a file and return its output path.
    fn setup_file(
        assembler: &FileAssembler,
        dir: &Path,
        job: &str,
        file: &str,
        filename: &str,
        total_segments: u32,
    ) -> PathBuf {
        let path = dir.join(filename);
        assembler
            .register_file(job, file, path.clone(), total_segments)
            .unwrap();
        path
    }

    #[test]
    fn test_sequential_assembly() {
        let tmp = TempDir::new().unwrap();
        let assembler = FileAssembler::new();
        let path = setup_file(&assembler, tmp.path(), "j1", "f1", "test.bin", 3);

        // Write 3 segments in order.
        let seg1 = b"AAAA";
        let seg2 = b"BBBB";
        let seg3 = b"CC";

        assert!(!assembler
            .assemble_article("j1", "f1", 1, 0, seg1)
            .unwrap());
        assert_eq!(assembler.get_file_progress("j1", "f1"), (1, 3));

        assert!(!assembler
            .assemble_article("j1", "f1", 2, 4, seg2)
            .unwrap());
        assert_eq!(assembler.get_file_progress("j1", "f1"), (2, 3));

        assert!(assembler
            .assemble_article("j1", "f1", 3, 8, seg3)
            .unwrap());
        assert!(assembler.is_file_complete("j1", "f1"));

        // Verify file contents.
        let contents = fs::read(&path).unwrap();
        assert_eq!(&contents[0..4], b"AAAA");
        assert_eq!(&contents[4..8], b"BBBB");
        assert_eq!(&contents[8..10], b"CC");
    }

    #[test]
    fn test_out_of_order_assembly() {
        let tmp = TempDir::new().unwrap();
        let assembler = FileAssembler::new();
        let path = setup_file(&assembler, tmp.path(), "j1", "f1", "ooo.bin", 3);

        // Write segments out of order: 3, 1, 2.
        let seg1 = b"AAAA";
        let seg2 = b"BBBB";
        let seg3 = b"CC";

        assert!(!assembler
            .assemble_article("j1", "f1", 3, 8, seg3)
            .unwrap());
        assert_eq!(assembler.get_file_progress("j1", "f1"), (1, 3));

        assert!(!assembler
            .assemble_article("j1", "f1", 1, 0, seg1)
            .unwrap());
        assert_eq!(assembler.get_file_progress("j1", "f1"), (2, 3));

        assert!(assembler
            .assemble_article("j1", "f1", 2, 4, seg2)
            .unwrap());
        assert!(assembler.is_file_complete("j1", "f1"));

        // Verify file contents — should be correctly assembled despite ordering.
        let contents = fs::read(&path).unwrap();
        assert_eq!(&contents[0..4], b"AAAA");
        assert_eq!(&contents[4..8], b"BBBB");
        assert_eq!(&contents[8..10], b"CC");
    }

    #[test]
    fn test_completion_detection() {
        let tmp = TempDir::new().unwrap();
        let assembler = FileAssembler::new();
        setup_file(&assembler, tmp.path(), "j1", "f1", "complete.bin", 2);

        assert!(!assembler.is_file_complete("j1", "f1"));
        assert_eq!(assembler.missing_segments("j1", "f1"), vec![1, 2]);

        assembler
            .assemble_article("j1", "f1", 1, 0, b"data")
            .unwrap();
        assert!(!assembler.is_file_complete("j1", "f1"));
        assert_eq!(assembler.missing_segments("j1", "f1"), vec![2]);

        assembler
            .assemble_article("j1", "f1", 2, 4, b"more")
            .unwrap();
        assert!(assembler.is_file_complete("j1", "f1"));
        assert!(assembler.missing_segments("j1", "f1").is_empty());
    }

    #[test]
    fn test_unregistered_file_error() {
        let assembler = FileAssembler::new();
        let result = assembler.assemble_article("j1", "nope", 1, 0, b"data");
        assert!(result.is_err());
        match result.unwrap_err() {
            AssemblerError::FileNotRegistered { job_id, file_id } => {
                assert_eq!(job_id, "j1");
                assert_eq!(file_id, "nope");
            }
            other => panic!("Expected FileNotRegistered, got: {other}"),
        }
    }

    #[test]
    fn test_segment_out_of_range() {
        let tmp = TempDir::new().unwrap();
        let assembler = FileAssembler::new();
        setup_file(&assembler, tmp.path(), "j1", "f1", "range.bin", 3);

        let result = assembler.assemble_article("j1", "f1", 0, 0, b"bad");
        assert!(matches!(
            result.unwrap_err(),
            AssemblerError::SegmentOutOfRange {
                segment: 0,
                total: 3
            }
        ));

        let result = assembler.assemble_article("j1", "f1", 4, 0, b"bad");
        assert!(matches!(
            result.unwrap_err(),
            AssemblerError::SegmentOutOfRange {
                segment: 4,
                total: 3
            }
        ));
    }

    #[test]
    fn test_progress_unregistered() {
        let assembler = FileAssembler::new();
        assert_eq!(assembler.get_file_progress("x", "y"), (0, 0));
        assert!(!assembler.is_file_complete("x", "y"));
    }

    #[test]
    fn test_clear_job() {
        let tmp = TempDir::new().unwrap();
        let assembler = FileAssembler::new();
        setup_file(&assembler, tmp.path(), "j1", "f1", "a.bin", 2);
        setup_file(&assembler, tmp.path(), "j1", "f2", "b.bin", 3);
        setup_file(&assembler, tmp.path(), "j2", "f3", "c.bin", 1);

        assembler.clear_job("j1");
        assert_eq!(assembler.get_file_progress("j1", "f1"), (0, 0));
        assert_eq!(assembler.get_file_progress("j1", "f2"), (0, 0));
        // j2 should still exist.
        assert_eq!(assembler.get_file_progress("j2", "f3"), (0, 1));
    }

    #[test]
    fn test_duplicate_segment_write() {
        let tmp = TempDir::new().unwrap();
        let assembler = FileAssembler::new();
        setup_file(&assembler, tmp.path(), "j1", "f1", "dup.bin", 2);

        assembler
            .assemble_article("j1", "f1", 1, 0, b"first")
            .unwrap();
        assert_eq!(assembler.get_file_progress("j1", "f1"), (1, 2));

        // Writing the same segment again should be idempotent for progress tracking.
        assembler
            .assemble_article("j1", "f1", 1, 0, b"retry")
            .unwrap();
        assert_eq!(assembler.get_file_progress("j1", "f1"), (1, 2));
    }

    #[test]
    fn test_concurrent_different_files() {
        // Ensure the assembler handles multiple files for the same job.
        let tmp = TempDir::new().unwrap();
        let assembler = FileAssembler::new();
        let path_a = setup_file(&assembler, tmp.path(), "j1", "f1", "a.bin", 1);
        let path_b = setup_file(&assembler, tmp.path(), "j1", "f2", "b.bin", 1);

        assert!(assembler
            .assemble_article("j1", "f1", 1, 0, b"AAA")
            .unwrap());
        assert!(assembler
            .assemble_article("j1", "f2", 1, 0, b"BBB")
            .unwrap());

        assert_eq!(fs::read(&path_a).unwrap(), b"AAA");
        assert_eq!(fs::read(&path_b).unwrap(), b"BBB");
    }
}
