//! Download engine — fetches articles via NNTP, decodes yEnc, assembles files.
//!
//! Retry logic:
//! 1. Try article on current server (up to MAX_TRIES_PER_SERVER attempts with reconnect)
//! 2. On ArticleNotFound (430) — immediately try next server (no local retry)
//! 3. On connection error — reconnect same server, re-queue article
//! 4. On decode error — try next server (data may differ)
//! 5. Only mark article as failed after ALL servers exhausted
//! 6. Job continues even with failed articles (par2 can repair)
//! 7. Job only fails if failed articles exceed threshold AND no par2 recovery possible

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use nzb_core::config::ServerConfig;
use nzb_core::models::NzbJob;
use nzb_decode::FileAssembler;
use nzb_decode::yenc::decode_yenc;
use nzb_nntp::Pipeline;
use nzb_nntp::connection::NntpConnection;
use nzb_nntp::error::NntpError;

use crate::bandwidth::BandwidthLimiter;
use crate::util::normalize_nfc;

/// Max times to retry an article on the SAME server before trying the next.
const MAX_TRIES_PER_SERVER: u32 = 3;
/// Delay between reconnection attempts.
const RECONNECT_DELAY: Duration = Duration::from_secs(5);
/// Max reconnect attempts before giving up on a server for this session.
const MAX_RECONNECT_ATTEMPTS: u32 = 5;

// ---------------------------------------------------------------------------
// Progress update messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ProgressUpdate {
    /// An article was successfully downloaded and decoded.
    ArticleComplete {
        job_id: String,
        file_id: String,
        segment_number: u32,
        decoded_bytes: u64,
        file_complete: bool,
        server_id: Option<String>,
    },
    /// An article failed on all servers (counted as bad/missing).
    ArticleFailed {
        job_id: String,
        file_id: String,
        segment_number: u32,
        error: String,
        server_id: Option<String>,
    },
    /// The entire job has finished (all articles processed).
    JobFinished {
        job_id: String,
        success: bool,
        articles_failed: usize,
    },
    /// No servers could be reached — job should be paused, not moved to history.
    NoServersAvailable { job_id: String, reason: String },
}

// ---------------------------------------------------------------------------
// Work item
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct WorkItem {
    job_id: String,
    file_id: String,
    filename: String,
    message_id: String,
    segment_number: u32,
    /// Servers already tried for this article (by server ID).
    tried_servers: Vec<String>,
    /// Number of attempts on the current server.
    tries_on_current: u32,
}

// ---------------------------------------------------------------------------
// Download engine
// ---------------------------------------------------------------------------

pub struct DownloadEngine {
    cancelled: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    /// Cumulative yEnc decode time across all workers (microseconds).
    pub total_decode_us: Arc<AtomicU64>,
    /// Cumulative file assembly time across all workers (microseconds).
    pub total_assemble_us: Arc<AtomicU64>,
    /// Total articles successfully decoded.
    pub total_articles_decoded: Arc<AtomicU64>,
}

