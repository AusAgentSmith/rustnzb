//! NNTP server and article configuration types.

use serde::{Deserialize, Serialize};

/// NNTP server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Unique server identifier
    pub id: String,
    /// Display name
    pub name: String,
    /// Server hostname
    pub host: String,
    /// Server port
    pub port: u16,
    /// Use SSL/TLS
    pub ssl: bool,
    /// Verify SSL certificates
    pub ssl_verify: bool,
    /// Username for authentication
    pub username: Option<String>,
    /// Password for authentication
    pub password: Option<String>,
    /// Max simultaneous connections
    pub connections: u16,
    /// Server priority (0 = highest)
    pub priority: u8,
    /// Enable this server
    pub enabled: bool,
    /// Article retention in days (0 = unlimited)
    pub retention: u32,
    /// Number of pipelined requests per connection
    pub pipelining: u8,
    /// Server is optional (failure is non-fatal)
    pub optional: bool,
    /// Enable XFEATURE COMPRESS GZIP negotiation
    #[serde(default)]
    pub compress: bool,
    /// Delay in milliseconds between opening new connections (0 = no delay).
    /// Prevents connection bursts that trigger server-side rate limiting.
    #[serde(default)]
    pub ramp_up_delay_ms: u32,
    /// Optional SOCKS5 proxy URL: `socks5://[username:password@]host:port`
    #[serde(default)]
    pub proxy_url: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            host: String::new(),
            port: 563,
            ssl: true,
            ssl_verify: true,
            username: None,
            password: None,
            connections: 4,
            priority: 0,
            enabled: true,
            retention: 0,
            pipelining: 1,
            optional: false,
            compress: false,
            ramp_up_delay_ms: 250,
            proxy_url: None,
        }
    }
}

/// Entry from `LIST ACTIVE` response.
///
/// Each line: `groupname last first posting_flag`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListActiveEntry {
    /// Newsgroup name (e.g., "alt.binaries.test")
    pub name: String,
    /// Highest article number
    pub high: u64,
    /// Lowest article number
    pub low: u64,
    /// Posting flag (y = posting allowed, n = no posting, m = moderated)
    pub status: String,
}

/// A Usenet article segment to be downloaded.
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
