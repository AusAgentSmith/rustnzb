//! Per-server connection pool.
//!
//! Manages a configurable number of NNTP connections for a single server.
//! Connections are created on demand up to the configured limit, returned
//! to the pool after use, and replaced when they become unhealthy.

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::Semaphore;
use tracing::{debug, trace, warn};

use crate::config::ServerConfig;

use crate::connection::{ConnectionState, NntpConnection};
use crate::error::{NntpError, NntpResult};

/// How long an idle connection can sit before we health-check it.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// How long to wait for a connection from the pool before giving up.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Pooled connection wrapper
// ---------------------------------------------------------------------------

/// A connection checked out from the pool, with metadata.
pub struct PooledConnection {
    /// The underlying NNTP connection.
    pub conn: NntpConnection,
    /// When this connection last completed an operation.
    pub last_used: Instant,
}

// ---------------------------------------------------------------------------
// Connection pool
// ---------------------------------------------------------------------------

/// Per-server connection pool.
///
/// The pool holds up to `max_connections` connections. Callers `acquire()` a
/// connection, use it, then `release()` it back. Connections that have entered
/// an error state are discarded rather than returned to the pool.
pub struct ConnectionPool {
    /// Server configuration (immutable reference data).
    config: Arc<ServerConfig>,
    /// Idle connections ready to be handed out.
    idle: Mutex<Vec<PooledConnection>>,
    /// Semaphore limiting total connections (idle + checked-out).
    semaphore: Arc<Semaphore>,
    /// Total connections ever created (for naming/debug).
    created_count: Mutex<u32>,
    /// Timestamp of the last connection creation (for ramp-up pacing).
    last_connect: tokio::sync::Mutex<Instant>,
    /// Delay between new connection opens (0 = no delay).
    ramp_up_delay: Duration,
}

impl ConnectionPool {
    /// Create a new pool for the given server. No connections are opened yet.
    pub fn new(config: Arc<ServerConfig>) -> Self {
        let max_conns = config.connections as usize;
        let ramp_up_delay = Duration::from_millis(config.ramp_up_delay_ms as u64);
        Self {
            config,
            idle: Mutex::new(Vec::with_capacity(max_conns)),
            semaphore: Arc::new(Semaphore::new(max_conns)),
            created_count: Mutex::new(0),
            last_connect: tokio::sync::Mutex::new(Instant::now() - Duration::from_secs(60)),
            ramp_up_delay,
        }
    }

    /// Acquire a connected, ready connection from the pool.
    ///
    /// If an idle connection is available it is returned immediately (after a
    /// health check). Otherwise a new connection is created, up to the server
    /// limit. If all connection slots are in use, this waits until one is
    /// released.
    pub async fn acquire(&self) -> NntpResult<PooledConnection> {
        // Wait for a connection slot
        let permit = tokio::time::timeout(ACQUIRE_TIMEOUT, self.semaphore.clone().acquire_owned())
            .await
            .map_err(|_| {
                NntpError::Timeout(format!(
                    "Timed out waiting for connection to {}",
                    self.config.name
                ))
            })?
            .map_err(|_| {
                NntpError::Connection(format!(
                    "Connection pool for {} is closed",
                    self.config.name
                ))
            })?;

        // Try to reuse an idle connection
        let maybe_idle = { self.idle.lock().pop() };

        if let Some(mut pooled) = maybe_idle {
            // Health check: if the connection is in a bad state, discard and make new
            if pooled.conn.state == ConnectionState::Ready && pooled.conn.is_connected() {
                // If idle too long, do a quick liveness check
                if pooled.last_used.elapsed() > IDLE_TIMEOUT {
                    debug!(
                        server = %self.config.name,
                        idle_secs = pooled.last_used.elapsed().as_secs(),
                        "Idle connection — health checking"
                    );
                    // STAT a bogus message-id; 430 = alive, I/O error = dead
                    match pooled.conn.stat_article("<health-check@pool>").await {
                        Ok(_) | Err(NntpError::ArticleNotFound(_)) => {
                            // Connection is alive
                            pooled.last_used = Instant::now();
                            permit.forget(); // slot is now checked out
                            return Ok(pooled);
                        }
                        Err(e) => {
                            warn!(
                                server = %self.config.name,
                                "Idle connection failed health check: {e}"
                            );
                            // Fall through to create a new one
                        }
                    }
                } else {
                    permit.forget();
                    return Ok(pooled);
                }
            }
            // Connection is broken — fall through to create new
        }

        // Create a new connection
        let conn = self.create_connection().await?;
        permit.forget(); // slot is now checked out

        Ok(PooledConnection {
            conn,
            last_used: Instant::now(),
        })
    }

    /// Return a connection to the pool after use.
    ///
    /// If the connection is still healthy it goes back to the idle list.
    /// If it is in an error state it is dropped and the slot is freed.
    pub fn release(&self, mut pooled: PooledConnection) {
        if pooled.conn.state == ConnectionState::Ready && pooled.conn.is_connected() {
            pooled.last_used = Instant::now();
            self.idle.lock().push(pooled);
        } else {
            debug!(
                server = %self.config.name,
                state = ?pooled.conn.state,
                "Discarding unhealthy connection"
            );
            // Drop the connection; free the semaphore slot
            drop(pooled);
            self.semaphore.add_permits(1);
        }
    }

    /// Discard a connection (e.g. after a fatal error) and free the slot.
    pub fn discard(&self, pooled: PooledConnection) {
        debug!(
            server = %self.config.name,
            "Discarding connection"
        );
        drop(pooled);
        self.semaphore.add_permits(1);
    }