impl Default for DownloadEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl DownloadEngine {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
            total_decode_us: Arc::new(AtomicU64::new(0)),
            total_assemble_us: Arc::new(AtomicU64::new(0)),
            total_articles_decoded: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Run the download engine for a single job.
    pub async fn run(
        &self,
        job: &NzbJob,
        servers: &[ServerConfig],
        progress_tx: mpsc::UnboundedSender<ProgressUpdate>,
        bandwidth: Arc<BandwidthLimiter>,
    ) {
        let job_id = job.id.clone();
        let engine_start = Instant::now();

        // Build work queue from all unfinished articles
        let work_items: Vec<WorkItem> = job
            .files
            .iter()
            .flat_map(|file| {
                file.articles
                    .iter()
                    .filter(|a| !a.downloaded)
                    .map(move |article| WorkItem {
                        job_id: job.id.clone(),
                        file_id: file.id.clone(),
                        filename: file.filename.clone(),
                        message_id: article.message_id.clone(),
                        segment_number: article.segment_number,
                        tried_servers: Vec::new(),
                        tries_on_current: 0,
                    })
            })
            .collect();

        if work_items.is_empty() {
            let _ = progress_tx.send(ProgressUpdate::JobFinished {
                job_id,
                success: true,
                articles_failed: 0,
            });
            return;
        }

        info!(
            job_id = %job_id,
            articles = work_items.len(),
            "Starting download engine"
        );

        // Create file assembler and register all files
        let assembler = Arc::new(FileAssembler::new());
        for file in &job.files {
            let output_path = job.work_dir.join(&file.filename);
            if let Err(e) =
                assembler.register_file(&job.id, &file.id, output_path, file.articles.len() as u32)
            {
                error!(file = %file.filename, "Failed to register file for assembly: {e}");
                let _ = progress_tx.send(ProgressUpdate::JobFinished {
                    job_id,
                    success: false,
                    articles_failed: work_items.len(),
                });
                return;
            }
        }

        // Shared work queue
        let work_queue = Arc::new(Mutex::new(VecDeque::from(work_items)));
        let articles_failed = Arc::new(AtomicUsize::new(0));

        // Reset cumulative decode timing counters for this job
        self.total_decode_us.store(0, Ordering::Relaxed);
        self.total_assemble_us.store(0, Ordering::Relaxed);
        self.total_articles_decoded.store(0, Ordering::Relaxed);

        // Track yEnc-derived real filenames for deobfuscation.
        // NZB subjects may be obfuscated (random hashes), but yEnc headers
        // contain the actual filename. We capture these during download and
        // rename files before post-processing runs.
        let nzb_filenames: HashMap<String, String> = job
            .files
            .iter()
            .map(|f| (f.id.clone(), f.filename.clone()))
            .collect();
        let yenc_names: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));

        // Sort servers by priority, filter enabled
        let mut sorted_servers: Vec<ServerConfig> =
            servers.iter().filter(|s| s.enabled).cloned().collect();
        sorted_servers.sort_by_key(|s| s.priority);

        if sorted_servers.is_empty() {
            error!(job_id = %job_id, "No enabled servers configured");
            let _ = progress_tx.send(ProgressUpdate::NoServersAvailable {
                job_id,
                reason: "No enabled servers configured".into(),
            });
            return;
        }

        // Pre-flight: verify at least one server is reachable before spawning
        // workers.  This prevents the scenario where all workers instantly fail
        // and every article is marked as failed.
        {
            let mut any_ok = false;
            for server in &sorted_servers {
                info!(
                    job_id = %job_id,
                    server = %server.name,
                    host = %server.host,
                    port = server.port,
                    ssl = server.ssl,
                    "Pre-flight: testing server connectivity"
                );
                let mut conn = NntpConnection::new(format!("preflight-{}", server.id));
                match tokio::time::timeout(Duration::from_secs(15), conn.connect(server)).await {
                    Ok(Ok(())) => {
                        info!(
                            job_id = %job_id,
                            server = %server.name,
                            "Pre-flight: server OK"
                        );
                        let _ = conn.quit().await;
                        any_ok = true;
                        break;
                    }
                    Ok(Err(e)) => {
                        warn!(
                            job_id = %job_id,
                            server = %server.name,
                            error = %e,
                            "Pre-flight: server connection failed"
                        );
                    }
                    Err(_) => {
                        warn!(
                            job_id = %job_id,
                            server = %server.name,
                            "Pre-flight: server connection timed out (15s)"
                        );
                    }
                }
            }

            if !any_ok {
                error!(
                    job_id = %job_id,
                    servers_tested = sorted_servers.len(),
                    "All servers failed pre-flight connectivity check — pausing job"
                );
                let _ = progress_tx.send(ProgressUpdate::NoServersAvailable {
                    job_id,
                    reason: format!(
                        "All {} server(s) failed connectivity check",
                        sorted_servers.len()
                    ),
                });
                return;
            }
        }

        // Spawn worker tasks: one per connection slot per server
        let mut worker_handles = Vec::new();

        for server in &sorted_servers {
            let num_conns = server.connections.min(50) as usize;
            for conn_idx in 0..num_conns {
                let handle = tokio::spawn({
                    let server_config = server.clone();
                    let work_queue = Arc::clone(&work_queue);
                    let assembler = Arc::clone(&assembler);
                    let progress_tx = progress_tx.clone();
                    let cancelled = Arc::clone(&self.cancelled);
                    let paused = Arc::clone(&self.paused);
                    let articles_failed = Arc::clone(&articles_failed);
                    let all_servers = sorted_servers.clone();
                    let yenc_names = Arc::clone(&yenc_names);
                    let total_decode_us = Arc::clone(&self.total_decode_us);
                    let total_assemble_us = Arc::clone(&self.total_assemble_us);
                    let total_articles_decoded = Arc::clone(&self.total_articles_decoded);
                    let bandwidth = Arc::clone(&bandwidth);

                    async move {
                        download_worker(
                            server_config,
                            conn_idx,
                            work_queue,
                            assembler,
                            progress_tx,
                            cancelled,
                            paused,
                            articles_failed,
                            all_servers,
                            yenc_names,
                            total_decode_us,
                            total_assemble_us,
                            total_articles_decoded,
                            bandwidth,
                        )
                        .await;
                    }
                });
                worker_handles.push(handle);
            }
        }

        // Wait for all workers
        for handle in worker_handles {
            let _ = handle.await;
        }

        let download_elapsed = engine_start.elapsed();
        let total_bytes = job.total_bytes;
        let throughput_mbps = if download_elapsed.as_secs_f64() > 0.001 {
            (total_bytes as f64 / download_elapsed.as_secs_f64()) / (1024.0 * 1024.0)
        } else {
            0.0
        };
        let decode_total_us = self.total_decode_us.load(Ordering::Relaxed);
        let assemble_total_us = self.total_assemble_us.load(Ordering::Relaxed);
        let articles_decoded = self.total_articles_decoded.load(Ordering::Relaxed);

        info!(
            job_id = %job_id,
            elapsed_secs = download_elapsed.as_secs_f64(),
            total_bytes,
            throughput_mbps = format!("{throughput_mbps:.2}"),
            "Download phase complete"
        );
        info!(
            job_id = %job_id,
            articles_decoded,
            decode_secs = format!("{:.3}", decode_total_us as f64 / 1_000_000.0),
            assemble_secs = format!("{:.3}", assemble_total_us as f64 / 1_000_000.0),
            decode_pct = format!("{:.1}", decode_total_us as f64 / download_elapsed.as_micros() as f64 * 100.0),
            assemble_pct = format!("{:.1}", assemble_total_us as f64 / download_elapsed.as_micros() as f64 * 100.0),
            "Decode timing summary (cumulative across all workers)"
        );

        // Drain any remaining items (stuck because needed servers exited)
        let remaining: Vec<WorkItem> = work_queue.lock().drain(..).collect();
        for item in remaining {
            articles_failed.fetch_add(1, Ordering::Relaxed);
            warn!(
                article = %item.message_id,
                "Article could not be downloaded — no available server"
            );
            let _ = progress_tx.send(ProgressUpdate::ArticleFailed {
                job_id: item.job_id,
                file_id: item.file_id,
                segment_number: item.segment_number,
                error: "No available server could download this article".into(),
                server_id: None,
            });
        }

        // Deobfuscate: choose the best filename between NZB subject and yEnc header.
        // Either source may be obfuscated (random hashes without extensions).
        // We always rename toward the name that has a recognizable file extension.
        {
            let renames = yenc_names.lock();
            for (file_id, yenc_name) in renames.iter() {
                if let Some(nzb_name) = nzb_filenames.get(file_id) {
                    if nzb_name == yenc_name {
                        continue;
                    }
                    // Sanitize: strip any path components from yEnc name
                    let clean_yenc = std::path::Path::new(yenc_name.as_str())
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(yenc_name);
                    if clean_yenc.is_empty() || nzb_name == clean_yenc {
                        continue;
                    }

                    let nzb_has_ext = has_known_extension(nzb_name);
                    let yenc_has_ext = has_known_extension(clean_yenc);

                    // Determine rename direction: always rename FROM obfuscated TO readable.
                    // If yEnc has a real extension but NZB doesn't → rename NZB→yEnc (original logic)
                    // If NZB has a real extension but yEnc doesn't → skip (NZB name is already good)
                    // If both have extensions → prefer yEnc (original logic)
                    // If neither has extensions → skip (nothing to improve)
                    let (old_name, new_name) = if yenc_has_ext && !nzb_has_ext {
                        (nzb_name.as_str(), clean_yenc)
                    } else if nzb_has_ext && !yenc_has_ext {
                        // NZB name is already the good one — skip rename
                        continue;
                    } else if yenc_has_ext && nzb_has_ext {
                        // Both have extensions — prefer yEnc (standard deobfuscation)
                        (nzb_name.as_str(), clean_yenc)
                    } else {
                        // Neither has a recognizable extension — skip
                        continue;
                    };

                    let old_path = job.work_dir.join(old_name);
                    let new_path = job.work_dir.join(new_name);
                    if old_path.exists() && !new_path.exists() {
                        if let Err(e) = std::fs::rename(&old_path, &new_path) {
                            warn!(
                                job_id = %job_id,
                                from = %old_name,
                                to = %new_name,
                                "Failed to deobfuscate file: {e}"
                            );
                        } else {
                            info!(
                                job_id = %job_id,
                                from = %old_name,
                                to = %new_name,
                                "Deobfuscated file"
                            );
                        }
                    }
                }
            }
        }

        let failed_count = articles_failed.load(Ordering::Relaxed);
        let _ = progress_tx.send(ProgressUpdate::JobFinished {
            job_id,
            success: failed_count == 0,
            articles_failed: failed_count,
        });
    }
}

