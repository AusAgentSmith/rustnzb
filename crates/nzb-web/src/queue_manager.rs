//! Queue manager — coordinates downloads across the application.
//!
//! The QueueManager owns the list of active NzbJobs, manages the download
//! engine instances, and exposes a thread-safe API for the HTTP handlers
//! to interact with.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::nzb_core::config::{CategoryConfig, ServerConfig};
use crate::nzb_core::db::Database;
use crate::nzb_core::models::*;
use crate::nzb_core::nzb_parser;
use nzb_postproc::{PostProcConfig, run_pipeline};

use crate::bandwidth::BandwidthLimiter;
use crate::download_engine::{DownloadEngine, ProgressUpdate};
use crate::log_buffer::LogBuffer;

/// Get free disk space for a path (returns 0 on error).
fn get_disk_free(path: &std::path::Path) -> u64 {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::mem::MaybeUninit;
        let c_path = match CString::new(path.to_string_lossy().as_bytes()) {
            Ok(p) => p,
            Err(_) => return 0,
        };
        unsafe {
            let mut stat = MaybeUninit::<libc::statvfs>::uninit();
            if libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) == 0 {
                let stat = stat.assume_init();
                #[allow(clippy::unnecessary_cast)] // u32 on macOS, u64 on Linux
                return stat.f_bavail as u64 * stat.f_frsize as u64;
            }
        }
        0
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        0
    }
}

// ---------------------------------------------------------------------------
// Job checkpoint for resume support
// ---------------------------------------------------------------------------

/// Compact representation of per-file article completion state.
/// Stored as JSON in the `job_data` column for resuming downloads after restart.
#[derive(Serialize, Deserialize)]
struct JobCheckpoint {
    /// Map of file_id -> set of downloaded segment numbers
    files: HashMap<String, Vec<u32>>,
    /// Bytes downloaded so far
    downloaded_bytes: u64,
    /// Number of articles downloaded
    articles_downloaded: usize,
    /// Number of articles failed
    articles_failed: usize,
    /// Number of files completed
    files_completed: usize,
}

// ---------------------------------------------------------------------------
// Speed tracker (simple rolling window)
// ---------------------------------------------------------------------------

pub(crate) struct SpeedTracker {
    /// Bytes downloaded in the current window.
    window_bytes: AtomicU64,
    /// Current speed in bytes per second.
    current_bps: AtomicU64,
}

impl SpeedTracker {
    pub fn new() -> Self {
        Self {
            window_bytes: AtomicU64::new(0),
            current_bps: AtomicU64::new(0),
        }
    }

