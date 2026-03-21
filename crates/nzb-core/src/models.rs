use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Job status lifecycle
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Downloading,
    Paused,
    Verifying,
    Repairing,
    Extracting,
    PostProcessing,
    Completed,
    Failed,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Queued => write!(f, "Queued"),
            Self::Downloading => write!(f, "Downloading"),
            Self::Paused => write!(f, "Paused"),
            Self::Verifying => write!(f, "Verifying"),
            Self::Repairing => write!(f, "Repairing"),
            Self::Extracting => write!(f, "Extracting"),
            Self::PostProcessing => write!(f, "PostProcessing"),
            Self::Completed => write!(f, "Completed"),
            Self::Failed => write!(f, "Failed"),
        }
    }
}

// ---------------------------------------------------------------------------
// Priority
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low = 0,
    Normal = 1,
    High = 2,
    Force = 3,
}

impl Default for Priority {
    fn default() -> Self {
        Self::Normal
    }
}

// ---------------------------------------------------------------------------
// NZB data model
// ---------------------------------------------------------------------------

/// Per-server article download statistics for a job.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerArticleStats {
    pub server_id: String,
    pub server_name: String,
    pub articles_downloaded: usize,
    pub articles_failed: usize,
    pub bytes_downloaded: u64,
}

/// A complete download job (parsed from one NZB file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NzbJob {
    /// Unique job identifier
    pub id: String,
    /// Human-readable name (from NZB filename or metadata)
    pub name: String,
    /// Category for this download
    pub category: String,
    /// Current status
    pub status: JobStatus,
    /// Download priority
    pub priority: Priority,
    /// Total size in bytes (sum of all articles)
    pub total_bytes: u64,
    /// Bytes downloaded so far
    pub downloaded_bytes: u64,
    /// Number of files in this job
    pub file_count: usize,
    /// Number of files completed
    pub files_completed: usize,
    /// Number of articles total
    pub article_count: usize,
    /// Number of articles downloaded
    pub articles_downloaded: usize,
    /// Number of articles failed
    pub articles_failed: usize,
    /// When the job was added
    pub added_at: DateTime<Utc>,
    /// When the job completed (if applicable)
    pub completed_at: Option<DateTime<Utc>>,
    /// Working directory for this job (incomplete)
    pub work_dir: PathBuf,
    /// Final output directory
    pub output_dir: PathBuf,
    /// Optional password for extraction
    pub password: Option<String>,
    /// Error message if failed
    pub error_message: Option<String>,
    /// Current download speed for this job (bytes/sec)
    #[serde(default)]
    pub speed_bps: u64,
    /// Per-server download statistics
    #[serde(default)]
    pub server_stats: Vec<ServerArticleStats>,
    /// Files in this job
    #[serde(skip)]
    pub files: Vec<NzbFile>,
}

/// A single file within an NZB job (collection of NNTP articles).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NzbFile {
    /// Unique file identifier
    pub id: String,
    /// Filename (from yEnc header or NZB subject)
    pub filename: String,
    /// Total size in bytes
    pub bytes: u64,
    /// Bytes downloaded
    pub bytes_downloaded: u64,
    /// Is this a par2 file?
    pub is_par2: bool,
    /// Par2 set name (if par2)
    pub par2_setname: Option<String>,
    /// Par2 volume number (if par2)
    pub par2_vol: Option<u32>,
    /// Par2 block count (if par2)
    pub par2_blocks: Option<u32>,
    /// File assembly complete
    pub assembled: bool,
    /// Newsgroup(s) this file was posted to
    pub groups: Vec<String>,
    /// Article segments
    #[serde(skip)]
    pub articles: Vec<Article>,
}

/// A single NNTP article (segment of a file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Article {
    /// Message-ID (e.g., "abc123@example.com")
    pub message_id: String,
    /// Segment number (1-based part number)
    pub segment_number: u32,
    /// Encoded size in bytes
    pub bytes: u64,
    /// Has this article been downloaded?
    pub downloaded: bool,
    /// Byte offset in the final file (set after yEnc decode)
    pub data_begin: Option<u64>,
    /// Size of decoded data for this segment
    pub data_size: Option<u64>,
    /// CRC32 of decoded data
    pub crc32: Option<u32>,
    /// Servers that have been tried for this article
    pub tried_servers: Vec<String>,
    /// Number of fetch attempts
    pub tries: u32,
}

// ---------------------------------------------------------------------------
// History record (for completed/failed jobs)
// ---------------------------------------------------------------------------

/// A history entry for a completed or failed job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub name: String,
    pub category: String,
    pub status: JobStatus,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub added_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub output_dir: PathBuf,
    /// Post-processing stages with results
    pub stages: Vec<StageResult>,
    pub error_message: Option<String>,
    /// Per-server download statistics
    #[serde(default)]
    pub server_stats: Vec<ServerArticleStats>,
    /// Raw NZB XML data (for retry)
    #[serde(skip_serializing)]
    pub nzb_data: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageResult {
    pub name: String,
    pub status: StageStatus,
    pub message: Option<String>,
    pub duration_secs: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Success,
    Failed,
    Skipped,
}

// ---------------------------------------------------------------------------
// RSS feed items and download rules
// ---------------------------------------------------------------------------

/// A discovered item from an RSS feed, persisted in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RssItem {
    /// Feed entry ID (from the RSS feed)
    pub id: String,
    /// Name of the feed this came from
    pub feed_name: String,
    /// Title of the entry
    pub title: String,
    /// NZB download URL
    pub url: Option<String>,
    /// When the entry was published (from feed)
    pub published_at: Option<DateTime<Utc>>,
    /// When we first saw this item
    pub first_seen_at: DateTime<Utc>,
    /// Whether this item has been downloaded
    pub downloaded: bool,
    /// When it was downloaded (if applicable)
    pub downloaded_at: Option<DateTime<Utc>>,
    /// Category used when downloaded
    pub category: Option<String>,
    /// Size in bytes (if available from feed)
    pub size_bytes: u64,
}

/// A download rule that automatically enqueues matching RSS feed items.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RssRule {
    /// Unique rule identifier
    pub id: String,
    /// Human-readable name for the rule
    pub name: String,
    /// Which feed(s) this rule applies to (one or more feed names)
    pub feed_names: Vec<String>,
    /// Category to assign to downloaded NZBs
    pub category: Option<String>,
    /// Download priority (0=low, 1=normal, 2=high, 3=force)
    pub priority: i32,
    /// Regex to match against feed item titles (applied to pre-filtered items)
    pub match_regex: String,
    /// Whether this rule is active
    pub enabled: bool,
}