// ---------------------------------------------------------------------------
// Worker task
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn download_worker(
    primary_server: ServerConfig,
    conn_idx: usize,
    work_queue: Arc<Mutex<VecDeque<WorkItem>>>,
    assembler: Arc<FileAssembler>,
    progress_tx: mpsc::UnboundedSender<ProgressUpdate>,
    cancelled: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    articles_failed: Arc<AtomicUsize>,
    all_servers: Vec<ServerConfig>,
    yenc_names: Arc<Mutex<HashMap<String, String>>>,
    total_decode_us: Arc<AtomicU64>,
    total_assemble_us: Arc<AtomicU64>,
    total_articles_decoded: Arc<AtomicU64>,
    bandwidth: Arc<BandwidthLimiter>,
) {
    let worker_id = format!("{}#{}", primary_server.id, conn_idx);

    // Connect to primary server
    let mut conn = NntpConnection::new(worker_id.clone());
    if let Err(e) = connect_with_retry(&mut conn, &primary_server, &worker_id).await {
        warn!(worker = %worker_id, server = %primary_server.name, "Failed to connect after retries: {e}");
        return;
    }

    let pipe_depth = primary_server.pipelining.max(1);
    debug!(
        worker = %worker_id,
        server = %primary_server.host,
        pipelining = pipe_depth,
        "Worker connected"
    );

    // Use non-pipelined path for depth 1 (simpler, avoids pipeline overhead)
    if pipe_depth <= 1 {
        download_worker_serial(
            &mut conn,
            &primary_server,
            &worker_id,
            &work_queue,
            &assembler,
            &progress_tx,
            &cancelled,
            &paused,
            &articles_failed,
            &all_servers,
            &yenc_names,
            &total_decode_us,
            &total_assemble_us,
            &total_articles_decoded,
            &bandwidth,
        )
        .await;
    } else {
        download_worker_pipelined(
            &mut conn,
            &primary_server,
            &worker_id,
            pipe_depth,
            &work_queue,
            &assembler,
            &progress_tx,
            &cancelled,
            &paused,
            &articles_failed,
            &all_servers,
            &yenc_names,
            &total_decode_us,
            &total_assemble_us,
            &total_articles_decoded,
            &bandwidth,
        )
        .await;
    }

    let _ = conn.quit().await;
}

