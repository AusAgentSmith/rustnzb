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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::mpsc;
use std::time::Instant;
use tracing::{debug, error, info, warn};

use nzb_core::config::ServerConfig;
use nzb_core::models::NzbJob;
use nzb_decode::yenc::decode_yenc;
use nzb_decode::FileAssembler;
use nzb_nntp::connection::NntpConnection;
use nzb_nntp::error::NntpError;
use nzb_nntp::Pipeline;

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
}

impl DownloadEngine {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
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
            if let Err(e) = assembler.register_file(
                &job.id,
                &file.id,
                output_path,
                file.articles.len() as u32,
            ) {
                error!(file = %file.filename, "Failed to register file: {e}");
            }
        }

        // Shared work queue
        let work_queue = Arc::new(Mutex::new(VecDeque::from(work_items)));
        let articles_failed = Arc::new(AtomicUsize::new(0));

        // Sort servers by priority, filter enabled
        let mut sorted_servers: Vec<ServerConfig> = servers
            .iter()
            .filter(|s| s.enabled)
            .cloned()
            .collect();
        sorted_servers.sort_by_key(|s| s.priority);

        if sorted_servers.is_empty() {
            error!("No enabled servers configured");
            let _ = progress_tx.send(ProgressUpdate::JobFinished {
                job_id,
                success: false,
                articles_failed: 0,
            });
            return;
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
        info!(
            job_id = %job_id,
            elapsed_secs = download_elapsed.as_secs_f64(),
            total_bytes,
            throughput_mbps = format!("{throughput_mbps:.2}"),
            "Download phase complete"
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
) {
    let worker_id = format!("{}#{}", primary_server.id, conn_idx);

    // Connect to primary server
    let mut conn = NntpConnection::new(worker_id.clone());
    if let Err(e) = connect_with_retry(&mut conn, &primary_server, &worker_id).await {
        warn!(worker = %worker_id, "Failed to connect after retries: {e}");
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
            warn!(worker = %worker_id, "Pipeline send error: {e}");
            // Re-queue all in-flight items
            requeue_all(&mut in_flight_items, work_queue);
            // Try reconnect
            consecutive_errors += 1;
            if consecutive_errors > MAX_RECONNECT_ATTEMPTS {
                warn!(worker = %worker_id, "Too many pipeline errors, exiting");
                break;
            }
            tokio::time::sleep(RECONNECT_DELAY).await;
            *conn = NntpConnection::new(worker_id.to_string());
            if let Err(e) = connect_with_retry(conn, primary_server, worker_id).await {
                warn!(worker = %worker_id, "Reconnect failed: {e}");
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
                        match decode_and_assemble(&item, &raw_data, assembler) {
                            Ok(process_result) => {
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
                            warn!(worker = %worker_id, "Too many errors, exiting");
                            break;
                        }
                        tokio::time::sleep(RECONNECT_DELAY).await;
                        *conn = NntpConnection::new(worker_id.to_string());
                        if let Err(e) = connect_with_retry(conn, primary_server, worker_id).await {
                            warn!(worker = %worker_id, "Reconnect failed: {e}");
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
                warn!(worker = %worker_id, "Pipeline receive error: {e}");
                requeue_all(&mut in_flight_items, work_queue);
                consecutive_errors += 1;
                if consecutive_errors > MAX_RECONNECT_ATTEMPTS {
                    break;
                }
                tokio::time::sleep(RECONNECT_DELAY).await;
                *conn = NntpConnection::new(worker_id.to_string());
                if let Err(e) = connect_with_retry(conn, primary_server, worker_id).await {
                    warn!(worker = %worker_id, "Reconnect failed: {e}");
                    break;
                }
                pipeline = Pipeline::new(pipe_depth);
            }
        }
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
        let result = fetch_article_with_retry(
            conn,
            &item,
            assembler,
            primary_server,
            worker_id,
        )
        .await;

        match result {
            Ok(process_result) => {
                consecutive_errors = 0;
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
                warn!(worker = %worker_id, "Connection lost: {msg}");

                // Put article back for retry
                work_queue.lock().push_front(item);

                // Try to reconnect
                if consecutive_errors > MAX_RECONNECT_ATTEMPTS {
                    warn!(worker = %worker_id, "Too many consecutive errors, worker exiting");
                    break;
                }

                tokio::time::sleep(RECONNECT_DELAY).await;
                *conn = NntpConnection::new(worker_id.to_string());
                if let Err(e) = connect_with_retry(conn, primary_server, worker_id).await {
                    warn!(worker = %worker_id, "Reconnect failed: {e}, worker exiting");
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
                        attempt,
                        "Connect attempt failed: {e}, retrying in {}s",
                        RECONNECT_DELAY.as_secs()
                    );
                    tokio::time::sleep(RECONNECT_DELAY).await;
                    *conn = NntpConnection::new(worker_id.to_string());
                } else {
                    return Err(format!("All {MAX_RECONNECT_ATTEMPTS} connect attempts failed: {e}"));
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
    })
}