    /// Record downloaded bytes.
    pub fn record(&self, bytes: u64) {
        self.window_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Called periodically to compute speed and reset the window.
    pub fn tick(&self, elapsed_secs: f64) {
        let bytes = self.window_bytes.swap(0, Ordering::Relaxed);
        if elapsed_secs > 0.001 {
            let bps = (bytes as f64 / elapsed_secs) as u64;
            self.current_bps.store(bps, Ordering::Relaxed);
        }
    }

    pub fn bps(&self) -> u64 {
        self.current_bps.load(Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------------------
// Per-job state
// ---------------------------------------------------------------------------

struct JobState {
    /// The job data (shared with API for reading).
    job: NzbJob,
    /// The download engine for this job.
    engine: Arc<DownloadEngine>,
    /// Handle to the download task (so we can await or abort it).
    task_handle: Option<tokio::task::JoinHandle<()>>,
    /// Per-job speed tracker.
    speed: Arc<SpeedTracker>,
    /// Raw NZB data for retry.
    nzb_data: Option<Vec<u8>>,
}

// ---------------------------------------------------------------------------
// QueueManager
// ---------------------------------------------------------------------------

/// Thread-safe queue manager that coordinates all downloads.
///
/// Wrapped in `Arc` for sharing between the background task and HTTP handlers.
pub struct QueueManager {
    /// Active jobs keyed by job ID.
    jobs: Mutex<HashMap<String, JobState>>,
    /// Order of job IDs for display.
    job_order: Mutex<Vec<String>>,
    /// Server configurations.
    servers: Mutex<Vec<ServerConfig>>,
    /// Whether all downloads are globally paused.
    globally_paused: AtomicBool,
    /// Global speed tracker.
    speed: SpeedTracker,
    /// Database for persistence.
    db: Mutex<Database>,
    /// App config (incomplete_dir, complete_dir).
    incomplete_dir: std::path::PathBuf,
    complete_dir: std::path::PathBuf,
    /// Timed pause: when to auto-resume (None = not timed).
    pause_until: Mutex<Option<DateTime<Utc>>>,
    /// History retention limit (None = keep all).
    history_retention: Mutex<Option<usize>>,
    /// Log buffer for capturing per-job logs into history.
    log_buffer: Option<LogBuffer>,
    /// Max concurrent active downloads (0 = unlimited).
    max_active_downloads: AtomicUsize,
    /// Category configs for post-processing decisions.
    categories: Mutex<Vec<CategoryConfig>>,
    /// Minimum free disk space in bytes before pausing downloads.
    min_free_space: u64,
    /// Bandwidth limiter for throttling downloads.
    bandwidth: Arc<BandwidthLimiter>,
}

impl QueueManager {
    /// Create a new queue manager.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        servers: Vec<ServerConfig>,
        db: Database,
        incomplete_dir: std::path::PathBuf,
        complete_dir: std::path::PathBuf,
        log_buffer: LogBuffer,
        max_active_downloads: usize,
        categories: Vec<CategoryConfig>,
        min_free_space: u64,
        speed_limit_bps: u64,
    ) -> Arc<Self> {
        use crate::bandwidth::BandwidthConfig;
        use std::num::NonZeroU32;

        let download_bps = if speed_limit_bps > 0 {
            NonZeroU32::new(speed_limit_bps as u32)
        } else {
            None
        };
        let bandwidth = Arc::new(BandwidthLimiter::new(BandwidthConfig { download_bps }));

        Arc::new(Self {
            jobs: Mutex::new(HashMap::new()),
            job_order: Mutex::new(Vec::new()),
            servers: Mutex::new(servers),
            globally_paused: AtomicBool::new(false),
            speed: SpeedTracker::new(),
            db: Mutex::new(db),
            incomplete_dir,
            complete_dir,
            pause_until: Mutex::new(None),
            history_retention: Mutex::new(None),
            log_buffer: Some(log_buffer),
            max_active_downloads: AtomicUsize::new(max_active_downloads),
            categories: Mutex::new(categories),
            min_free_space,
            bandwidth,
        })
    }

    /// Update category configs (e.g. after config reload).
    pub fn set_categories(&self, categories: Vec<CategoryConfig>) {
        *self.categories.lock() = categories;
    }

    /// Set history retention limit.
    pub fn set_history_retention(&self, limit: Option<usize>) {
        *self.history_retention.lock() = limit;
    }

    /// Set max active downloads and start queued jobs if capacity allows.
    pub fn set_max_active_downloads(self: &Arc<Self>, max: usize) {
        self.max_active_downloads.store(max, Ordering::Relaxed);
        self.start_next_queued();
    }

    /// Get max active downloads.
    pub fn get_max_active_downloads(&self) -> usize {
        self.max_active_downloads.load(Ordering::Relaxed)
    }

    /// Set the download speed limit in bytes per second (0 = unlimited).
    pub fn set_speed_limit(&self, bps: u64) {
        use std::num::NonZeroU32;
        let limit = if bps > 0 {
            NonZeroU32::new(bps as u32)
        } else {
            None
        };
        self.bandwidth.set_download_bps(limit);
    }

    /// Get the current download speed limit in bytes per second (0 = unlimited).
    pub fn get_speed_limit(&self) -> u64 {
        self.bandwidth
            .get_download_bps()
            .map(|v| v.get() as u64)
            .unwrap_or(0)
    }

    /// Atomically find the next queued job that can start, mark it as
    /// `Downloading` in the jobs map, and return its ID.
    ///
    /// Returns `None` if no download slot is available or there are no
    /// queued jobs.  Because the status transition happens under the same
    /// lock acquisition as the active-count check, concurrent callers
    /// cannot both claim the same slot (no TOCTOU race).
    fn claim_next_download_slot(&self, max: usize) -> Option<String> {
        let mut jobs = self.jobs.lock();
        let active = jobs
            .values()
            .filter(|s| s.job.status == JobStatus::Downloading)
            .count();
        if max > 0 && active >= max {
            return None;
        }

        let order = self.job_order.lock();
        let mut best: Option<(String, u8)> = None;
        for id in order.iter() {
            if let Some(s) = jobs.get(id)
                && s.job.status == JobStatus::Queued
            {
                let p = s.job.priority as u8;
                if best.as_ref().is_none_or(|(_, bp)| p > *bp) {
                    best = Some((id.clone(), p));
                }
            }
        }
        let (id, _) = best?;

        // Mark as Downloading while still holding the lock
        if let Some(state) = jobs.get_mut(&id) {
            state.job.status = JobStatus::Downloading;
            info!(job_id = %id, name = %state.job.name, "Starting queued job");
        }
        Some(id)
    }

    /// Start queued jobs up to the concurrency limit.
    fn start_next_queued(self: &Arc<Self>) {
        let max = self.max_active_downloads.load(Ordering::Relaxed);
        loop {
            let Some(job_id) = self.claim_next_download_slot(max) else {
                break;
            };
            self.launch_download(&job_id);
        }
    }

    /// Add a job to the queue and start downloading.
    ///
    /// The job should already have its `work_dir` and `output_dir` set.
    pub fn add_job(
        self: &Arc<Self>,
        mut job: NzbJob,
        nzb_data: Option<Vec<u8>>,
    ) -> crate::nzb_core::Result<()> {
        // Ensure work directory exists
        std::fs::create_dir_all(&job.work_dir)?;

        // Persist to DB
        {
            let db = self.db.lock();
            db.queue_insert(&job)?;
            // Store raw NZB data if available
            if let Some(ref data) = nzb_data {
                let _ = db.queue_store_nzb_data(&job.id, data);
            }
        }

        let job_id = job.id.clone();
        info!(
            job_id = %job_id,
            name = %job.name,
            files = job.file_count,
            articles = job.article_count,
            "Job added to queue"
        );

        // If globally paused, add as paused
        if self.globally_paused.load(Ordering::Relaxed) {
            job.status = JobStatus::Paused;
            let engine = Arc::new(DownloadEngine::new());
            engine.pause();
            let state = JobState {
                job,
                engine,
                task_handle: None,
                speed: Arc::new(SpeedTracker::new()),
                nzb_data,
            };
            self.jobs.lock().insert(job_id.clone(), state);
            self.job_order.lock().push(job_id);
            return Ok(());
        }

        // Insert as Queued — start_next_queued will atomically claim a
        // download slot if one is available.
        job.status = JobStatus::Queued;
        let engine = Arc::new(DownloadEngine::new());
        let state = JobState {
            job,
            engine,
            task_handle: None,
            speed: Arc::new(SpeedTracker::new()),
            nzb_data,
        };
        self.jobs.lock().insert(job_id.clone(), state);
        self.job_order.lock().push(job_id);

        // Try to start this or other queued jobs
        self.start_next_queued();
        Ok(())
    }

    /// Launch the download task for a job that is already in the jobs map
    /// with status `Downloading`.
    ///
    /// Creates a `DownloadEngine`, spawns the download task, and updates
    /// the existing map entry with the engine and task handle.  If the
    /// pre-flight disk-space check fails, the job is set to `Paused`.
    fn launch_download(self: &Arc<Self>, job_id: &str) {
        // Read job data from the map (we need a copy for the spawned task)
        let (job, _nzb_data) = {
            let jobs = self.jobs.lock();
            let Some(state) = jobs.get(job_id) else {
                return;
            };
            (state.job.clone(), state.nzb_data.clone())
        };

        // Pre-flight disk space check
        let free = get_disk_free(&self.incomplete_dir);
        if self.min_free_space > 0 && free > 0 && free < self.min_free_space {
            warn!(
                job_id = %job_id,
                free_bytes = free,
                min_free_space = self.min_free_space,
                "Paused job due to low disk space"
            );
            let mut jobs = self.jobs.lock();
            if let Some(state) = jobs.get_mut(job_id) {
                state.job.status = JobStatus::Paused;
                state.job.error_message = Some("Paused: low disk space".to_string());
                state.engine.pause();
            }
            return;
        }

        info!(
            job_id = %job_id,
            name = %job.name,
            total_bytes = job.total_bytes,
            article_count = job.article_count,
            file_count = job.file_count,
            "Starting download job"
        );

        let engine = Arc::new(DownloadEngine::new());
        let job_speed = Arc::new(SpeedTracker::new());
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();

        let servers = self.servers.lock().clone();
        let engine_clone = Arc::clone(&engine);
        let job_clone = job;
        let bandwidth = Arc::clone(&self.bandwidth);

        // Spawn the download task
        let task_handle = tokio::spawn(async move {
            engine_clone
                .run(&job_clone, &servers, progress_tx, bandwidth)
                .await;
        });

        // Update the existing map entry with the engine and task handle
        {
            let mut jobs = self.jobs.lock();
            if let Some(state) = jobs.get_mut(job_id) {
                state.engine = engine;
                state.task_handle = Some(task_handle);
                state.speed = Arc::clone(&job_speed);
            }
        }

        // Spawn the progress handler
        let qm = Arc::clone(self);
        let jid = job_id.to_string();
        tokio::spawn(async move {
            qm.handle_progress(jid, progress_rx, job_speed).await;
        });
    }

    /// Handle progress updates from the download engine.
    async fn handle_progress(
        self: Arc<Self>,
        job_id: String,
        mut progress_rx: mpsc::UnboundedReceiver<ProgressUpdate>,
        job_speed: Arc<SpeedTracker>,
    ) {
        let mut last_db_update = Instant::now();

        while let Some(update) = progress_rx.recv().await {
            match update {
                ProgressUpdate::ArticleComplete {
                    file_id,
                    segment_number,
                    decoded_bytes,
                    file_complete,
                    server_id,
                    ..
                } => {
                    self.speed.record(decoded_bytes);
                    job_speed.record(decoded_bytes);

                    // Update in-memory job state
                    {
                        let mut jobs = self.jobs.lock();
                        if let Some(state) = jobs.get_mut(&job_id) {
                            state.job.downloaded_bytes += decoded_bytes;
                            state.job.articles_downloaded += 1;

                            // Update per-server stats
                            if let Some(ref sid) = server_id {
                                let stats = &mut state.job.server_stats;
                                if let Some(ss) = stats.iter_mut().find(|s| s.server_id == *sid) {
                                    ss.articles_downloaded += 1;
                                    ss.bytes_downloaded += decoded_bytes;
                                } else {
                                    // Find server name from config
                                    let sname = self
                                        .servers
                                        .lock()
                                        .iter()
                                        .find(|s| s.id == *sid)
                                        .map(|s| s.name.clone())
                                        .unwrap_or_else(|| sid.clone());
                                    stats.push(ServerArticleStats {
                                        server_id: sid.clone(),
                                        server_name: sname,
                                        articles_downloaded: 1,
                                        articles_failed: 0,
                                        bytes_downloaded: decoded_bytes,
                                    });
                                }
                            }

                            for file in &mut state.job.files {
                                if file.id == file_id {
                                    file.bytes_downloaded += decoded_bytes;
                                    for article in &mut file.articles {
                                        if article.segment_number == segment_number {
                                            article.downloaded = true;
                                            article.data_size = Some(decoded_bytes);
                                        }
                                    }
                                    if file_complete {
                                        file.assembled = true;
                                        state.job.files_completed += 1;
                                        info!(
                                            job_id = %job_id,
                                            file = %file.filename,
                                            completed = state.job.files_completed,
                                            total = state.job.file_count,
                                            "File assembly complete"
                                        );
                                    }
                                    break;
                                }
                            }
                        }
                    }

                    // Batch DB writes (every 2 seconds)
                    if last_db_update.elapsed() >= Duration::from_secs(2) {
                        self.persist_job_progress(&job_id);
                        last_db_update = Instant::now();
                    }
                }
                ProgressUpdate::ArticleFailed {
                    error, server_id, ..
                } => {
                    let mut jobs = self.jobs.lock();
                    if let Some(state) = jobs.get_mut(&job_id) {
                        state.job.articles_failed += 1;

                        // Update per-server failed stats
                        if let Some(ref sid) = server_id {
                            let stats = &mut state.job.server_stats;
                            if let Some(ss) = stats.iter_mut().find(|s| s.server_id == *sid) {
                                ss.articles_failed += 1;
                            } else {
                                let sname = self
                                    .servers
                                    .lock()
                                    .iter()
                                    .find(|s| s.id == *sid)
                                    .map(|s| s.name.clone())
                                    .unwrap_or_else(|| sid.clone());
                                stats.push(ServerArticleStats {
                                    server_id: sid.clone(),
                                    server_name: sname,
                                    articles_downloaded: 0,
                                    articles_failed: 1,
                                    bytes_downloaded: 0,
                                });
                            }
                        }
                    }
                    warn!(job_id = %job_id, "Article failed: {error}");
                }
                ProgressUpdate::JobFinished {
                    success,
                    articles_failed,
                    ..
                } => {
                    info!(
                        job_id = %job_id,
                        success,
                        articles_failed,
                        "Job download finished"
                    );

                    // Mark as PostProcessing immediately so the slot is freed
                    // for the next queued job. This lets the next download ramp
                    // up while post-processing (par2/unpack) runs concurrently.
                    {
                        let mut jobs = self.jobs.lock();
                        if let Some(state) = jobs.get_mut(&job_id) {
                            state.job.status = JobStatus::PostProcessing;
                            state.job.completed_at = Some(chrono::Utc::now());
                        }
                    }
                    self.start_next_queued();

                    self.on_job_finished(&job_id, success, articles_failed)
                        .await;
                    break;
                }
                ProgressUpdate::NoServersAvailable { reason, .. } => {
                    warn!(
                        job_id = %job_id,
                        reason = %reason,
                        "No servers available — pausing job for retry"
                    );
                    {
                        let mut jobs = self.jobs.lock();
                        if let Some(state) = jobs.get_mut(&job_id) {
                            state.job.status = JobStatus::Paused;
                            state.job.error_message = Some(reason);
                            state.engine.pause();
                        }
                    }
                    self.persist_job_progress(&job_id);
                    break;
                }
            }
        }
    }

    /// Called when a job's download phase completes.
    ///
    /// Note: the job's status is already set to `PostProcessing` and
    /// `start_next_queued()` has already been called by `handle_progress`,
    /// so the next download is ramping up concurrently with this work.
    async fn on_job_finished(
        self: &Arc<Self>,
        job_id: &str,
        success: bool,
        articles_failed: usize,
    ) {
        let pipeline_start = Instant::now();

        // Extract info needed for post-processing
        let (work_dir, output_dir, category, pp_level) = {
            let jobs = self.jobs.lock();
            let Some(state) = jobs.get(job_id) else {
                return;
            };

            if success {
                info!(job_id = %job_id, "Job moving to post-processing");
            } else {
                info!(
                    job_id = %job_id,
                    articles_failed,
                    "Job moving to post-processing ({articles_failed} article(s) failed, par2 may repair)"
                );
            }

            let cat = state.job.category.clone();
            let pp = self
                .categories
                .lock()
                .iter()
                .find(|c| c.name == cat)
                .map(|c| c.post_processing)
                .unwrap_or(3); // default: repair+unpack
            (
                state.job.work_dir.clone(),
                state.job.output_dir.clone(),
                cat,
                pp,
            )
        };

        // Run post-processing pipeline (par2 can repair failed articles)
        let stages = if pp_level > 0 {
            info!(
                job_id = %job_id,
                category = %category,
                pp_level,
                "Running post-processing pipeline"
            );

            let config = PostProcConfig {
                cleanup_after_extract: true,
                output_dir: Some(output_dir.clone()),
                articles_failed,
            };

            let result = run_pipeline(&work_dir, &config).await;

            info!(
                job_id = %job_id,
                success = result.success,
                stages = result.stages.len(),
                elapsed_secs = pipeline_start.elapsed().as_secs_f64(),
                "Post-processing pipeline finished"
            );

            // Update job status based on pipeline result
            {
                let mut jobs = self.jobs.lock();
                if let Some(state) = jobs.get_mut(job_id)
                    && !result.success
                {
                    state.job.status = JobStatus::Failed;
                    state.job.error_message = result.error.clone();
                }
            }

            result.stages
        } else {
            info!(job_id = %job_id, pp_level, "Post-processing disabled for category, skipping pipeline");
            // No pipeline to repair — if articles failed, mark as failed now
            if !success {
                let mut jobs = self.jobs.lock();
                if let Some(state) = jobs.get_mut(job_id) {
                    state.job.status = JobStatus::Failed;
                    state.job.error_message =
                        Some(format!("{articles_failed} article(s) failed to download"));
                }
            }
            Vec::new()
        };

        // Move to history with real stage results
        {
            let mut jobs = self.jobs.lock();
            if let Some(state) = jobs.get_mut(job_id) {
                self.move_to_history(state, stages);
            }
        }

        // Persist final state then remove from active queue
        self.persist_job_progress(job_id);

        // Remove from in-memory queue
        self.jobs.lock().remove(job_id);
        self.job_order.lock().retain(|jid| jid != job_id);
    }

    /// Move a job's files to output and insert a history entry.
    fn move_to_history(&self, state: &mut JobState, stages: Vec<StageResult>) {
        let move_start = Instant::now();

        let final_status = if state.job.status == JobStatus::Failed {
            // Already marked failed (by pipeline or download with pp disabled)
            JobStatus::Failed
        } else {
            // Pipeline ran successfully (or no articles failed) — job is complete.
            // Par2 may have repaired missing articles, so articles_failed > 0 is OK.
            JobStatus::Completed
        };

        // Move files from work_dir to output_dir (if not already done by pipeline extract)
        if final_status == JobStatus::Completed {
            if let Err(e) = std::fs::create_dir_all(&state.job.output_dir) {
                warn!(job_id = %state.job.id, "Failed to create output dir: {e}");
            }
            if let Ok(entries) = std::fs::read_dir(&state.job.work_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let dest = state.job.output_dir.join(entry.file_name());
                        if let Err(e) = std::fs::rename(&path, &dest) {
                            if let Err(e2) = std::fs::copy(&path, &dest) {
                                warn!(
                                    job_id = %state.job.id,
                                    file = %path.display(),
                                    "Failed to move file: rename={e}, copy={e2}"
                                );
                            } else {
                                let _ = std::fs::remove_file(&path);
                            }
                        }
                    }
                }
            }
        }

        let file_move_secs = move_start.elapsed().as_secs_f64();
        info!(
            job_id = %state.job.id,
            final_status = %final_status,
            file_move_secs = format!("{file_move_secs:.3}"),
            stage_count = stages.len(),
            "Moving job to history"
        );

        state.job.status = final_status;

        // Insert into history with real stage results
        let history_entry = HistoryEntry {
            id: state.job.id.clone(),
            name: state.job.name.clone(),
            category: state.job.category.clone(),
            status: final_status,
            total_bytes: state.job.total_bytes,
            downloaded_bytes: state.job.downloaded_bytes,
            added_at: state.job.added_at,
            completed_at: state.job.completed_at.unwrap_or_else(chrono::Utc::now),
            output_dir: state.job.output_dir.clone(),
            stages,
            error_message: state.job.error_message.clone(),
            server_stats: state.job.server_stats.clone(),
            nzb_data: state.nzb_data.clone(),
        };

        let db = self.db.lock();
        if let Err(e) = db.history_insert(&history_entry) {
            error!(job_id = %state.job.id, "Failed to insert history: {e}");
        }

        // Capture and persist per-job logs from the ring buffer
        if let Some(ref log_buffer) = self.log_buffer {
            let logs = log_buffer.get_entries(Some(&state.job.id), None, None, 5000);
            if !logs.is_empty() {
                let logs_json = serde_json::to_string(&logs).unwrap_or_default();
                if let Err(e) = db.history_store_logs(&state.job.id, &logs_json) {
                    warn!(job_id = %state.job.id, "Failed to store logs in history: {e}");
                }
            }
        }

        if let Err(e) = db.queue_remove(&state.job.id) {
            error!(job_id = %state.job.id, "Failed to remove from queue: {e}");
        }

        // Enforce retention
        if let Some(max) = *self.history_retention.lock()
            && let Err(e) = db.history_enforce_retention(max)
        {
            warn!("Failed to enforce history retention: {e}");
        }
    }

    /// Persist current job progress to the database, including article-level
    /// checkpoint data for resume support.
    fn persist_job_progress(&self, job_id: &str) {
        let jobs = self.jobs.lock();
        if let Some(state) = jobs.get(job_id) {
            let db = self.db.lock();
            if let Err(e) = db.queue_update_progress(
                job_id,
                state.job.status,
                state.job.downloaded_bytes,
                state.job.articles_downloaded,
                state.job.articles_failed,
                state.job.files_completed,
            ) {
                warn!(job_id = %job_id, "Failed to persist progress: {e}");
            }

            // Build and store checkpoint of downloaded article segments
            let checkpoint = JobCheckpoint {
                files: state
                    .job
                    .files
                    .iter()
                    .map(|f| {
                        let downloaded_segments: Vec<u32> = f
                            .articles
                            .iter()
                            .filter(|a| a.downloaded)
                            .map(|a| a.segment_number)
                            .collect();
                        (f.id.clone(), downloaded_segments)
                    })
                    .collect(),
                downloaded_bytes: state.job.downloaded_bytes,
                articles_downloaded: state.job.articles_downloaded,
                articles_failed: state.job.articles_failed,
                files_completed: state.job.files_completed,
            };

            if let Ok(data) = serde_json::to_vec(&checkpoint)
                && let Err(e) = db.queue_store_job_data(job_id, &data)
            {
                warn!(job_id = %job_id, "Failed to persist checkpoint: {e}");
            }
        }
    }

    // -----------------------------------------------------------------------
    // Job control
    // -----------------------------------------------------------------------

    /// Change the priority of a specific job, reorder the queue, and preempt
    /// lower-priority downloads when a higher-priority job is waiting.
    pub fn set_job_priority(
        self: &Arc<Self>,
        id: &str,
        priority: Priority,
    ) -> crate::nzb_core::Result<()> {
        let max = self.max_active_downloads.load(Ordering::Relaxed);

        // 1. Update priority
        {
            let mut jobs = self.jobs.lock();
            let job_state = jobs
                .get_mut(id)
                .ok_or_else(|| crate::nzb_core::NzbError::JobNotFound(id.to_string()))?;
            job_state.job.priority = priority;
            let db = self.db.lock();
            db.queue_update_priority(id, priority as i32)?;
            info!(
                job_id = %id,
                ?priority,
                priority_val = priority as u8,
                max_active = max,
                "Job priority changed"
            );
        }

        // 2. Reorder job_order by priority (stable: preserves order within same priority)
        {
            let jobs = self.jobs.lock();
            let mut order = self.job_order.lock();
            let before: Vec<String> = order.clone();
            order.sort_by(|a, b| {
                let pa = jobs.get(a).map(|s| s.job.priority as u8).unwrap_or(0);
                let pb = jobs.get(b).map(|s| s.job.priority as u8).unwrap_or(0);
                pb.cmp(&pa) // descending: highest priority first
            });
            if *order != before {
                info!(
                    before = ?before.iter().take(6).collect::<Vec<_>>(),
                    after = ?order.iter().take(6).collect::<Vec<_>>(),
                    "Queue reordered by priority"
                );
            }
        }

        // 3. Preempt lower-priority downloads if a higher-priority queued job is waiting
        self.preempt_if_needed();

        Ok(())
    }

    /// Check whether a queued job has higher priority than a running download,
    /// and if so, pause the lower-priority download to make room.
    fn preempt_if_needed(self: &Arc<Self>) {
        let max = self.max_active_downloads.load(Ordering::Relaxed);

        loop {
            // Snapshot current state
            let (active, best_queued, worst_downloading) = {
                let jobs = self.jobs.lock();
                let order = self.job_order.lock();

                let active = jobs
                    .values()
                    .filter(|s| s.job.status == JobStatus::Downloading)
                    .count();

                let mut best_q: Option<(String, u8, String)> = None;
                let mut worst_d: Option<(String, u8, String)> = None;

                for id in order.iter() {
                    if let Some(s) = jobs.get(id) {
                        let p = s.job.priority as u8;
                        let name = s.job.name.clone();
                        if s.job.status == JobStatus::Queued
                            && best_q.as_ref().is_none_or(|(_, bp, _)| p > *bp)
                        {
                            best_q = Some((id.clone(), p, name));
                        } else if s.job.status == JobStatus::Downloading
                            && worst_d.as_ref().is_none_or(|(_, wp, _)| p < *wp)
                        {
                            worst_d = Some((id.clone(), p, name));
                        }
                    }
                }
                (active, best_q, worst_d)
            };

            // If unlimited slots or free slots available, just start queued jobs
            if max == 0 || active < max {
                self.start_next_queued();
                return;
            }

            // All slots full — check if preemption is warranted
            match (&best_queued, &worst_downloading) {
                (Some((q_id, q_pri, q_name)), Some((d_id, d_pri, d_name))) if q_pri > d_pri => {
                    info!(
                        preempted_id = %d_id,
                        preempted_name = %d_name,
                        preempted_priority = d_pri,
                        starting_id = %q_id,
                        starting_name = %q_name,
                        starting_priority = q_pri,
                        active_downloads = active,
                        max_downloads = max,
                        "Preempting lower-priority download for higher-priority job"
                    );

                    // Pause the lower-priority download
                    {
                        let mut jobs = self.jobs.lock();
                        if let Some(state) = jobs.get_mut(d_id.as_str()) {
                            state.engine.pause();
                            state.job.status = JobStatus::Paused;
                            // Persist to DB
                            let db = self.db.lock();
                            let _ = db.queue_update_progress(
                                d_id,
                                JobStatus::Paused,
                                state.job.downloaded_bytes,
                                state.job.articles_downloaded,
                                state.job.articles_failed,
                                state.job.files_completed,
                            );
                        }
                    }
                    // Loop back — active count decreased, start_next_queued will run
                }
                _ => {
                    info!(
                        active_downloads = active,
                        max_downloads = max,
                        best_queued_pri = best_queued.as_ref().map(|q| q.1),
                        worst_dl_pri = worst_downloading.as_ref().map(|d| d.1),
                        "No preemption needed"
                    );
                    return;
                }
            }
        }
    }

    /// Pause a specific job.
    pub fn pause_job(&self, id: &str) -> crate::nzb_core::Result<()> {
        let mut jobs = self.jobs.lock();
        let state = jobs
            .get_mut(id)
            .ok_or_else(|| crate::nzb_core::NzbError::JobNotFound(id.to_string()))?;

        state.job.status = JobStatus::Paused;
        state.engine.pause();

        let db = self.db.lock();
        db.queue_update_progress(
            id,
            JobStatus::Paused,
            state.job.downloaded_bytes,
            state.job.articles_downloaded,
            state.job.articles_failed,
            state.job.files_completed,
        )?;

        info!(job_id = %id, "Job paused");
        Ok(())
    }

    /// Resume a specific job.
    pub fn resume_job(self: &Arc<Self>, id: &str) -> crate::nzb_core::Result<()> {
        let needs_launch = {
            let mut jobs = self.jobs.lock();
            if !jobs.contains_key(id) {
                return Err(crate::nzb_core::NzbError::JobNotFound(id.to_string()));
            }

            let active = jobs
                .values()
                .filter(|s| s.job.status == JobStatus::Downloading)
                .count();

            let state = jobs.get_mut(id).unwrap();

            if state.task_handle.is_some() {
                // Engine is running, just resume it
                state.job.status = JobStatus::Downloading;
                state.engine.resume();
                let db = self.db.lock();
                let _ = db.queue_update_progress(
                    id,
                    JobStatus::Downloading,
                    state.job.downloaded_bytes,
                    state.job.articles_downloaded,
                    state.job.articles_failed,
                    state.job.files_completed,
                );
                false
            } else {
                // Check concurrency limit
                let max = self.max_active_downloads.load(Ordering::Relaxed);
                if max > 0 && active >= max {
                    state.job.status = JobStatus::Queued;
                    info!(job_id = %id, "Job queued (active download limit reached)");
                    false
                } else {
                    // Atomically mark as Downloading while holding the lock
                    state.job.status = JobStatus::Downloading;
                    state.job.error_message = None;
                    true
                }
            }
        };

        if needs_launch {
            self.launch_download(id);
        }

        info!(job_id = %id, "Job resumed");
        Ok(())
    }

    /// Remove a specific job from the queue.
    pub fn remove_job(&self, id: &str) -> crate::nzb_core::Result<()> {
        let removed = self.jobs.lock().remove(id);
        if let Some(state) = removed {
            // Cancel the download
            state.engine.cancel();
            if let Some(handle) = state.task_handle {
                handle.abort();
            }

            // Remove from DB
            let db = self.db.lock();
            let _ = db.queue_remove(id);

            // Remove from order
            self.job_order.lock().retain(|jid| jid != id);

            // Try to clean up work directory
            if state.job.work_dir.exists() {
                let _ = std::fs::remove_dir_all(&state.job.work_dir);
            }

            info!(job_id = %id, "Job removed");
        }
        Ok(())
    }

    /// Rename a job in the queue.
    pub fn rename_job(&self, id: &str, new_name: &str) -> crate::nzb_core::Result<()> {
        let mut jobs = self.jobs.lock();
        let state = jobs
            .iter_mut()
            .find(|(_, s)| s.job.id == id || s.job.id.starts_with(id));
        match state {
            Some((_, s)) => {
                s.job.name = new_name.to_string();
                info!(job_id = %id, new_name = %new_name, "Job renamed");
                Ok(())
            }
            None => Err(crate::nzb_core::NzbError::JobNotFound(id.to_string())),
        }
    }

    /// Change a job's category in the queue.
    pub fn change_job_category(&self, id: &str, category: &str) -> crate::nzb_core::Result<()> {
        let mut jobs = self.jobs.lock();
        let state = jobs
            .iter_mut()
            .find(|(_, s)| s.job.id == id || s.job.id.starts_with(id));
        match state {
            Some((_, s)) => {
                s.job.category = category.to_string();
                // Update the output directory to match the new category
                let complete_dir = self.complete_dir.join(category).join(&s.job.name);
                s.job.output_dir = complete_dir;
                info!(job_id = %id, category = %category, "Job category changed");
                Ok(())
            }
            None => Err(crate::nzb_core::NzbError::JobNotFound(id.to_string())),
        }
    }

    /// Move a job to a new position in the queue order.
    pub fn move_job(&self, id: &str, position: usize) -> crate::nzb_core::Result<()> {
        let mut order = self.job_order.lock();
        let current_pos = order
            .iter()
            .position(|x| x == id)
            .ok_or_else(|| crate::nzb_core::NzbError::JobNotFound(id.to_string()))?;
        let id_str = order.remove(current_pos);
        let new_pos = position.min(order.len());
        order.insert(new_pos, id_str);
        Ok(())
    }

    /// Pause all downloads globally.
    pub fn pause_all(&self) {
        self.globally_paused.store(true, Ordering::Relaxed);
        let mut jobs = self.jobs.lock();
        for (_id, state) in jobs.iter_mut() {
            if state.job.status == JobStatus::Downloading {
                state.engine.pause();
                state.job.status = JobStatus::Paused;
            }
        }
        info!("All downloads paused");
    }

    /// Pause all downloads for a specified duration.
    pub fn pause_for(self: &Arc<Self>, duration_secs: u64) {
        self.pause_all();
        let until = Utc::now() + chrono::Duration::seconds(duration_secs as i64);
        *self.pause_until.lock() = Some(until);

        let qm = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(duration_secs)).await;
            // Only auto-resume if the pause_until hasn't been cleared
            let should_resume = {
                let until = qm.pause_until.lock();
                until.is_some()
            };
            if should_resume {
                *qm.pause_until.lock() = None;
                qm.resume_all();
                info!("Auto-resumed after timed pause");
            }
        });

        info!(duration_secs, "Paused for duration");
    }

    /// Get remaining pause time in seconds (None if not timed).
    pub fn pause_remaining_secs(&self) -> Option<i64> {
        let until = self.pause_until.lock();
        until.map(|u| {
            let remaining = u - Utc::now();
            remaining.num_seconds().max(0)
        })
    }

    /// Resume all downloads globally.
    pub fn resume_all(self: &Arc<Self>) {
        self.globally_paused.store(false, Ordering::Relaxed);
        *self.pause_until.lock() = None;

        // Resume already-running paused engines and mark paused jobs as queued
        {
            let mut jobs = self.jobs.lock();
            for (_id, state) in jobs.iter_mut() {
                if state.job.status == JobStatus::Paused {
                    state.engine.resume();
                    if state.task_handle.is_some() {
                        // Engine is running, just resume it
                        state.job.status = JobStatus::Downloading;
                    } else {
                        // Needs a fresh start — mark as queued for start_next_queued
                        state.job.status = JobStatus::Queued;
                    }
                }
            }
        }

        // Start queued jobs up to the concurrency limit
        self.start_next_queued();

        info!("All downloads resumed");
    }

    // -----------------------------------------------------------------------
    // Server management
    // -----------------------------------------------------------------------

    /// Update the server list at runtime.
    pub fn update_servers(&self, servers: Vec<ServerConfig>) {
        *self.servers.lock() = servers;
    }

    /// Get current server configs.
    pub fn get_servers(&self) -> Vec<ServerConfig> {
        self.servers.lock().clone()
    }

    /// Access the database for newsgroup browsing operations.
    pub fn with_db<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Database) -> R,
    {
        let db = self.db.lock();
        f(&db)
    }

    // -----------------------------------------------------------------------
    // Query methods (for API handlers)
    // -----------------------------------------------------------------------

    /// Get a snapshot of all jobs in the queue.
    pub fn get_jobs(&self) -> Vec<NzbJob> {
        let jobs = self.jobs.lock();
        let order = self.job_order.lock();
        let mut result = Vec::with_capacity(order.len());
        for id in order.iter() {
            if let Some(state) = jobs.get(id) {
                let mut job = state.job.clone();
                job.speed_bps = state.speed.bps();
                result.push(job);
            }
        }
        result
    }

    /// Get the current download speed in bytes per second.
    pub fn get_speed(&self) -> u64 {
        self.speed.bps()
    }

    /// Check if downloads are globally paused.
    pub fn is_paused(&self) -> bool {
        self.globally_paused.load(Ordering::Relaxed)
    }

    /// Get the number of jobs in the queue.
    pub fn queue_size(&self) -> usize {
        self.jobs.lock().len()
    }

    /// Get a reference to the incomplete_dir.
    pub fn incomplete_dir(&self) -> &std::path::Path {
        &self.incomplete_dir
    }

    /// Get a reference to the complete_dir.
    pub fn complete_dir(&self) -> &std::path::Path {
        &self.complete_dir
    }

    /// Get the minimum free disk space threshold.
    pub fn min_free_space(&self) -> u64 {
        self.min_free_space
    }

    // -----------------------------------------------------------------------
    // History query methods (delegate to DB)
    // -----------------------------------------------------------------------

    /// List history entries.
    pub fn history_list(&self, limit: usize) -> crate::nzb_core::Result<Vec<HistoryEntry>> {
        let db = self.db.lock();
        db.history_list(limit)
    }

    /// Get a single history entry.
    pub fn history_get(&self, id: &str) -> crate::nzb_core::Result<Option<HistoryEntry>> {
        let db = self.db.lock();
        db.history_get(id)
    }

    /// Get raw NZB data for retry.
    pub fn history_get_nzb_data(&self, id: &str) -> crate::nzb_core::Result<Option<Vec<u8>>> {
        let db = self.db.lock();
        db.history_get_nzb_data(id)
    }

    /// Remove a history entry.
    pub fn history_remove(&self, id: &str) -> crate::nzb_core::Result<()> {
        let db = self.db.lock();
        db.history_remove(id)
    }

    /// Clear all history.
    pub fn history_clear(&self) -> crate::nzb_core::Result<()> {
        let db = self.db.lock();
        db.history_clear()
    }

    /// Get persisted logs for a history entry.
    pub fn history_get_logs(&self, id: &str) -> crate::nzb_core::Result<Option<String>> {
        let db = self.db.lock();
        db.history_get_logs(id)
    }

    // -----------------------------------------------------------------------
    // RSS item/rule query methods (delegate to DB)
    // -----------------------------------------------------------------------

    /// List RSS feed items.
    pub fn rss_items_list(
        &self,
        feed_name: Option<&str>,
        limit: usize,
    ) -> crate::nzb_core::Result<Vec<RssItem>> {
        let db = self.db.lock();
        db.rss_items_list(feed_name, limit)
    }

    /// Get a single RSS item by ID.
    pub fn rss_item_get(&self, id: &str) -> crate::nzb_core::Result<Option<RssItem>> {
        let db = self.db.lock();
        db.rss_item_get(id)
    }

    /// Mark an RSS item as downloaded.
    pub fn rss_item_mark_downloaded(
        &self,
        id: &str,
        category: Option<&str>,
    ) -> crate::nzb_core::Result<()> {
        let db = self.db.lock();
        db.rss_item_mark_downloaded(id, category)
    }

    /// Upsert an RSS feed item.
    pub fn rss_item_upsert(&self, item: &RssItem) -> crate::nzb_core::Result<()> {
        let db = self.db.lock();
        db.rss_item_upsert(item)
    }

    /// Batch upsert RSS feed items (single DB lock + transaction).
    pub fn rss_items_batch_upsert(&self, items: &[RssItem]) -> crate::nzb_core::Result<usize> {
        let db = self.db.lock();
        db.rss_items_batch_upsert(items)
    }

    /// Check if an RSS item exists.
    pub fn rss_item_exists(&self, id: &str) -> crate::nzb_core::Result<bool> {
        let db = self.db.lock();
        db.rss_item_exists(id)
    }

    /// Count total RSS items.
    pub fn rss_item_count(&self) -> crate::nzb_core::Result<usize> {
        let db = self.db.lock();
        db.rss_item_count()
    }

    /// Prune RSS items to keep only N most recent.
    pub fn rss_items_prune(&self, keep: usize) -> crate::nzb_core::Result<usize> {
        let db = self.db.lock();
        db.rss_items_prune(keep)
    }

    /// List all RSS download rules.
    pub fn rss_rule_list(&self) -> crate::nzb_core::Result<Vec<RssRule>> {
        let db = self.db.lock();
        db.rss_rule_list()
    }

    /// Insert a new RSS download rule.
    pub fn rss_rule_insert(&self, rule: &RssRule) -> crate::nzb_core::Result<()> {
        let db = self.db.lock();
        db.rss_rule_insert(rule)
    }

    /// Update an RSS download rule.
    pub fn rss_rule_update(&self, rule: &RssRule) -> crate::nzb_core::Result<()> {
        let db = self.db.lock();
        db.rss_rule_update(rule)
    }

    /// Delete an RSS download rule.
    pub fn rss_rule_delete(&self, id: &str) -> crate::nzb_core::Result<()> {
        let db = self.db.lock();
        db.rss_rule_delete(id)
    }

    // -----------------------------------------------------------------------
    // Startup: restore jobs from DB
    // -----------------------------------------------------------------------

    /// Restore in-progress jobs from the database on startup.
    ///
    /// Re-parses NZB data for each job and applies any saved checkpoint to
    /// mark already-downloaded articles, so downloads resume where they left off.
    pub fn restore_from_db(self: &Arc<Self>) -> crate::nzb_core::Result<()> {
        let jobs = {
            let db = self.db.lock();
            db.queue_list()?
        };

        if jobs.is_empty() {
            return Ok(());
        }

        info!(count = jobs.len(), "Restoring jobs from database");

        for mut job in jobs {
            let job_id = job.id.clone();
            let engine = Arc::new(DownloadEngine::new());

            // Try to load NZB data from DB
            let nzb_data = {
                let db = self.db.lock();
                db.queue_get_nzb_data(&job_id).unwrap_or(None)
            };

            // Re-parse NZB to populate files and articles
            if let Some(ref data) = nzb_data {
                match nzb_parser::parse_nzb(&job.name, data) {
                    Ok(parsed) => {
                        job.files = parsed.files;
                    }
                    Err(e) => {
                        warn!(job_id = %job_id, "Failed to re-parse NZB data: {e}");
                    }
                }
            }

            // Load and apply checkpoint to mark downloaded articles
            let checkpoint_data = {
                let db = self.db.lock();
                db.queue_load_job_data(&job_id).unwrap_or(None)
            };

            if let Some(ref data) = checkpoint_data {
                match serde_json::from_slice::<JobCheckpoint>(data) {
                    Ok(checkpoint) => {
                        // Restore progress counters from checkpoint
                        job.downloaded_bytes = checkpoint.downloaded_bytes;
                        job.articles_downloaded = checkpoint.articles_downloaded;
                        job.articles_failed = checkpoint.articles_failed;
                        job.files_completed = checkpoint.files_completed;

                        // Mark articles as downloaded based on checkpoint
                        for file in &mut job.files {
                            if let Some(segments) = checkpoint.files.get(&file.id) {
                                let mut file_bytes_downloaded: u64 = 0;
                                for article in &mut file.articles {
                                    if segments.contains(&article.segment_number) {
                                        article.downloaded = true;
                                        file_bytes_downloaded += article.bytes;
                                    }
                                }
                                file.bytes_downloaded = file_bytes_downloaded;

                                // Mark file as assembled if all articles are downloaded
                                if file.articles.iter().all(|a| a.downloaded) {
                                    file.assembled = true;
                                }
                            }
                        }

                        let remaining = job
                            .article_count
                            .saturating_sub(job.articles_downloaded + job.articles_failed);
                        info!(
                            job_id = %job_id,
                            name = %job.name,
                            articles_downloaded = job.articles_downloaded,
                            articles_failed = job.articles_failed,
                            remaining,
                            "Restored job checkpoint — resuming from previous progress"
                        );
                    }
                    Err(e) => {
                        warn!(
                            job_id = %job_id,
                            "Failed to deserialize checkpoint, starting from scratch: {e}"
                        );
                    }
                }
            }

            if job.status == JobStatus::Paused {
                engine.pause();
            } else if job.status == JobStatus::Downloading {
                // Mark as queued; start_next_queued will pick them up
                job.status = JobStatus::Queued;
            }

            let state = JobState {
                job,
                engine,
                task_handle: None,
                speed: Arc::new(SpeedTracker::new()),
                nzb_data,
            };
            self.jobs.lock().insert(job_id.clone(), state);
            self.job_order.lock().push(job_id);
        }

        // Start queued jobs up to the concurrency limit
        self.start_next_queued();

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Background task: speed calculation
    // -----------------------------------------------------------------------

    /// Spawn the background task that periodically updates the speed counter.
    /// Gracefully shut down the queue manager.
    ///
    /// Cancels all in-flight downloads, waits for tasks to stop, and persists
    /// final job state to the database so progress is not lost.
    pub async fn shutdown(&self) {
        info!("Shutting down queue manager...");

        // 1. Set globally paused to prevent new downloads from starting
        self.globally_paused.store(true, Ordering::Relaxed);

        // 2. Cancel all download engines and collect task handles
        let mut handles = Vec::new();
        {
            let mut jobs = self.jobs.lock();
            for (id, state) in jobs.iter_mut() {
                if state.job.status == JobStatus::Downloading {
                    info!(job_id = %id, "Cancelling download for shutdown");
                    state.engine.cancel();
                    state.job.status = JobStatus::Queued; // Will resume on restart
                }
                if let Some(handle) = state.task_handle.take() {
                    handles.push(handle);
                }
            }
        }

        // 3. Wait for all download tasks to finish (with timeout)
        if !handles.is_empty() {
            info!("Waiting for {} download tasks to stop...", handles.len());
            let timeout = Duration::from_secs(10);
            for handle in handles {
                let _ = tokio::time::timeout(timeout, handle).await;
            }
        }

        // 4. Persist final state for all jobs to DB
        {
            let jobs = self.jobs.lock();
            let db = self.db.lock();
            for (id, state) in jobs.iter() {
                if let Err(e) = db.queue_update_progress(
                    id,
                    state.job.status,
                    state.job.downloaded_bytes,
                    state.job.articles_downloaded,
                    state.job.articles_failed,
                    state.job.files_completed,
                ) {
                    error!(job_id = %id, error = %e, "Failed to persist job state on shutdown");
                }
            }
        }

        info!("Queue manager shutdown complete");
    }

    pub fn spawn_speed_tracker(self: &Arc<Self>) {
        let qm = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            let mut tick_count: u64 = 0;
            loop {
                interval.tick().await;
                qm.speed.tick(1.0);
                // Tick per-job speed trackers
                {
                    let jobs = qm.jobs.lock();
                    for (_id, state) in jobs.iter() {
                        state.speed.tick(1.0);
                    }
                }

                // Periodic disk space check (every 30 seconds)
                tick_count += 1;
                if tick_count.is_multiple_of(30) && qm.min_free_space > 0 {
                    let free = get_disk_free(&qm.incomplete_dir);
                    if free > 0
                        && free < qm.min_free_space
                        && !qm.globally_paused.load(Ordering::Relaxed)
                    {
                        warn!(
                            free_bytes = free,
                            min_free_space = qm.min_free_space,
                            "Low disk space, auto-pausing downloads"
                        );
                        qm.globally_paused.store(true, Ordering::Relaxed);
                        let jobs = qm.jobs.lock();
                        for (_id, state) in jobs.iter() {
                            if state.job.status == JobStatus::Downloading {
                                state.engine.pause();
                            }
                        }
                    }
                }
            }
        });
    }
}