/// Pipelined download: sends multiple ARTICLE commands before reading responses.
#[allow(clippy::too_many_arguments)]
async fn download_worker_pipelined(
    conn: &mut NntpConnection,
    primary_server: &ServerConfig,
    worker_id: &str,
    pipe_depth: u8,
    work_queue: &Arc<Mutex<VecDeque<WorkItem>>>,
    assembler: &Arc<FileAssembler>,
    progress_tx: &mpsc::UnboundedSender<ProgressUpdate>,
    cancelled: &Arc<AtomicBool>,
    paused: &Arc<AtomicBool>,
    articles_failed: &Arc<AtomicUsize>,
    all_servers: &[ServerConfig],
    yenc_names: &Arc<Mutex<HashMap<String, String>>>,
    total_decode_us: &Arc<AtomicU64>,
    total_assemble_us: &Arc<AtomicU64>,
    total_articles_decoded: &Arc<AtomicU64>,
    bandwidth: &Arc<BandwidthLimiter>,
) {
    let mut pipeline = Pipeline::new(pipe_depth);
    // In-flight items indexed by pipeline tag
    let mut in_flight_items: HashMap<u64, WorkItem> = HashMap::new();
    let mut next_tag: u64 = 0;
    let mut consecutive_errors: u32 = 0;

    loop {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }

        // Wait while paused
        while paused.load(Ordering::Relaxed) && !cancelled.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        if cancelled.load(Ordering::Relaxed) {
            break;
        }

        // Fill the pipeline with work items
        while pipeline.pending_count() + pipeline.in_flight_count() < pipe_depth as usize {
            let item = { work_queue.lock().pop_front() };
            let Some(item) = item else {
                break;
            };

            // Skip if this worker's server was already tried
            if item.tried_servers.contains(&primary_server.id) {
                work_queue.lock().push_back(item);
                continue;
            }

            let tag = next_tag;
            next_tag += 1;
            pipeline.submit(item.message_id.clone(), tag);
            in_flight_items.insert(tag, item);
        }

        // If nothing to do, we're done
        if pipeline.is_empty() && in_flight_items.is_empty() {
            debug!(worker = %worker_id, "Pipeline empty, work queue exhausted, exiting");
            break;
        }

        // Flush pending sends
        if let Err(e) = pipeline.flush_sends(conn).await {
            warn!(worker = %worker_id, server = %primary_server.name, "Pipeline send error: {e}");
            // Re-queue all in-flight items
            requeue_all(&mut in_flight_items, work_queue);
            // Try reconnect
            consecutive_errors += 1;
            if consecutive_errors > MAX_RECONNECT_ATTEMPTS {
                warn!(worker = %worker_id, server = %primary_server.name, "Too many pipeline errors, exiting");
                break;
            }
            tokio::time::sleep(RECONNECT_DELAY).await;
            *conn = NntpConnection::new(worker_id.to_string());
            if let Err(e) = connect_with_retry(conn, primary_server, worker_id).await {
                warn!(worker = %worker_id, server = %primary_server.name, "Reconnect failed: {e}");
                break;
            }
            pipeline = Pipeline::new(pipe_depth);
            continue;
        }

        // Read one response
        let result = pipeline.receive_one(conn).await;
        match result {
            Ok(Some(pipe_result)) => {
                let Some(mut item) = in_flight_items.remove(&pipe_result.request.tag) else {
                    continue;
                };

                match pipe_result.result {
                    Ok(response) => {
                        consecutive_errors = 0;
                        let raw_data = response.data.unwrap_or_default();
                        // Yield to the runtime so other tasks (HTTP server, etc.)
                        // get scheduled between CPU-bound decode+assemble work.
                        tokio::task::yield_now().await;
                        match decode_and_assemble(&item, &raw_data, assembler) {
                            Ok(process_result) => {
                                total_decode_us
                                    .fetch_add(process_result.decode_us, Ordering::Relaxed);
                                total_assemble_us
                                    .fetch_add(process_result.assemble_us, Ordering::Relaxed);
                                total_articles_decoded.fetch_add(1, Ordering::Relaxed);
                                if let Some(ref yname) = process_result.yenc_filename {
                                    yenc_names
                                        .lock()
                                        .entry(item.file_id.clone())
                                        .or_insert_with(|| normalize_nfc(yname));
                                }
                                // Throttle via bandwidth limiter
                                if let Some(n) =
                                    std::num::NonZeroU32::new(process_result.decoded_bytes as u32)
                                {
                                    let _ = bandwidth.acquire_download(n).await;
                                }
                                let _ = progress_tx.send(ProgressUpdate::ArticleComplete {
                                    job_id: item.job_id.clone(),
                                    file_id: item.file_id.clone(),
                                    segment_number: item.segment_number,
                                    decoded_bytes: process_result.decoded_bytes,
                                    file_complete: process_result.file_complete,
                                    server_id: Some(primary_server.id.clone()),
                                });
                            }
                            Err(ArticleError::DecodeError(msg)) => {
                                handle_article_not_available(
                                    &mut item,
                                    primary_server,
                                    all_servers,
                                    articles_failed,
                                    work_queue,
                                    progress_tx,
                                    &format!("Decode error: {msg}"),
                                );
                            }
                            Err(ArticleError::AssemblyError(msg)) => {
                                articles_failed.fetch_add(1, Ordering::Relaxed);
                                error!(article = %item.message_id, "Assembly error: {msg}");
                                let _ = progress_tx.send(ProgressUpdate::ArticleFailed {
                                    job_id: item.job_id.clone(),
                                    file_id: item.file_id.clone(),
                                    segment_number: item.segment_number,
                                    error: format!("Assembly error: {msg}"),
                                    server_id: Some(primary_server.id.clone()),
                                });
                            }
                            Err(_) => {}
                        }
                    }
                    Err(NntpError::ArticleNotFound(_)) => {
                        handle_article_not_available(
                            &mut item,
                            primary_server,
                            all_servers,
                            articles_failed,
                            work_queue,
                            progress_tx,
                            "Article not found on any server",
                        );
                    }
                    Err(NntpError::Connection(_) | NntpError::Io(_)) => {
                        // Connection lost — requeue this item and all remaining in-flight
                        work_queue.lock().push_front(item);
                        requeue_all(&mut in_flight_items, work_queue);
                        consecutive_errors += 1;
                        if consecutive_errors > MAX_RECONNECT_ATTEMPTS {
                            warn!(worker = %worker_id, server = %primary_server.name, "Too many errors, exiting");
                            break;
                        }
                        tokio::time::sleep(RECONNECT_DELAY).await;
                        *conn = NntpConnection::new(worker_id.to_string());
                        if let Err(e) = connect_with_retry(conn, primary_server, worker_id).await {
                            warn!(worker = %worker_id, server = %primary_server.name, "Reconnect failed: {e}");
                            break;
                        }
                        pipeline = Pipeline::new(pipe_depth);
                        continue;
                    }
                    Err(e) => {
                        warn!(worker = %worker_id, article = %item.message_id, "Pipeline error: {e}");
                        handle_article_not_available(
                            &mut item,
                            primary_server,
                            all_servers,
                            articles_failed,
                            work_queue,
                            progress_tx,
                            &format!("Pipeline error: {e}"),
                        );
                    }
                }
            }
            Ok(None) => {
                // No in-flight requests — loop will either fill more or exit
            }
            Err(e) => {
                warn!(worker = %worker_id, server = %primary_server.name, "Pipeline receive error: {e}");
                requeue_all(&mut in_flight_items, work_queue);
                consecutive_errors += 1;
                if consecutive_errors > MAX_RECONNECT_ATTEMPTS {
                    break;
                }
                tokio::time::sleep(RECONNECT_DELAY).await;
                *conn = NntpConnection::new(worker_id.to_string());
                if let Err(e) = connect_with_retry(conn, primary_server, worker_id).await {
                    warn!(worker = %worker_id, server = %primary_server.name, "Reconnect failed: {e}");
                    break;
                }
                pipeline = Pipeline::new(pipe_depth);
            }
        }
    }
}

