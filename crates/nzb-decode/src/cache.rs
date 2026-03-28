//! Bounded article cache with disk spill.
//!
//! Stores decoded article data in memory up to a configurable limit.
//! When the cache is full, the oldest entries are spilled to disk as
//! temporary files in the job's work directory.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::RwLock;
use thiserror::Error;
use tracing::{debug, trace, warn};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("I/O error during disk spill: {0}")]
    Io(#[from] io::Error),
}

pub type CacheResult<T> = std::result::Result<T, CacheError>;

// ---------------------------------------------------------------------------
// Cache key
// ---------------------------------------------------------------------------

/// Unique key for a cached article: (job_id, file_id, segment_number).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub job_id: String,
    pub file_id: String,
    pub segment_number: u32,
}

impl CacheKey {
    pub fn new(job_id: impl Into<String>, file_id: impl Into<String>, segment_number: u32) -> Self {
        Self {
            job_id: job_id.into(),
            file_id: file_id.into(),
            segment_number,
        }
    }
}

// ---------------------------------------------------------------------------
// Internal entry
// ---------------------------------------------------------------------------

/// Where a cached entry's data actually lives.
enum EntryLocation {
    /// Data is in memory.
    Memory(Vec<u8>),
    /// Data was spilled to disk at this path.
    Disk(PathBuf),
}

struct CacheEntry {
    location: EntryLocation,
    /// Size of the decoded data in bytes.
    size: usize,
}

// ---------------------------------------------------------------------------
// Cache stats
// ---------------------------------------------------------------------------

/// Runtime statistics for the article cache.
#[derive(Debug, Clone, Copy)]
pub struct CacheStats {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub disk_spills: u64,
}

// ---------------------------------------------------------------------------
// ArticleCache
// ---------------------------------------------------------------------------

/// Thread-safe, bounded in-memory cache for decoded article data.
///
/// When the in-memory size exceeds `max_bytes`, the oldest entries are
/// spilled to disk (temporary files inside `work_dir`).
pub struct ArticleCache {
    inner: RwLock<CacheInner>,
    // Atomic stats so reads don't need a write lock.
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    disk_spills: AtomicU64,
}

struct CacheInner {
    /// Map from key to entry.
    entries: HashMap<CacheKey, CacheEntry>,
    /// Insertion order for eviction (oldest first).
    insertion_order: VecDeque<CacheKey>,
    /// Current total in-memory size.
    memory_bytes: usize,
    /// Maximum in-memory size before spilling to disk.
    max_bytes: usize,
    /// Per-job work directories for disk spill.
    work_dirs: HashMap<String, PathBuf>,
}

impl ArticleCache {
    /// Create a new article cache with the given memory limit in bytes.
    pub fn new(max_bytes: usize) -> Self {
        Self {
            inner: RwLock::new(CacheInner {
                entries: HashMap::new(),
                insertion_order: VecDeque::new(),
                memory_bytes: 0,
                max_bytes,
                work_dirs: HashMap::new(),
            }),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            disk_spills: AtomicU64::new(0),
        }
    }

    /// Register the work directory for a job (used for disk spill paths).
    pub fn register_work_dir(&self, job_id: &str, work_dir: PathBuf) {
        let mut inner = self.inner.write();
        inner.work_dirs.insert(job_id.to_string(), work_dir);
    }

    /// Store decoded article data in the cache.
    ///
    /// If inserting this entry would push the cache over its limit, the
    /// oldest entries are spilled to disk until there is enough room.
    pub fn store(&self, key: CacheKey, data: Vec<u8>) -> CacheResult<()> {
        let data_len = data.len();
        let mut inner = self.inner.write();

        // If entry already exists, remove old data first.
        if let Some(old) = inner.entries.remove(&key) {
            if matches!(old.location, EntryLocation::Memory(_)) {
                inner.memory_bytes = inner.memory_bytes.saturating_sub(old.size);
            }
            // Remove from insertion_order
            inner.insertion_order.retain(|k| k != &key);
        }

        // Spill oldest entries to disk while we are over the limit.
        while inner.memory_bytes + data_len > inner.max_bytes && !inner.insertion_order.is_empty() {
            if let Some(evict_key) = inner.insertion_order.pop_front() {
                // Compute the spill path before borrowing entries mutably.
                let spill_path = Self::spill_path(&inner.work_dirs, &evict_key);

                if let Some(entry) = inner.entries.get_mut(&evict_key) {
                    if let EntryLocation::Memory(_) = &entry.location {
                        let entry_size = entry.size;
                        // Take the data out of the entry, replacing with disk location.
                        if let EntryLocation::Memory(mem_data) = std::mem::replace(
                            &mut entry.location,
                            EntryLocation::Disk(spill_path.clone()),
                        ) {
                            if let Err(e) = Self::write_spill(&spill_path, &mem_data) {
                                warn!(
                                    job_id = %evict_key.job_id,
                                    file_id = %evict_key.file_id,
                                    segment = evict_key.segment_number,
                                    "Failed to spill article to disk: {e}"
                                );
                                // Put data back in memory rather than lose it.
                                entry.location = EntryLocation::Memory(mem_data);
                                inner.insertion_order.push_front(evict_key);
                                break;
                            }
                            inner.memory_bytes = inner.memory_bytes.saturating_sub(entry_size);
                            self.disk_spills.fetch_add(1, Ordering::Relaxed);
                            debug!(
                                job_id = %evict_key.job_id,
                                file_id = %evict_key.file_id,
                                segment = evict_key.segment_number,
                                "Spilled article to disk"
                            );
                        }
                    }
                    // Re-add to back of insertion order (it still exists, just on disk).
                    inner.insertion_order.push_back(evict_key);
                }
            }
        }

        inner.memory_bytes += data_len;
        inner.entries.insert(
            key.clone(),
            CacheEntry {
                location: EntryLocation::Memory(data),
                size: data_len,
            },
        );
        inner.insertion_order.push_back(key);

        trace!(
            memory_bytes = inner.memory_bytes,
            max_bytes = inner.max_bytes,
            entries = inner.entries.len(),
            "Article cached"
        );

        Ok(())
    }