    /// Number of idle connections currently in the pool.
    pub fn idle_count(&self) -> usize {
        self.idle.lock().len()
    }

    /// Close all idle connections. In-use connections are unaffected.
    pub async fn close_idle(&self) {
        let conns: Vec<PooledConnection> = {
            let mut idle = self.idle.lock();
            idle.drain(..).collect()
        };
        let count = conns.len();
        for mut c in conns {
            let _ = c.conn.quit().await;
            self.semaphore.add_permits(1);
        }
        if count > 0 {
            debug!(server = %self.config.name, count, "Closed idle connections");
        }
    }

    /// The server configuration for this pool.
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// Number of available semaphore permits (for testing).
    #[cfg(test)]
    pub(crate) fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    pub async fn wait_for_ramp_up(&self) {
        if self.ramp_up_delay.is_zero() {
            return;
        }
        let mut last = self.last_connect.lock().await;
        let elapsed = last.elapsed();
        if elapsed < self.ramp_up_delay {
            let wait = self.ramp_up_delay - elapsed;
            trace!(
                server = %self.config.name,
                wait_ms = wait.as_millis(),
                "Ramp-up: waiting before new connection"
            );
            tokio::time::sleep(wait).await;
        }
        *last = Instant::now();
    }

    pub fn ramp_up_delay(&self) -> Duration {
        self.ramp_up_delay
    }

    // ------------------------------------------------------------------
    // Internal
    // ------------------------------------------------------------------

    /// Create and connect a new NNTP connection.
    async fn create_connection(&self) -> NntpResult<NntpConnection> {
        self.wait_for_ramp_up().await;
        let idx = {
            let mut count = self.created_count.lock();
            *count += 1;
            *count
        };

        let conn_id = format!("{}#{}", self.config.id, idx);
        debug!(server = %self.config.name, conn_id = %conn_id, "Creating new connection");

        let mut conn = NntpConnection::new(conn_id);
        conn.connect(&self.config).await.inspect_err(|_| {
            // Free the semaphore slot since we failed
            self.semaphore.add_permits(1);
        })?;

        Ok(conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{MockConfig, MockNntpServer, test_config};

    #[tokio::test]
    async fn test_pool_new() {
        let config = Arc::new(test_config(12345));
        let pool = ConnectionPool::new(config.clone());
        assert_eq!(pool.idle_count(), 0);
        assert_eq!(pool.config().id, "test-server");
        assert_eq!(pool.available_permits(), 4); // connections = 4
    }

    #[tokio::test]
    async fn test_pool_acquire_creates_connection() {
        let server = MockNntpServer::start(MockConfig::default()).await;
        let config = Arc::new(test_config(server.port()));
        let pool = ConnectionPool::new(config);

        let pooled = pool.acquire().await.unwrap();
        assert_eq!(pooled.conn.state, ConnectionState::Ready);
        assert!(pooled.conn.is_connected());
        assert_eq!(pool.idle_count(), 0);

        // Release it back
        pool.release(pooled);
        assert_eq!(pool.idle_count(), 1);
    }

    #[tokio::test]
    async fn test_pool_release_and_reuse() {
        let server = MockNntpServer::start(MockConfig::default()).await;
        let config = Arc::new(test_config(server.port()));
        let pool = ConnectionPool::new(config);

        // Acquire and release
        let pooled = pool.acquire().await.unwrap();
        let first_id = pooled.conn.server_id.clone();
        pool.release(pooled);
        assert_eq!(pool.idle_count(), 1);

        // Acquire again — should reuse the idle connection
        let pooled = pool.acquire().await.unwrap();
        assert_eq!(pooled.conn.server_id, first_id);
        assert_eq!(pool.idle_count(), 0);

        pool.release(pooled);
    }

    #[tokio::test]
    async fn test_pool_discard_frees_slot() {
        let server = MockNntpServer::start(MockConfig::default()).await;
        let mut cfg = test_config(server.port());
        cfg.connections = 2;
        let pool = ConnectionPool::new(Arc::new(cfg));

        let c1 = pool.acquire().await.unwrap();
        let c2 = pool.acquire().await.unwrap();
        assert_eq!(pool.available_permits(), 0);

        // Discard one — frees a permit
        pool.discard(c1);
        assert_eq!(pool.available_permits(), 1);
        assert_eq!(pool.idle_count(), 0);

        pool.release(c2);
    }

    #[tokio::test]
    async fn test_pool_close_idle() {
        let server = MockNntpServer::start(MockConfig::default()).await;
        let config = Arc::new(test_config(server.port()));
        let pool = ConnectionPool::new(config);

        // Create and release two connections
        let c1 = pool.acquire().await.unwrap();
        let c2 = pool.acquire().await.unwrap();
        pool.release(c1);
        pool.release(c2);
        assert_eq!(pool.idle_count(), 2);

        // Close all idle
        pool.close_idle().await;
        assert_eq!(pool.idle_count(), 0);
    }

    #[tokio::test]
    async fn test_pool_idle_count_tracking() {
        let server = MockNntpServer::start(MockConfig::default()).await;
        let config = Arc::new(test_config(server.port()));
        let pool = ConnectionPool::new(config);

        assert_eq!(pool.idle_count(), 0);

        let c1 = pool.acquire().await.unwrap();
        assert_eq!(pool.idle_count(), 0);

        let c2 = pool.acquire().await.unwrap();
        assert_eq!(pool.idle_count(), 0);

        pool.release(c1);
        assert_eq!(pool.idle_count(), 1);

        pool.release(c2);
        assert_eq!(pool.idle_count(), 2);
    }
}