/// Check if a filename has a known file extension (archive, video, par2, etc.).
/// Used to distinguish real filenames from obfuscated hashes during deobfuscation.
fn has_known_extension(name: &str) -> bool {
    let lower = name.to_lowercase();
    // Check for dot-separated extension
    if let Some(dot_pos) = lower.rfind('.') {
        let ext = &lower[dot_pos + 1..];
        matches!(
            ext,
            // Archives
            "rar" | "r00" | "r01" | "r02" | "r03" | "r04" | "r05"
            | "zip" | "7z" | "gz" | "bz2" | "xz" | "tar"
            // Video
            | "mkv" | "mp4" | "avi" | "wmv" | "ts" | "m4v" | "mov" | "mpg" | "mpeg"
            // Audio
            | "mp3" | "flac" | "ogg" | "m4a" | "aac" | "wav"
            // Subtitles
            | "srt" | "sub" | "idx" | "ass" | "ssa" | "sup"
            // Images
            | "nfo" | "jpg" | "jpeg" | "png" | "gif" | "bmp"
            // PAR2
            | "par2"
            // NZB split archive patterns (e.g., .001, .002)
            | "001" | "002" | "003" | "004" | "005"
        )
    } else {
        false
    }
}

/// Handle an article that's not available on this server (not found or decode error).
fn handle_article_not_available(
    item: &mut WorkItem,
    primary_server: &ServerConfig,
    all_servers: &[ServerConfig],
    articles_failed: &Arc<AtomicUsize>,
    work_queue: &Arc<Mutex<VecDeque<WorkItem>>>,
    progress_tx: &mpsc::UnboundedSender<ProgressUpdate>,
    error_msg: &str,
) {
    item.tried_servers.push(primary_server.id.clone());
    item.tries_on_current = 0;

    let all_tried = all_servers
        .iter()
        .all(|s| item.tried_servers.contains(&s.id));

    if all_tried {
        articles_failed.fetch_add(1, Ordering::Relaxed);
        warn!(article = %item.message_id, "{error_msg}");
        let _ = progress_tx.send(ProgressUpdate::ArticleFailed {
            job_id: item.job_id.clone(),
            file_id: item.file_id.clone(),
            segment_number: item.segment_number,
            error: error_msg.to_string(),
            server_id: Some(primary_server.id.clone()),
        });
    } else {
        work_queue.lock().push_back(item.clone());
    }
}

