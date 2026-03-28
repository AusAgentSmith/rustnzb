//! Error types for the NNTP subsystem.

use thiserror::Error;

/// NNTP-specific errors.
#[derive(Error, Debug)]
pub enum NntpError {
    /// TCP or connection-level failure.
    #[error("Connection error: {0}")]
    Connection(String),

    /// TLS handshake or configuration failure.
    #[error("TLS error: {0}")]
    Tls(String),

    /// Authentication failure (481, 482).
    #[error("Authentication failed: {0}")]
    Auth(String),

    /// Server requires authentication (480).
    #[error("Authentication required: {0}")]
    AuthRequired(String),

    /// Service permanently unavailable (502).
    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),

    /// Article not found (430).
    #[error("Article not found: {0}")]
    ArticleNotFound(String),

    /// No such newsgroup (411).
    #[error("No such group: {0}")]
    NoSuchGroup(String),

    /// No article selected / no article in group (412, 420).
    #[error("No article selected: {0}")]
    NoArticleSelected(String),

    /// NNTP protocol violation or unexpected response.
    #[error("Protocol error: {0}")]
    Protocol(String),

    /// Underlying I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The connection pool has no available connections.
    #[error("No connections available for server {0}")]
    NoConnectionsAvailable(String),

    /// A timeout expired.
    #[error("Timeout: {0}")]
    Timeout(String),

    /// All servers have been tried for this article.
    #[error("All servers exhausted for article {0}")]
    AllServersExhausted(String),

    /// The downloader has been shut down.
    #[error("Downloader shut down")]
    Shutdown,
}

pub type NntpResult<T> = std::result::Result<T, NntpError>;
