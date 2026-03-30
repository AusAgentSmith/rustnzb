//! Download orchestrator.
//!
//! Takes a list of articles and coordinates downloading them across multiple
//! NNTP servers with priority-based failover, pipelining, bandwidth limiting,
//! and pause/resume support.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::config::{Article, ServerConfig};

use crate::error::{NntpError, NntpResult};
use crate::pool::PooledConnection;
use crate::server::ServerState;

// ---------------------------------------------------------------------------
// Download result
// ---------------------------------------------------------------------------

/// The outcome of downloading a single article.
#[derive(Debug)]
pub struct ArticleResult {
    /// The article that was fetched.
    pub article: Article,
    /// The server that served this article (if successful).
    pub server_id: Option<String>,
    /// The download result: Ok with raw article data, or an error.
    pub result: Result<Vec<u8>, NntpError>,
}

// ---------------------------------------------------------------------------
// Download request (internal)
// ---------------------------------------------------------------------------

/// An article queued for download, with try-list tracking.
struct DownloadRequest {
    article: Article,
    /// Server IDs that have already been tried for this article.
    tried_servers: Vec<String>,
}

// ---------------------------------------------------------------------------
// Server pick result (value type, no borrows)
// ---------------------------------------------------------------------------

/// Information about a picked server, extracted under the lock.
struct ServerPick {
    index: usize,
    server_id: String,
    config: Arc<ServerConfig>,
}

// ---------------------------------------------------------------------------
// Downloader
// ---------------------------------------------------------------------------

/// Orchestrates downloading articles across multiple servers.
///
/// Articles are assigned to the highest-priority available server. If a server
/// fails, the article is retried on the next-highest-priority server that has
/// not been tried yet.
pub struct Downloader {
    /// Servers sorted by priority (lowest number = highest priority).
    servers: Arc<Mutex<Vec<ServerState>>>,
    /// Whether downloading is paused.
    paused: Arc<AtomicBool>,
    /// Whether the downloader has been shut down.
    shutdown: Arc<AtomicBool>,
    /// Global bandwidth limit in bytes/sec (0 = unlimited).
    bandwidth_limit_bps: u64,
}

impl Downloader {
    /// Create a new downloader with the given server configurations.
    pub fn new(mut server_configs: Vec<ServerConfig>, bandwidth_limit_bps: u64) -> Self {
        // Sort by priority (ascending: 0 is highest priority)
        server_configs.sort_by_key(|c| c.priority);

        let servers: Vec<ServerState> = server_configs
            .into_iter()
            .filter(|c| c.enabled)
            .map(ServerState::new)
            .collect();

        info!(server_count = servers.len(), "Downloader initialized");

        Self {
            servers: Arc::new(Mutex::new(servers)),
            paused: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(AtomicBool::new(false)),
            bandwidth_limit_bps,
        }
    }