/// Re-queue all in-flight items back to the work queue (on connection loss).
fn requeue_all(
    in_flight: &mut HashMap<u64, WorkItem>,
    work_queue: &Arc<Mutex<VecDeque<WorkItem>>>,
) {
    let items: Vec<WorkItem> = in_flight.drain().map(|(_, item)| item).collect();
    if !items.is_empty() {
        let mut q = work_queue.lock();
        for item in items {
            q.push_front(item);
        }
    }
}

/// Serial (non-pipelined) download path — used when pipelining depth is 1.
#[allow(clippy::too_many_arguments)]
async fn download_worker_serial(
    conn: &mut NntpConnection,
    primary_server: &ServerConfig,
    worker_id: &str,
    work_queue: &Arc<Mutex<VecDeque<WorkItem>>>,
    assembler: &Arc<FileAssembler>,
    progress_tx: &mpsc::UnboundedSender<ProgressUpdate>,
    cancelled: &Arc<AtomicBool>,
    paused: &Arc<AtomicBool>,
    articles_failed: &Arc<AtomicUsize>,
    all_servers: &[ServerConfig],
    yenc_names: &Arc<Mutex<HashMap<String, String>>>,
    total_decode_us: &Arc<AtomicU64>,
    total_assemble_us: &Arc<AtomicU64>,
    total_articles_decoded: &Arc<AtomicU64>,
    bandwidth: &Arc<BandwidthLimiter>,
) {
    let mut consecutive_errors: u32 = 0;
    let mut consecutive_skips: usize = 0;

    loop {
        // Check cancellation
        if cancelled.load(Ordering::Relaxed) {
            break;
        }

        // Wait while paused
        while paused.load(Ordering::Relaxed) && !cancelled.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        if cancelled.load(Ordering::Relaxed) {
            break;
        }

        // Pull next work item
        let item = { work_queue.lock().pop_front() };
        let Some(mut item) = item else {
            debug!(worker = %worker_id, "Work queue empty, exiting");
            break;
        };

        // Skip if this worker's server was already tried for this article
        if item.tried_servers.contains(&primary_server.id) {
            let queue_len = {
                let mut q = work_queue.lock();
                q.push_back(item);
                q.len()
            };
            consecutive_skips += 1;

            // If we've skipped more items than exist in the queue,
            // every remaining item needs a different server — exit
            if consecutive_skips > queue_len {
                debug!(worker = %worker_id, "No serviceable articles remaining, exiting");
                break;
            }

            // Brief yield to avoid busy-loop
            tokio::time::sleep(Duration::from_millis(10)).await;
            continue;
        }
        consecutive_skips = 0;

        // Try to fetch on our server
        let result =
            fetch_article_with_retry(conn, &item, assembler, primary_server, worker_id).await;

        match result {
            Ok(process_result) => {
                consecutive_errors = 0;
                total_decode_us.fetch_add(process_result.decode_us, Ordering::Relaxed);
                total_assemble_us.fetch_add(process_result.assemble_us, Ordering::Relaxed);
                total_articles_decoded.fetch_add(1, Ordering::Relaxed);
                if let Some(ref yname) = process_result.yenc_filename {
                    yenc_names
                        .lock()
                        .entry(item.file_id.clone())
                        .or_insert_with(|| normalize_nfc(yname));
                }
                // Throttle via bandwidth limiter
                if let Some(n) = std::num::NonZeroU32::new(process_result.decoded_bytes as u32) {
                    let _ = bandwidth.acquire_download(n).await;
                }
                let _ = progress_tx.send(ProgressUpdate::ArticleComplete {
                    job_id: item.job_id.clone(),
                    file_id: item.file_id.clone(),
                    segment_number: item.segment_number,
                    decoded_bytes: process_result.decoded_bytes,
                    file_complete: process_result.file_complete,
                    server_id: Some(primary_server.id.clone()),
                });
            }
            Err(ArticleError::ArticleNotFound) => {
                handle_article_not_available(
                    &mut item,
                    primary_server,
                    all_servers,
                    articles_failed,
                    work_queue,
                    progress_tx,
                    "Article not found on any server",
                );
            }
            Err(ArticleError::ConnectionLost(msg)) => {
                consecutive_errors += 1;
                warn!(worker = %worker_id, server = %primary_server.name, "Connection lost: {msg}");

                // Put article back for retry
                work_queue.lock().push_front(item);

                // Try to reconnect
                if consecutive_errors > MAX_RECONNECT_ATTEMPTS {
                    warn!(worker = %worker_id, server = %primary_server.name, "Too many consecutive errors, worker exiting");
                    break;
                }

                tokio::time::sleep(RECONNECT_DELAY).await;
                *conn = NntpConnection::new(worker_id.to_string());
                if let Err(e) = connect_with_retry(conn, primary_server, worker_id).await {
                    warn!(worker = %worker_id, server = %primary_server.name, "Reconnect failed: {e}, worker exiting");
                    break;
                }
                debug!(worker = %worker_id, "Reconnected successfully");
            }
            Err(ArticleError::DecodeError(msg)) => {
                handle_article_not_available(
                    &mut item,
                    primary_server,
                    all_servers,
                    articles_failed,
                    work_queue,
                    progress_tx,
                    &format!("Decode error: {msg}"),
                );
            }
            Err(ArticleError::AssemblyError(msg)) => {
                // Assembly error is local — don't retry on other servers
                articles_failed.fetch_add(1, Ordering::Relaxed);
                error!(article = %item.message_id, "Assembly error: {msg}");
                let _ = progress_tx.send(ProgressUpdate::ArticleFailed {
                    job_id: item.job_id.clone(),
                    file_id: item.file_id.clone(),
                    segment_number: item.segment_number,
                    error: format!("Assembly error: {msg}"),
                    server_id: Some(primary_server.id.clone()),
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Connection with retry
// ---------------------------------------------------------------------------

async fn connect_with_retry(
    conn: &mut NntpConnection,
    server: &ServerConfig,
    worker_id: &str,
) -> Result<(), String> {
    for attempt in 1..=MAX_RECONNECT_ATTEMPTS {
        match conn.connect(server).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                if attempt < MAX_RECONNECT_ATTEMPTS {
                    warn!(
                        worker = %worker_id,
                        server = %server.name,
                        attempt,
                        "Connect attempt failed: {e}, retrying in {}s",
                        RECONNECT_DELAY.as_secs()
                    );
                    tokio::time::sleep(RECONNECT_DELAY).await;
                    *conn = NntpConnection::new(worker_id.to_string());
                } else {
                    return Err(format!(
                        "All {MAX_RECONNECT_ATTEMPTS} connect attempts failed: {e}"
                    ));
                }
            }
        }
    }
    Err("Connect retry loop exited unexpectedly".into())
}

// ---------------------------------------------------------------------------
// Article fetch with per-server retry
// ---------------------------------------------------------------------------

/// Fetch a single article with retry logic on the same server.
///
/// - On transient errors (timeout, connection hiccup): retry up to MAX_TRIES_PER_SERVER
/// - On ArticleNotFound (430): return immediately (caller should try next server)
/// - On connection loss: return ConnectionLost (caller should reconnect)
async fn fetch_article_with_retry(
    conn: &mut NntpConnection,
    item: &WorkItem,
    assembler: &FileAssembler,
    _server: &ServerConfig,
    worker_id: &str,
) -> Result<ProcessResult, ArticleError> {
    let mut last_error = None;

    for attempt in 1..=MAX_TRIES_PER_SERVER {
        let fetch_start = Instant::now();
        match conn.fetch_article(&item.message_id).await {
            Ok(response) => {
                let fetch_us = fetch_start.elapsed().as_micros();
                let raw_data = response.data.unwrap_or_default();
                debug!(
                    worker = %worker_id,
                    article = %item.message_id,
                    raw_bytes = raw_data.len(),
                    fetch_us,
                    "NNTP fetch complete"
                );
                return decode_and_assemble(item, &raw_data, assembler);
            }
            Err(NntpError::ArticleNotFound(_)) => {
                return Err(ArticleError::ArticleNotFound);
            }
            Err(NntpError::Connection(_) | NntpError::Io(_)) => {
                return Err(ArticleError::ConnectionLost(format!(
                    "Connection error on attempt {attempt}"
                )));
            }
            Err(NntpError::Tls(msg)) => {
                return Err(ArticleError::ConnectionLost(format!("TLS error: {msg}")));
            }
            Err(e) => {
                // Transient error — retry on same server
                last_error = Some(format!("{e}"));
                if attempt < MAX_TRIES_PER_SERVER {
                    debug!(
                        worker = %worker_id,
                        article = %item.message_id,
                        attempt,
                        "Fetch error, retrying: {e}"
                    );
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }

    // All retries on this server exhausted — report as decode error
    // so caller tries next server
    Err(ArticleError::DecodeError(
        last_error.unwrap_or_else(|| "Unknown error after retries".into()),
    ))
}

// ---------------------------------------------------------------------------
// Article processing
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct ProcessResult {
    decoded_bytes: u64,
    file_complete: bool,
    /// Real filename from yEnc =ybegin header (may differ from NZB subject name).
    yenc_filename: Option<String>,
    /// yEnc decode time in microseconds.
    decode_us: u64,
    /// File assembly time in microseconds.
    assemble_us: u64,
}

#[derive(Debug, thiserror::Error)]
enum ArticleError {
    #[error("Article not found on server")]
    ArticleNotFound,
    #[error("Connection lost: {0}")]
    ConnectionLost(String),
    #[error("Decode error: {0}")]
    DecodeError(String),
    #[error("Assembly error: {0}")]
    AssemblyError(String),
}

fn decode_and_assemble(
    item: &WorkItem,
    raw_data: &[u8],
    assembler: &FileAssembler,
) -> Result<ProcessResult, ArticleError> {
    let decode_start = Instant::now();
    let decoded = decode_yenc(raw_data).map_err(|e| {
        ArticleError::DecodeError(format!(
            "yEnc decode failed for {} seg {}: {e}",
            item.filename, item.segment_number
        ))
    })?;
    let decode_us = decode_start.elapsed().as_micros();

    let yenc_filename = decoded.filename;
    let data_begin = decoded.part_begin.unwrap_or(0);
    let decoded_len = decoded.data.len() as u64;

    let assemble_start = Instant::now();
    let file_complete = assembler
        .assemble_article(
            &item.job_id,
            &item.file_id,
            item.segment_number,
            data_begin,
            &decoded.data,
        )
        .map_err(|e| {
            ArticleError::AssemblyError(format!(
                "Assembly failed for {} seg {}: {e}",
                item.filename, item.segment_number
            ))
        })?;
    let assemble_us = assemble_start.elapsed().as_micros();

    debug!(
        file = %item.filename,
        segment = item.segment_number,
        raw_bytes = raw_data.len(),
        decoded_bytes = decoded_len,
        decode_us,
        assemble_us,
        "Article decode+assemble timing"
    );

    Ok(ProcessResult {
        decoded_bytes: decoded_len,
        file_complete,
        yenc_filename,
        decode_us: decode_us as u64,
        assemble_us: assemble_us as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_known_extension_recognizes_archives() {
        assert!(has_known_extension("movie.rar"));
        assert!(has_known_extension("movie.part01.rar"));
        assert!(has_known_extension("file.zip"));
        assert!(has_known_extension("file.7z"));
        assert!(has_known_extension("archive.001"));
    }

    #[test]
    fn has_known_extension_recognizes_video() {
        assert!(has_known_extension("episode.mkv"));
        assert!(has_known_extension("movie.mp4"));
        assert!(has_known_extension("video.avi"));
        assert!(has_known_extension("clip.ts"));
    }

    #[test]
    fn has_known_extension_recognizes_par2() {
        assert!(has_known_extension("file.par2"));
        assert!(has_known_extension("file.vol00+01.par2"));
        assert!(has_known_extension("file.vol015-031.par2"));
    }

    #[test]
    fn has_known_extension_recognizes_misc() {
        assert!(has_known_extension("info.nfo"));
        assert!(has_known_extension("sub.srt"));
        assert!(has_known_extension("cover.jpg"));
        assert!(has_known_extension("song.flac"));
    }

    #[test]
    fn has_known_extension_rejects_obfuscated_hashes() {
        // Typical obfuscated filenames — no extension
        assert!(!has_known_extension("9b6a324d7560b87091685020371ba869"));
        assert!(!has_known_extension("1fG1GP7L2263LHXH213HTNIxZsX7l0cv44BZ"));
        assert!(!has_known_extension("DfKUx3bl7L6PSo6276WSaXSZ7"));
        assert!(!has_known_extension("Q77O1ZxL237vc241z77hFoLBxl"));
    }

    #[test]
    fn has_known_extension_rejects_unknown_extensions() {
        assert!(!has_known_extension("file.xyz123"));
        assert!(!has_known_extension("noext"));
        assert!(!has_known_extension(""));
    }

    #[test]
    fn has_known_extension_case_insensitive() {
        assert!(has_known_extension("file.RAR"));
        assert!(has_known_extension("file.MKV"));
        assert!(has_known_extension("file.Par2"));
        assert!(has_known_extension("file.MP4"));
    }

    #[test]
    fn deobfuscation_direction_nzb_readable_yenc_hash() {
        // Scenario: NZB subject has real name, yEnc header has hash
        // Should NOT rename (keep the good NZB name)
        let nzb_name = "DTF.St.Louis.S01E04.part70.rar";
        let yenc_name = "9b6a324d7560b87091685020371ba869";

        let nzb_has_ext = has_known_extension(nzb_name);
        let yenc_has_ext = has_known_extension(yenc_name);

        assert!(nzb_has_ext, "NZB name should have known extension");
        assert!(!yenc_has_ext, "yEnc hash should NOT have known extension");
        // With nzb_has_ext && !yenc_has_ext → the code skips rename (continue)
    }

    #[test]
    fn deobfuscation_direction_nzb_hash_yenc_readable() {
        // Scenario: NZB subject is obfuscated, yEnc has real name
        // Should rename FROM hash TO real name
        let nzb_name = "a8f3c72d1e4b5689";
        let yenc_name = "movie.mkv";

        let nzb_has_ext = has_known_extension(nzb_name);
        let yenc_has_ext = has_known_extension(yenc_name);

        assert!(!nzb_has_ext, "NZB hash should NOT have known extension");
        assert!(yenc_has_ext, "yEnc name should have known extension");
        // With !nzb_has_ext && yenc_has_ext → rename from nzb to yenc
    }

    #[test]
    fn deobfuscation_direction_both_readable() {
        // Both have extensions — prefer yEnc (standard deobfuscation)
        let nzb_name = "file_from_subject.rar";
        let yenc_name = "actual_file.rar";

        assert!(has_known_extension(nzb_name));
        assert!(has_known_extension(yenc_name));
        // With both having extensions → rename from nzb to yenc
    }

    #[test]
    fn deobfuscation_direction_both_obfuscated() {
        // Neither has extension — skip (nothing to improve)
        let nzb_name = "abc123def456";
        let yenc_name = "789ghi012jkl";

        assert!(!has_known_extension(nzb_name));
        assert!(!has_known_extension(yenc_name));
        // With neither having extensions → skip rename
    }
}
