//! Queue manager — coordinates downloads across the application.
//!
//! The QueueManager owns the list of active NzbJobs, manages the download
//! engine instances, and exposes a thread-safe API for the HTTP handlers
//! to interact with.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use nzb_core::config::ServerConfig;
use nzb_core::db::Database;
use nzb_core::models::*;

use crate::download_engine::{DownloadEngine, ProgressUpdate};
use crate::log_buffer::LogBuffer;

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
}

impl QueueManager {
    /// Create a new queue manager.
    pub fn new(
        servers: Vec<ServerConfig>,
        db: Database,
        incomplete_dir: std::path::PathBuf,
        complete_dir: std::path::PathBuf,
        log_buffer: LogBuffer,
        max_active_downloads: usize,
    ) -> Arc<Self> {
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
        })
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

    /// Count currently downloading jobs.
    fn active_download_count(&self) -> usize {
        let jobs = self.jobs.lock();
        jobs.values()
            .filter(|s| s.job.status == JobStatus::Downloading)
            .count()
    }

    /// Start queued jobs up to the concurrency limit.
    fn start_next_queued(self: &Arc<Self>) {
        let max = self.max_active_downloads.load(Ordering::Relaxed);
        loop {
            let active = self.active_download_count();
            if max > 0 && active >= max {
                break;
            }

            // Find the first queued job (in order)
            let next = {
                let jobs = self.jobs.lock();
                let order = self.job_order.lock();
                order
                    .iter()
                    .find(|id| {
                        jobs.get(*id)
                            .map(|s| s.job.status == JobStatus::Queued)
                            .unwrap_or(false)
                    })
                    .cloned()
            };

            let Some(job_id) = next else { break };

            // Remove from jobs map and start download
            let job_data = {
                let mut jobs = self.jobs.lock();
                jobs.remove(&job_id).map(|s| (s.job, s.nzb_data))
            };

            if let Some((mut job, nzb_data)) = job_data {
                job.status = JobStatus::Downloading;
                info!(job_id = %job_id, name = %job.name, "Starting queued job");
                self.start_download(job, nzb_data);
            }
        }
    }

    /// Add a job to the queue and start downloading.
    ///
    /// The job should already have its `work_dir` and `output_dir` set.
    pub fn add_job(
        self: &Arc<Self>,
        mut job: NzbJob,
        nzb_data: Option<Vec<u8>>,
    ) -> nzb_core::Result<()> {
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

        // Check concurrency limit
        let max = self.max_active_downloads.load(Ordering::Relaxed);
        if max > 0 && self.active_download_count() >= max {
            // Queue the job instead of starting it immediately
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
            self.job_order.lock().push(job_id.clone());
            info!(job_id = %job_id, "Job queued (active download limit reached)");
            return Ok(());
        }

        // Start downloading
        job.status = JobStatus::Downloading;
        self.start_download(job, nzb_data);
        Ok(())
    }

    /// Start the download for a job.
    fn start_download(self: &Arc<Self>, job: NzbJob, nzb_data: Option<Vec<u8>>) {
        let job_id = job.id.clone();
        let engine = Arc::new(DownloadEngine::new());
        let job_speed = Arc::new(SpeedTracker::new());
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();

        let servers = self.servers.lock().clone();
        let engine_clone = Arc::clone(&engine);
        let job_clone = job.clone();

        // Spawn the download task
        let task_handle = tokio::spawn(async move {
            engine_clone.run(&job_clone, &servers, progress_tx).await;
        });

        let state = JobState {
            job,
            engine,
            task_handle: Some(task_handle),
            speed: Arc::clone(&job_speed),
            nzb_data,
        };

        self.jobs.lock().insert(job_id.clone(), state);
        {
            let mut order = self.job_order.lock();
            if !order.contains(&job_id) {
                order.push(job_id.clone());
            }
        }

        // Spawn the progress handler
        let qm = Arc::clone(self);
        tokio::spawn(async move {
            qm.handle_progress(job_id, progress_rx, job_speed).await;
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
                                if let Some(ss) = stats.iter_mut().find(|s| s.server_id == *sid)
                                {
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
                    self.on_job_finished(&job_id, success, articles_failed);
                    break;
                }
            }
        }
    }

    /// Called when a job's download phase completes.
    fn on_job_finished(self: &Arc<Self>, job_id: &str, success: bool, articles_failed: usize) {
        // Complete the job and move to history
        {
            let mut jobs = self.jobs.lock();
            if let Some(state) = jobs.get_mut(job_id) {
                if success {
                    state.job.status = JobStatus::PostProcessing;
                    state.job.completed_at = Some(chrono::Utc::now());
                    info!(job_id = %job_id, "Job moving to post-processing");
                } else {
                    state.job.status = JobStatus::Failed;
                    state.job.completed_at = Some(chrono::Utc::now());
                    state.job.error_message = Some(format!(
                        "{articles_failed} article(s) failed to download"
                    ));
                }

                self.move_to_history(state);
            }
        }

        // Persist final state then remove from active queue
        self.persist_job_progress(job_id);

        // Remove from in-memory queue
        self.jobs.lock().remove(job_id);
        self.job_order.lock().retain(|jid| jid != job_id);

        // Start next queued job(s) now that a slot is free
        self.start_next_queued();
    }

    /// Move a job's files to output and insert a history entry.
    fn move_to_history(&self, state: &mut JobState) {
        let final_status = if state.job.articles_failed == 0 {
            JobStatus::Completed
        } else {
            JobStatus::Failed
        };

        // Try to move files from work_dir to output_dir
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

        state.job.status = final_status;

        // Insert into history
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
            stages: Vec::new(),
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
        if let Some(max) = *self.history_retention.lock() {
            if let Err(e) = db.history_enforce_retention(max) {
                warn!("Failed to enforce history retention: {e}");
            }
        }
    }

    /// Persist current job progress to the database.
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
        }
    }

    // -----------------------------------------------------------------------
    // Job control
    // -----------------------------------------------------------------------

    /// Pause a specific job.
    pub fn pause_job(&self, id: &str) -> nzb_core::Result<()> {
        let mut jobs = self.jobs.lock();
        let state = jobs
            .get_mut(id)
            .ok_or_else(|| nzb_core::NzbError::JobNotFound(id.to_string()))?;

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
    pub fn resume_job(self: &Arc<Self>, id: &str) -> nzb_core::Result<()> {
        let needs_start = {
            let mut jobs = self.jobs.lock();
            // Check existence first
            if !jobs.contains_key(id) {
                return Err(nzb_core::NzbError::JobNotFound(id.to_string()));
            }

            // Count active downloads before taking a mutable borrow
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
                None
            } else {
                // Check concurrency limit
                let max = self.max_active_downloads.load(Ordering::Relaxed);
                if max > 0 && active >= max {
                    // Mark as queued — will start when a slot opens
                    state.job.status = JobStatus::Queued;
                    info!(job_id = %id, "Job queued (active download limit reached)");
                    None
                } else {
                    Some((state.job.clone(), state.nzb_data.clone()))
                }
            }
        };

        if let Some((mut job, nzb_data)) = needs_start {
            // Remove old state and start fresh
            self.jobs.lock().remove(&job.id);
            job.status = JobStatus::Downloading;
            self.start_download(job, nzb_data);
        }

        info!(job_id = %id, "Job resumed");
        Ok(())
    }

    /// Remove a specific job from the queue.
    pub fn remove_job(&self, id: &str) -> nzb_core::Result<()> {
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

    /// Pause all downloads globally.
    pub fn pause_all(&self) {
        self.globally_paused.store(true, Ordering::Relaxed);
        let jobs = self.jobs.lock();
        for (_id, state) in jobs.iter() {
            if state.job.status == JobStatus::Downloading {
                state.engine.pause();
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

    // -----------------------------------------------------------------------
    // History query methods (delegate to DB)
    // -----------------------------------------------------------------------

    /// List history entries.
    pub fn history_list(&self, limit: usize) -> nzb_core::Result<Vec<HistoryEntry>> {
        let db = self.db.lock();
        db.history_list(limit).map_err(Into::into)
    }

    /// Get a single history entry.
    pub fn history_get(&self, id: &str) -> nzb_core::Result<Option<HistoryEntry>> {
        let db = self.db.lock();
        db.history_get(id).map_err(Into::into)
    }

    /// Get raw NZB data for retry.
    pub fn history_get_nzb_data(&self, id: &str) -> nzb_core::Result<Option<Vec<u8>>> {
        let db = self.db.lock();
        db.history_get_nzb_data(id).map_err(Into::into)
    }

    /// Remove a history entry.
    pub fn history_remove(&self, id: &str) -> nzb_core::Result<()> {
        let db = self.db.lock();
        db.history_remove(id).map_err(Into::into)
    }

    /// Clear all history.
    pub fn history_clear(&self) -> nzb_core::Result<()> {
        let db = self.db.lock();
        db.history_clear().map_err(Into::into)
    }

    /// Get persisted logs for a history entry.
    pub fn history_get_logs(&self, id: &str) -> nzb_core::Result<Option<String>> {
        let db = self.db.lock();
        db.history_get_logs(id).map_err(Into::into)
    }

    // -----------------------------------------------------------------------
    // Startup: restore jobs from DB
    // -----------------------------------------------------------------------

    /// Restore in-progress jobs from the database on startup.
    pub fn restore_from_db(self: &Arc<Self>) -> nzb_core::Result<()> {
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
    pub fn spawn_speed_tracker(self: &Arc<Self>) {
        let qm = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                qm.speed.tick(1.0);
                // Tick per-job speed trackers
                let jobs = qm.jobs.lock();
                for (_id, state) in jobs.iter() {
                    state.speed.tick(1.0);
                }
            }
        });
    }
}
