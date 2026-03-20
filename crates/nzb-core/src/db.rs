use std::path::Path;

use chrono::Utc;
use rusqlite::{params, Connection};
use tracing::info;

use crate::error::NzbError;
use crate::models::*;

const SCHEMA_VERSION: u32 = 1;

/// Database handle for queue and history persistence.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open (or create) the database at the given path.
    pub fn open(path: &Path) -> Result<Self, NzbError> {
        let conn = Connection::open(path)?;

        // Enable WAL mode for concurrent reads during downloads
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;

        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Open an in-memory database (for testing).
    pub fn open_memory() -> Result<Self, NzbError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<(), NzbError> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER NOT NULL
            );",
        )?;

        let version: u32 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if version < 1 {
            info!("Applying database migration v1");
            self.conn.execute_batch(
                "
                -- Active download queue
                CREATE TABLE IF NOT EXISTS queue (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    category TEXT NOT NULL DEFAULT 'Default',
                    status TEXT NOT NULL DEFAULT 'queued',
                    priority INTEGER NOT NULL DEFAULT 1,
                    total_bytes INTEGER NOT NULL DEFAULT 0,
                    downloaded_bytes INTEGER NOT NULL DEFAULT 0,
                    file_count INTEGER NOT NULL DEFAULT 0,
                    files_completed INTEGER NOT NULL DEFAULT 0,
                    article_count INTEGER NOT NULL DEFAULT 0,
                    articles_downloaded INTEGER NOT NULL DEFAULT 0,
                    articles_failed INTEGER NOT NULL DEFAULT 0,
                    added_at TEXT NOT NULL,
                    completed_at TEXT,
                    work_dir TEXT NOT NULL,
                    output_dir TEXT NOT NULL,
                    password TEXT,
                    error_message TEXT,
                    -- Serialized NzbFile/Article data (bincode)
                    job_data BLOB
                );

                CREATE INDEX IF NOT EXISTS idx_queue_status ON queue(status);
                CREATE INDEX IF NOT EXISTS idx_queue_priority ON queue(priority DESC, added_at ASC);

                -- Completed/failed job history
                CREATE TABLE IF NOT EXISTS history (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    category TEXT NOT NULL DEFAULT 'Default',
                    status TEXT NOT NULL,
                    total_bytes INTEGER NOT NULL DEFAULT 0,
                    downloaded_bytes INTEGER NOT NULL DEFAULT 0,
                    added_at TEXT NOT NULL,
                    completed_at TEXT NOT NULL,
                    output_dir TEXT NOT NULL,
                    stages TEXT, -- JSON array of StageResult
                    error_message TEXT
                );

                CREATE INDEX IF NOT EXISTS idx_history_completed ON history(completed_at DESC);
                CREATE INDEX IF NOT EXISTS idx_history_status ON history(status);

                -- Server configuration (persisted separately from TOML for runtime changes)
                CREATE TABLE IF NOT EXISTS servers (
                    id TEXT PRIMARY KEY,
                    config TEXT NOT NULL -- JSON ServerConfig
                );

                INSERT INTO schema_version (version) VALUES (1);
                ",
            )?;
        }

        if version < 2 {
            info!("Applying database migration v2");
            self.conn.execute_batch(
                "
                -- Add NZB data storage and server stats to history
                ALTER TABLE history ADD COLUMN nzb_data BLOB;
                ALTER TABLE history ADD COLUMN server_stats TEXT DEFAULT '[]';

                -- Add server stats to queue
                ALTER TABLE queue ADD COLUMN server_stats TEXT DEFAULT '[]';

                -- Add NZB data to queue for preservation
                ALTER TABLE queue ADD COLUMN nzb_raw BLOB;

                UPDATE schema_version SET version = 2;
                ",
            )?;
        }

        if version < 3 {
            info!("Applying database migration v3");
            self.conn.execute_batch(
                "
                -- Per-job log storage for history
                ALTER TABLE history ADD COLUMN job_logs TEXT DEFAULT '[]';

                UPDATE schema_version SET version = 3;
                ",
            )?;
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Queue operations
    // -----------------------------------------------------------------------

    /// Insert a new job into the queue.
    pub fn queue_insert(&self, job: &NzbJob) -> Result<(), NzbError> {
        self.conn.execute(
            "INSERT INTO queue (id, name, category, status, priority, total_bytes,
             downloaded_bytes, file_count, files_completed, article_count,
             articles_downloaded, articles_failed, added_at, work_dir, output_dir, password)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                job.id,
                job.name,
                job.category,
                job.status.to_string(),
                job.priority as i32,
                job.total_bytes as i64,
                job.downloaded_bytes as i64,
                job.file_count as i64,
                job.files_completed as i64,
                job.article_count as i64,
                job.articles_downloaded as i64,
                job.articles_failed as i64,
                job.added_at.to_rfc3339(),
                job.work_dir.to_string_lossy().to_string(),
                job.output_dir.to_string_lossy().to_string(),
                job.password,
            ],
        )?;
        Ok(())
    }

    /// Update job progress in the queue.
    pub fn queue_update_progress(
        &self,
        id: &str,
        status: JobStatus,
        downloaded_bytes: u64,
        articles_downloaded: usize,
        articles_failed: usize,
        files_completed: usize,
    ) -> Result<(), NzbError> {
        self.conn.execute(
            "UPDATE queue SET status=?2, downloaded_bytes=?3, articles_downloaded=?4,
             articles_failed=?5, files_completed=?6 WHERE id=?1",
            params![
                id,
                status.to_string(),
                downloaded_bytes as i64,
                articles_downloaded as i64,
                articles_failed as i64,
                files_completed as i64,
            ],
        )?;
        Ok(())
    }

    /// Remove a job from the queue.
    pub fn queue_remove(&self, id: &str) -> Result<(), NzbError> {
        self.conn
            .execute("DELETE FROM queue WHERE id=?1", params![id])?;
        Ok(())
    }

    /// List all jobs in the queue, ordered by priority then add time.
    pub fn queue_list(&self) -> Result<Vec<NzbJob>, NzbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, category, status, priority, total_bytes, downloaded_bytes,
             file_count, files_completed, article_count, articles_downloaded, articles_failed,
             added_at, completed_at, work_dir, output_dir, password, error_message
             FROM queue ORDER BY priority DESC, added_at ASC",
        )?;

        let jobs = stmt
            .query_map([], |row| {
                Ok(NzbJob {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    category: row.get(2)?,
                    status: parse_status(&row.get::<_, String>(3)?),
                    priority: parse_priority(row.get::<_, i32>(4)?),
                    total_bytes: row.get::<_, i64>(5)? as u64,
                    downloaded_bytes: row.get::<_, i64>(6)? as u64,
                    file_count: row.get::<_, i64>(7)? as usize,
                    files_completed: row.get::<_, i64>(8)? as usize,
                    article_count: row.get::<_, i64>(9)? as usize,
                    articles_downloaded: row.get::<_, i64>(10)? as usize,
                    articles_failed: row.get::<_, i64>(11)? as usize,
                    added_at: parse_datetime(&row.get::<_, String>(12)?),
                    completed_at: row
                        .get::<_, Option<String>>(13)?
                        .map(|s| parse_datetime(&s)),
                    work_dir: row.get::<_, String>(14)?.into(),
                    output_dir: row.get::<_, String>(15)?.into(),
                    password: row.get(16)?,
                    error_message: row.get(17)?,
                    speed_bps: 0,
                    server_stats: Vec::new(),
                    files: Vec::new(), // Loaded separately
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(jobs)
    }

    // -----------------------------------------------------------------------
    // History operations
    // -----------------------------------------------------------------------

    /// Move a completed/failed job to history.
    pub fn history_insert(&self, entry: &HistoryEntry) -> Result<(), NzbError> {
        let stages_json = serde_json::to_string(&entry.stages).unwrap_or_default();
        let server_stats_json = serde_json::to_string(&entry.server_stats).unwrap_or_default();
        self.conn.execute(
            "INSERT INTO history (id, name, category, status, total_bytes, downloaded_bytes,
             added_at, completed_at, output_dir, stages, error_message, nzb_data, server_stats)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                entry.id,
                entry.name,
                entry.category,
                entry.status.to_string(),
                entry.total_bytes as i64,
                entry.downloaded_bytes as i64,
                entry.added_at.to_rfc3339(),
                entry.completed_at.to_rfc3339(),
                entry.output_dir.to_string_lossy().to_string(),
                stages_json,
                entry.error_message,
                entry.nzb_data,
                server_stats_json,
            ],
        )?;
        Ok(())
    }

    /// List history entries, most recent first.
    pub fn history_list(&self, limit: usize) -> Result<Vec<HistoryEntry>, NzbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, category, status, total_bytes, downloaded_bytes,
             added_at, completed_at, output_dir, stages, error_message, server_stats,
             CASE WHEN nzb_data IS NOT NULL THEN 1 ELSE 0 END as has_nzb
             FROM history ORDER BY completed_at DESC LIMIT ?1",
        )?;

        let entries = stmt
            .query_map(params![limit as i64], |row| {
                let stages_json: String = row.get::<_, Option<String>>(9)?.unwrap_or_default();
                let stages: Vec<StageResult> =
                    serde_json::from_str(&stages_json).unwrap_or_default();
                let stats_json: String = row.get::<_, Option<String>>(11)?.unwrap_or_default();
                let server_stats: Vec<ServerArticleStats> =
                    serde_json::from_str(&stats_json).unwrap_or_default();
                let has_nzb: i64 = row.get(12)?;

                Ok(HistoryEntry {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    category: row.get(2)?,
                    status: parse_status(&row.get::<_, String>(3)?),
                    total_bytes: row.get::<_, i64>(4)? as u64,
                    downloaded_bytes: row.get::<_, i64>(5)? as u64,
                    added_at: parse_datetime(&row.get::<_, String>(6)?),
                    completed_at: parse_datetime(&row.get::<_, String>(7)?),
                    output_dir: row.get::<_, String>(8)?.into(),
                    stages,
                    error_message: row.get(10)?,
                    server_stats,
                    // Don't load actual blob in list - just note if it exists
                    nzb_data: if has_nzb != 0 { Some(Vec::new()) } else { None },
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    /// Get the raw NZB data for a history entry (for retry).
    pub fn history_get_nzb_data(&self, id: &str) -> Result<Option<Vec<u8>>, NzbError> {
        let result = self.conn.query_row(
            "SELECT nzb_data FROM history WHERE id = ?1",
            params![id],
            |row| row.get::<_, Option<Vec<u8>>>(0),
        );
        match result {
            Ok(data) => Ok(data),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(NzbError::Database(e)),
        }
    }

    /// Enforce history retention limit by deleting oldest entries.
    pub fn history_enforce_retention(&self, max_entries: usize) -> Result<(), NzbError> {
        self.conn.execute(
            "DELETE FROM history WHERE id NOT IN (
                SELECT id FROM history ORDER BY completed_at DESC LIMIT ?1
            )",
            params![max_entries as i64],
        )?;
        Ok(())
    }

    /// Get a single history entry by ID.
    pub fn history_get(&self, id: &str) -> Result<Option<HistoryEntry>, NzbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, category, status, total_bytes, downloaded_bytes,
             added_at, completed_at, output_dir, stages, error_message, server_stats
             FROM history WHERE id = ?1",
        )?;

        let result = stmt.query_row(params![id], |row| {
            let stages_json: String = row.get::<_, Option<String>>(9)?.unwrap_or_default();
            let stages: Vec<StageResult> =
                serde_json::from_str(&stages_json).unwrap_or_default();
            let stats_json: String = row.get::<_, Option<String>>(11)?.unwrap_or_default();
            let server_stats: Vec<ServerArticleStats> =
                serde_json::from_str(&stats_json).unwrap_or_default();

            Ok(HistoryEntry {
                id: row.get(0)?,
                name: row.get(1)?,
                category: row.get(2)?,
                status: parse_status(&row.get::<_, String>(3)?),
                total_bytes: row.get::<_, i64>(4)? as u64,
                downloaded_bytes: row.get::<_, i64>(5)? as u64,
                added_at: parse_datetime(&row.get::<_, String>(6)?),
                completed_at: parse_datetime(&row.get::<_, String>(7)?),
                output_dir: row.get::<_, String>(8)?.into(),
                stages,
                error_message: row.get(10)?,
                server_stats,
                nzb_data: None,
            })
        });

        match result {
            Ok(entry) => Ok(Some(entry)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(NzbError::Database(e)),
        }
    }

    /// Store raw NZB data for a queue job.
    pub fn queue_store_nzb_data(&self, id: &str, nzb_data: &[u8]) -> Result<(), NzbError> {
        self.conn.execute(
            "UPDATE queue SET nzb_raw = ?2 WHERE id = ?1",
            params![id, nzb_data],
        )?;
        Ok(())
    }

    /// Get raw NZB data from a queue job.
    pub fn queue_get_nzb_data(&self, id: &str) -> Result<Option<Vec<u8>>, NzbError> {
        let result = self.conn.query_row(
            "SELECT nzb_raw FROM queue WHERE id = ?1",
            params![id],
            |row| row.get::<_, Option<Vec<u8>>>(0),
        );
        match result {
            Ok(data) => Ok(data),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(NzbError::Database(e)),
        }
    }

    /// Count history entries.
    pub fn history_count(&self) -> Result<usize, NzbError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM history", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// Remove a history entry.
    pub fn history_remove(&self, id: &str) -> Result<(), NzbError> {
        self.conn
            .execute("DELETE FROM history WHERE id=?1", params![id])?;
        Ok(())
    }

    /// Clear all history.
    pub fn history_clear(&self) -> Result<(), NzbError> {
        self.conn.execute("DELETE FROM history", [])?;
        Ok(())
    }

    /// Store per-job logs for a history entry.
    pub fn history_store_logs(&self, id: &str, logs_json: &str) -> Result<(), NzbError> {
        self.conn.execute(
            "UPDATE history SET job_logs = ?2 WHERE id = ?1",
            params![id, logs_json],
        )?;
        Ok(())
    }

    /// Get per-job logs for a history entry.
    pub fn history_get_logs(&self, id: &str) -> Result<Option<String>, NzbError> {
        let result = self.conn.query_row(
            "SELECT job_logs FROM history WHERE id = ?1",
            params![id],
            |row| row.get::<_, Option<String>>(0),
        );
        match result {
            Ok(data) => Ok(data),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(NzbError::Database(e)),
        }
    }
}

// ---------------------------------------------------------------------------
// Parse helpers
// ---------------------------------------------------------------------------

fn parse_status(s: &str) -> JobStatus {
    match s.to_lowercase().as_str() {
        "queued" => JobStatus::Queued,
        "downloading" => JobStatus::Downloading,
        "paused" => JobStatus::Paused,
        "verifying" => JobStatus::Verifying,
        "repairing" => JobStatus::Repairing,
        "extracting" => JobStatus::Extracting,
        "postprocessing" => JobStatus::PostProcessing,
        "completed" => JobStatus::Completed,
        "failed" => JobStatus::Failed,
        _ => JobStatus::Queued,
    }
}

fn parse_priority(v: i32) -> Priority {
    match v {
        0 => Priority::Low,
        1 => Priority::Normal,
        2 => Priority::High,
        3 => Priority::Force,
        _ => Priority::Normal,
    }
}

fn parse_datetime(s: &str) -> chrono::DateTime<Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_create_and_migrate() {
        let db = Database::open_memory().unwrap();
        let jobs = db.queue_list().unwrap();
        assert!(jobs.is_empty());
    }

    #[test]
    fn test_queue_insert_and_list() {
        let db = Database::open_memory().unwrap();
        let job = NzbJob {
            id: "test-123".into(),
            name: "Test Download".into(),
            category: "Default".into(),
            status: JobStatus::Queued,
            priority: Priority::Normal,
            total_bytes: 1_000_000,
            downloaded_bytes: 0,
            file_count: 3,
            files_completed: 0,
            article_count: 30,
            articles_downloaded: 0,
            articles_failed: 0,
            added_at: Utc::now(),
            completed_at: None,
            work_dir: "/tmp/test".into(),
            output_dir: "/downloads/test".into(),
            password: None,
            error_message: None,
            speed_bps: 0,
            server_stats: Vec::new(),
            files: Vec::new(),
        };

        db.queue_insert(&job).unwrap();
        let jobs = db.queue_list().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name, "Test Download");
        assert_eq!(jobs[0].total_bytes, 1_000_000);
    }

    #[test]
    fn test_history_insert_and_list() {
        let db = Database::open_memory().unwrap();
        let entry = HistoryEntry {
            id: "hist-1".into(),
            name: "Completed Job".into(),
            category: "movies".into(),
            status: JobStatus::Completed,
            total_bytes: 5_000_000,
            downloaded_bytes: 5_000_000,
            added_at: Utc::now(),
            completed_at: Utc::now(),
            output_dir: "/downloads/complete/movie".into(),
            stages: vec![StageResult {
                name: "Verify".into(),
                status: StageStatus::Success,
                message: None,
                duration_secs: 2.5,
            }],
            error_message: None,
            server_stats: Vec::new(),
            nzb_data: None,
        };

        db.history_insert(&entry).unwrap();
        let history = db.history_list(10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].name, "Completed Job");
        assert_eq!(history[0].stages.len(), 1);
    }
}