    /// Load article data from the cache (memory or disk).
    ///
    /// Returns `None` if the key is not in the cache.
    pub fn load(&self, key: &CacheKey) -> CacheResult<Option<Vec<u8>>> {
        let inner = self.inner.read();
        match inner.entries.get(key) {
            Some(entry) => match &entry.location {
                EntryLocation::Memory(data) => {
                    self.cache_hits.fetch_add(1, Ordering::Relaxed);
                    Ok(Some(data.clone()))
                }
                EntryLocation::Disk(path) => {
                    self.cache_hits.fetch_add(1, Ordering::Relaxed);
                    let data = Self::read_spill(path)?;
                    Ok(Some(data))
                }
            },
            None => {
                self.cache_misses.fetch_add(1, Ordering::Relaxed);
                Ok(None)
            }
        }
    }

    /// Remove an entry from the cache (memory and disk).
    pub fn remove(&self, key: &CacheKey) {
        let mut inner = self.inner.write();
        if let Some(entry) = inner.entries.remove(key) {
            match &entry.location {
                EntryLocation::Memory(_) => {
                    inner.memory_bytes = inner.memory_bytes.saturating_sub(entry.size);
                }
                EntryLocation::Disk(path) => {
                    let _ = fs::remove_file(path);
                }
            }
            inner.insertion_order.retain(|k| k != key);
        }
    }

    /// Remove all entries for a given job.
    pub fn clear_job(&self, job_id: &str) {
        let mut inner = self.inner.write();

        // Collect keys to remove.
        let keys_to_remove: Vec<CacheKey> = inner
            .entries
            .keys()
            .filter(|k| k.job_id == job_id)
            .cloned()
            .collect();

        for key in &keys_to_remove {
            if let Some(entry) = inner.entries.remove(key) {
                match &entry.location {
                    EntryLocation::Memory(_) => {
                        inner.memory_bytes = inner.memory_bytes.saturating_sub(entry.size);
                    }
                    EntryLocation::Disk(path) => {
                        let _ = fs::remove_file(path);
                    }
                }
            }
        }

        inner.insertion_order.retain(|k| k.job_id != job_id);
        inner.work_dirs.remove(job_id);

        debug!(
            job_id,
            removed = keys_to_remove.len(),
            "Cleared job from cache"
        );
    }

    /// Current in-memory cache size in bytes.
    pub fn size(&self) -> usize {
        self.inner.read().memory_bytes
    }

    /// Update the cache memory limit.
    pub fn set_limit(&self, bytes: usize) {
        let mut inner = self.inner.write();
        inner.max_bytes = bytes;
        // Note: we don't proactively spill here; spilling happens on next store().
    }

    /// Snapshot of cache statistics.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            cache_misses: self.cache_misses.load(Ordering::Relaxed),
            disk_spills: self.disk_spills.load(Ordering::Relaxed),
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Compute the spill file path for a given cache key.
    fn spill_path(work_dirs: &HashMap<String, PathBuf>, key: &CacheKey) -> PathBuf {
        let base = work_dirs
            .get(&key.job_id)
            .cloned()
            .unwrap_or_else(std::env::temp_dir);
        base.join(format!(".cache_{}_{}.tmp", key.file_id, key.segment_number))
    }

