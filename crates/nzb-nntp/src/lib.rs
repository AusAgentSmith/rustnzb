//! NNTP client with async I/O, TLS, pipelining, and multi-server support.
//!
//! Modules:
//! - `error` — NNTP-specific error types
//! - `connection` — Single NNTP connection state machine (TCP/TLS, auth, article fetch)
//! - `pipeline` — Request pipelining (send N ARTICLE commands before reading)
//! - `pool` — Per-server async connection pool
//! - `server` — Server health tracking, penalties, speed measurement
//! - `downloader` — Download orchestrator (assigns articles to servers with failover)

pub mod connection;
pub mod downloader;
pub mod error;
pub mod pipeline;
pub mod pool;
pub mod server;

#[cfg(test)]
pub(crate) mod testutil;

pub use connection::{ConnectionState, GroupResponse, NntpConnection, NntpResponse, XoverEntry};
pub use downloader::{ArticleResult, Downloader};
pub use error::{NntpError, NntpResult};
pub use pipeline::Pipeline;
pub use pool::ConnectionPool;
pub use server::ServerState;
