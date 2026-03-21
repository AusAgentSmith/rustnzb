use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub servers: Vec<ServerConfig>,
    pub categories: Vec<CategoryConfig>,
    #[serde(default)]
    pub otel: OtelConfig,
    #[serde(default)]
    pub rss_feeds: Vec<RssFeedConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            servers: Vec::new(),
            categories: vec![CategoryConfig::default()],
            otel: OtelConfig::default(),
            rss_feeds: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// HTTP API listen address
    pub listen_addr: String,
    /// HTTP API port
    pub port: u16,
    /// API key for authentication
    pub api_key: Option<String>,
    /// Directory for incomplete downloads
    pub incomplete_dir: PathBuf,
    /// Directory for completed downloads
    pub complete_dir: PathBuf,
    /// Directory for application data (DB, logs)
    pub data_dir: PathBuf,
    /// Download speed limit in bytes/sec (0 = unlimited)
    pub speed_limit_bps: u64,
    /// Article cache size in bytes
    pub cache_size: u64,
    /// Log level
    pub log_level: String,
    /// Log file path (None = stdout only)
    pub log_file: Option<PathBuf>,
    /// History retention: how many NZBs to keep in history (None = keep all)
    pub history_retention: Option<usize>,
    /// Max number of NZBs downloading simultaneously (default 1)
    pub max_active_downloads: usize,
    /// Minimum free disk space in bytes before pausing downloads (default 1 GB)
    #[serde(default = "default_min_free_space")]
    pub min_free_space_bytes: u64,
    /// Directory to watch for new .nzb files to auto-enqueue
    pub watch_dir: Option<PathBuf>,
    /// RSS feed history limit: how many feed items to keep (None = keep all, default 500)
    #[serde(default = "default_rss_history_limit")]
    pub rss_history_limit: Option<usize>,
}

fn default_rss_history_limit() -> Option<usize> {
    Some(500)
}

fn default_min_free_space() -> u64 {
    1_073_741_824 // 1 GB
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0".into(),
            port: 9090,
            api_key: None,
            incomplete_dir: PathBuf::from("downloads/incomplete"),
            complete_dir: PathBuf::from("downloads/complete"),
            data_dir: PathBuf::from("data"),
            speed_limit_bps: 0,
            cache_size: 500 * 1024 * 1024, // 500 MB
            log_level: "info".into(),
            log_file: None,
            history_retention: None, // keep all
            max_active_downloads: 1,
            min_free_space_bytes: default_min_free_space(),
            watch_dir: None,
            rss_history_limit: default_rss_history_limit(),
        }
    }
}

/// OpenTelemetry configuration. All values can be overridden via env vars.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OtelConfig {
    /// Enable OpenTelemetry export
    pub enabled: bool,
    /// OTLP endpoint for logs and metrics
    pub endpoint: String,
    /// Service name reported to the collector
    pub service_name: String,
}

impl Default for OtelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: "http://100.96.114.15:3100".into(),
            service_name: "rustnzb".into(),
        }
    }
}

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
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: String::new(),
            host: String::new(),
            port: 563,
            ssl: true,
            ssl_verify: true,
            username: None,
            password: None,
            connections: 8,
            priority: 0,
            enabled: true,
            retention: 0,
            pipelining: 1,
            optional: false,
        }
    }
}

/// Category configuration for organizing downloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryConfig {
    /// Category name
    pub name: String,
    /// Output directory override (relative to complete_dir)
    pub output_dir: Option<PathBuf>,
    /// Post-processing level: 0=none, 1=repair, 2=unpack, 3=repair+unpack
    pub post_processing: u8,
}

impl Default for CategoryConfig {
    fn default() -> Self {
        Self {
            name: "Default".into(),
            output_dir: None,
            post_processing: 3,
        }
    }
}

/// RSS feed configuration for automatic NZB downloading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RssFeedConfig {
    /// Display name for the feed
    pub name: String,
    /// Feed URL (RSS 2.0 or Atom)
    pub url: String,
    /// How often to poll, in seconds (default 900 = 15 minutes)
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    /// Category to assign to downloaded NZBs
    #[serde(default)]
    pub category: Option<String>,
    /// Regex pattern to filter feed entries by title
    #[serde(default)]
    pub filter_regex: Option<String>,
    /// Whether this feed is active
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Auto-download all items from this feed (no rules needed).
    /// Ignored when filter_regex is set (use download rules instead).
    #[serde(default)]
    pub auto_download: bool,
}

fn default_poll_interval() -> u64 {
    900
}

fn default_true() -> bool {
    true
}

impl AppConfig {
    /// Load config from a TOML file, creating default if it doesn't exist.
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        if path.exists() {
            let contents = std::fs::read_to_string(path)?;
            let config: AppConfig = toml::from_str(&contents)?;
            Ok(config)
        } else {
            let config = AppConfig::default();
            config.save(path)?;
            Ok(config)
        }
    }

    /// Save config to a TOML file.
    pub fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(self)?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Find a category by name.
    pub fn category(&self, name: &str) -> Option<&CategoryConfig> {
        self.categories.iter().find(|c| c.name == name)
    }

    /// Find a server by ID.
    pub fn server(&self, id: &str) -> Option<&ServerConfig> {
        self.servers.iter().find(|s| s.id == id)
    }
}