    /// Write data to a spill file, creating parent directories if needed.
    fn write_spill(path: &Path, data: &[u8]) -> CacheResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = fs::File::create(path)?;
        f.write_all(data)?;
        f.sync_data()?;
        Ok(())
    }

    /// Read data back from a spill file.
    fn read_spill(path: &Path) -> CacheResult<Vec<u8>> {
        let mut f = fs::File::open(path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        Ok(buf)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn key(job: &str, file: &str, seg: u32) -> CacheKey {
        CacheKey::new(job, file, seg)
    }

    #[test]
    fn test_store_and_load() {
        let cache = ArticleCache::new(1024 * 1024); // 1 MB
        let k = key("j1", "f1", 1);
        let data = vec![42u8; 100];

        cache.store(k.clone(), data.clone()).unwrap();
        let loaded = cache.load(&k).unwrap().unwrap();
        assert_eq!(loaded, data);
        assert_eq!(cache.size(), 100);

        let stats = cache.stats();
        assert_eq!(stats.cache_hits, 1);
        assert_eq!(stats.cache_misses, 0);
    }

    #[test]
    fn test_cache_miss() {
        let cache = ArticleCache::new(1024);
        let k = key("j1", "f1", 99);
        let loaded = cache.load(&k).unwrap();
        assert!(loaded.is_none());

        let stats = cache.stats();
        assert_eq!(stats.cache_misses, 1);
    }

    #[test]
    fn test_remove() {
        let cache = ArticleCache::new(1024);
        let k = key("j1", "f1", 1);
        cache.store(k.clone(), vec![1; 50]).unwrap();
        assert_eq!(cache.size(), 50);

        cache.remove(&k);
        assert_eq!(cache.size(), 0);
        assert!(cache.load(&k).unwrap().is_none());
    }

    #[test]
    fn test_clear_job() {
        let cache = ArticleCache::new(1024 * 1024);
        cache.store(key("j1", "f1", 1), vec![1; 10]).unwrap();
        cache.store(key("j1", "f1", 2), vec![2; 20]).unwrap();
        cache.store(key("j1", "f2", 1), vec![3; 30]).unwrap();
        cache.store(key("j2", "f1", 1), vec![4; 40]).unwrap();

        assert_eq!(cache.size(), 100);
        cache.clear_job("j1");
        assert_eq!(cache.size(), 40);

        assert!(cache.load(&key("j1", "f1", 1)).unwrap().is_none());
        assert!(cache.load(&key("j2", "f1", 1)).unwrap().is_some());
    }

    #[test]
    fn test_eviction_spills_to_disk() {
        let tmp = TempDir::new().unwrap();
        let cache = ArticleCache::new(200); // Very small limit

        cache.register_work_dir("j1", tmp.path().to_path_buf());

        // Store 3 x 100 bytes — only 200 bytes fit, so the first one should spill.
        cache.store(key("j1", "f1", 1), vec![1; 100]).unwrap();
        cache.store(key("j1", "f1", 2), vec![2; 100]).unwrap();
        // At this point we are at 200 bytes (exactly at limit).
        assert_eq!(cache.size(), 200);

        cache.store(key("j1", "f1", 3), vec![3; 100]).unwrap();
        // Segment 1 should have been spilled to disk.
        // Memory should be 200 (segments 2 and 3 in memory).
        assert_eq!(cache.size(), 200);

        let stats = cache.stats();
        assert!(
            stats.disk_spills >= 1,
            "Expected at least 1 disk spill, got {}",
            stats.disk_spills
        );

        // The spilled entry should still be loadable from disk.
        let data = cache.load(&key("j1", "f1", 1)).unwrap().unwrap();
        assert_eq!(data, vec![1; 100]);

        // Entries still in memory are also fine.
        let data2 = cache.load(&key("j1", "f1", 2)).unwrap().unwrap();
        assert_eq!(data2, vec![2; 100]);

        let data3 = cache.load(&key("j1", "f1", 3)).unwrap().unwrap();
        assert_eq!(data3, vec![3; 100]);
    }

    #[test]
    fn test_set_limit() {
        let cache = ArticleCache::new(1024);
        cache.store(key("j1", "f1", 1), vec![0; 100]).unwrap();

        cache.set_limit(50);
        // The limit changed but existing entries aren't evicted until next store().
        assert_eq!(cache.size(), 100);
    }

    #[test]
    fn test_overwrite_existing_key() {
        let cache = ArticleCache::new(1024);
        let k = key("j1", "f1", 1);

        cache.store(k.clone(), vec![1; 50]).unwrap();
        assert_eq!(cache.size(), 50);

        // Overwrite with larger data.
        cache.store(k.clone(), vec![2; 80]).unwrap();
        assert_eq!(cache.size(), 80);

        let loaded = cache.load(&k).unwrap().unwrap();
        assert_eq!(loaded, vec![2; 80]);
    }

    #[test]
    fn test_remove_cleans_disk() {
        let tmp = TempDir::new().unwrap();
        let cache = ArticleCache::new(50);
        cache.register_work_dir("j1", tmp.path().to_path_buf());

        // Force spill: store 2 x 50 bytes.
        cache.store(key("j1", "f1", 1), vec![1; 50]).unwrap();
        cache.store(key("j1", "f1", 2), vec![2; 50]).unwrap();

        // Segment 1 should be on disk.
        let spill = tmp.path().join(".cache_f1_1.tmp");
        assert!(spill.exists(), "Spill file should exist");

        cache.remove(&key("j1", "f1", 1));
        assert!(!spill.exists(), "Spill file should be removed");
    }
}