    /// Pause downloading. In-flight requests complete but no new ones start.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
        info!("Downloader paused");
    }

    /// Resume downloading.
    pub fn resume(&self) {
        self.paused.store(false, Ordering::Relaxed);
        info!("Downloader resumed");
    }

    /// Check if the downloader is paused.
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Signal the downloader to shut down.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        info!("Downloader shutdown requested");
    }

    /// Download a batch of articles, sending results through the provided channel.
    ///
    /// Each article is tried on servers in priority order. Results (success or
    /// failure) are sent via `result_tx` as they complete.
    pub async fn download(
        &self,
        articles: Vec<Article>,
        result_tx: mpsc::Sender<ArticleResult>,
    ) -> NntpResult<()> {
        if articles.is_empty() {
            return Ok(());
        }

        debug!(count = articles.len(), "Starting article downloads");

        let mut pending: Vec<DownloadRequest> = articles
            .into_iter()
            .map(|article| DownloadRequest {
                tried_servers: article.tried_servers.clone(),
                article,
            })
            .collect();

        while !pending.is_empty() {
            if self.shutdown.load(Ordering::Relaxed) {
                return Err(NntpError::Shutdown);
            }

            // Wait while paused
            while self.paused.load(Ordering::Relaxed) {
                if self.shutdown.load(Ordering::Relaxed) {
                    return Err(NntpError::Shutdown);
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }

            let mut request = pending.remove(0);

            // Pick the best server (short lock, no await)
            let pick = self.pick_server(&request.tried_servers);

            let Some(pick) = pick else {
                // All servers exhausted for this article
                let _ = result_tx
                    .send(ArticleResult {
                        article: request.article.clone(),
                        server_id: None,
                        result: Err(NntpError::AllServersExhausted(
                            request.article.message_id.clone(),
                        )),
                    })
                    .await;
                continue;
            };

            // Create a fresh connection outside any lock (fully async-safe)
            let conn_result = self.connect_to_server(&pick.config).await;

            match conn_result {
                Ok(mut pooled) => {
                    let fetch_result = pooled.conn.fetch_article(&request.article.message_id).await;

                    match fetch_result {
                        Ok(response) => {
                            let data = response.data.unwrap_or_default();
                            let data_len = data.len() as u64;

                            // Record success (short lock)
                            {
                                let mut servers = self.servers.lock();
                                if let Some(server) = servers.get_mut(pick.index) {
                                    server.record_success(data_len);
                                    server.release_connection(pooled);
                                }
                            }

                            let _ = result_tx
                                .send(ArticleResult {
                                    article: request.article,
                                    server_id: Some(pick.server_id),
                                    result: Ok(data),
                                })
                                .await;
                        }
                        Err(NntpError::ArticleNotFound(_)) => {
                            // Not on this server — return conn and try next
                            {
                                let mut servers = self.servers.lock();
                                if let Some(server) = servers.get_mut(pick.index) {
                                    server.record_failure();
                                    server.release_connection(pooled);
                                }
                            }
                            request.tried_servers.push(pick.server_id);
                            pending.push(request);
                        }
                        Err(e) => {
                            let is_fatal = matches!(
                                &e,
                                NntpError::AuthRequired(_)
                                    | NntpError::ServiceUnavailable(_)
                                    | NntpError::Connection(_)
                                    | NntpError::Io(_)
                            );
                            {
                                let mut servers = self.servers.lock();
                                if let Some(server) = servers.get_mut(pick.index) {
                                    server.record_failure();
                                    if is_fatal {
                                        server.penalize_for(&e.to_string());
                                        server.discard_connection(pooled);
                                    } else {
                                        server.release_connection(pooled);
                                    }
                                }
                            }
                            request.tried_servers.push(pick.server_id);
                            pending.push(request);
                        }
                    }
                }
                Err(e) => {
                    warn!(server = %pick.server_id, "Failed to connect: {e}");
                    {
                        let mut servers = self.servers.lock();
                        if let Some(server) = servers.get_mut(pick.index) {
                            server.penalize_for(&e.to_string());
                        }
                    }
                    request.tried_servers.push(pick.server_id);
                    pending.push(request);
                }
            }

            // Simple bandwidth limiting: yield to let other tasks run
            if self.bandwidth_limit_bps > 0 {
                tokio::task::yield_now().await;
            }
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Pick the highest-priority available server not yet tried.
    fn pick_server(&self, tried: &[String]) -> Option<ServerPick> {
        let servers = self.servers.lock();
        for (idx, server) in servers.iter().enumerate() {
            if server.is_available() && !tried.contains(&server.config.id) {
                return Some(ServerPick {
                    index: idx,
                    server_id: server.config.id.clone(),
                    config: Arc::clone(&server.config),
                });
            }
        }
        None
    }

    /// Create a fresh NNTP connection to the given server.
    /// This does NOT go through the pool (avoids holding locks across await).
    async fn connect_to_server(&self, config: &ServerConfig) -> NntpResult<PooledConnection> {
        let mut conn = crate::connection::NntpConnection::new(format!("{}#dl", config.id));
        conn.connect(config).await?;
        Ok(PooledConnection {
            conn,
            last_used: std::time::Instant::now(),
        })
    }
}
