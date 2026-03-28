use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;
use tracing::info;

use nzb_core::config::AppConfig;
use nzb_core::db::Database;

use crate::auth::{CredentialStore, TokenStore};
use crate::log_buffer::LogBuffer;
use crate::queue_manager::QueueManager;
use crate::state::AppState;

/// Configuration for engine initialization.
///
/// All fields except `config_path` are optional overrides —
/// when `None`, values from the TOML config file are used.
pub struct StartupConfig {
    /// Path to the TOML config file.
    pub config_path: PathBuf,
    /// Override listen address (e.g. "0.0.0.0").
    pub listen_addr: Option<String>,
    /// Override listen port.
    pub port: Option<u16>,
    /// Override data directory.
    pub data_dir: Option<PathBuf>,
    /// Log level filter string (e.g. "info", "debug").
    pub log_level: Option<String>,
}

/// Result of engine initialization — everything needed to run the server.
pub struct StartupResult {
    pub state: Arc<AppState>,
    pub queue_manager: Arc<QueueManager>,
    pub log_buffer: LogBuffer,
}

/// Initialize the rustnzb engine: load config, open database,
/// create QueueManager, spawn background services, build AppState.
///
/// Does **not** start the HTTP server or initialize logging/tracing —
/// callers are responsible for those.
///
/// Pass an existing `LogBuffer` if one was already created (e.g. for a
/// tracing layer that must be installed before this function runs).
/// If `None`, a new one is created.
pub async fn initialize(
    startup: StartupConfig,
    log_buffer: Option<LogBuffer>,
) -> anyhow::Result<StartupResult> {
    let config_path = startup.config_path;
    let mut config = AppConfig::load(&config_path)?;

    // Apply overrides
    if let Some(addr) = startup.listen_addr {
        config.general.listen_addr = addr;
    }
    if let Some(port) = startup.port {
        config.general.port = port;
    }
    if let Some(data_dir) = startup.data_dir {
        config.general.data_dir = data_dir;
    }

    // Apply env var overrides for OpenTelemetry
    if let Ok(val) = std::env::var("OTEL_ENABLED") {
        config.otel.enabled = val == "true" || val == "1";
    }
    if let Ok(val) = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
        config.otel.endpoint = val;
    }
    if let Ok(val) = std::env::var("OTEL_SERVICE_NAME") {
        config.otel.service_name = val;
    }

    // Ensure directories exist
    std::fs::create_dir_all(&config.general.data_dir)?;
    std::fs::create_dir_all(&config.general.incomplete_dir)?;
    std::fs::create_dir_all(&config.general.complete_dir)?;

    // Open database
    let db_path = config.general.data_dir.join("rustnzb.db");
    let db = Database::open(&db_path)?;
    info!(path = %db_path.display(), "Database opened");

    // Use provided log buffer or create a new one
    let log_buffer = log_buffer.unwrap_or_default();

    // Create the queue manager
    let queue_manager = QueueManager::new(
        config.servers.clone(),
        db,
        config.general.incomplete_dir.clone(),
        config.general.complete_dir.clone(),
        log_buffer.clone(),
        config.general.max_active_downloads,
        config.categories.clone(),
        config.general.min_free_space_bytes,
        config.general.speed_limit_bps,
    );

    // Set history retention
    if let Some(retention) = config.general.history_retention {
        queue_manager.set_history_retention(Some(retention));
    }

    // Restore any in-progress jobs from the database
    if let Err(e) = queue_manager.restore_from_db() {
        tracing::warn!("Failed to restore queue from database: {e}");
    }

    // Spawn the speed tracker background task
    queue_manager.spawn_speed_tracker();

    info!(servers = config.servers.len(), "Queue manager initialized");

    // Start directory watcher if configured
    if let Some(ref watch_dir) = config.general.watch_dir {
        let watcher =
            crate::dir_watcher::DirWatcher::new(watch_dir.clone(), Arc::clone(&queue_manager));
        tokio::spawn(async move { watcher.run().await });
        info!(dir = %watch_dir.display(), "Directory watcher started");
    }

    // Create auth stores
    let credential_store = Arc::new(CredentialStore::new(config.general.data_dir.clone()));
    let token_store = Arc::new(TokenStore::new());

    if credential_store.has_credentials() {
        info!("Authentication enabled (credentials configured)");
    } else {
        info!("Authentication not yet configured; first-boot setup required");
    }

    // Build shared config (ArcSwap) so the RSS monitor and AppState share
    // the same live config — feeds added/removed via the API are picked up
    // without a restart.
    let shared_config = Arc::new(ArcSwap::new(Arc::new(config)));

    // Always start the RSS monitor so feeds added later via the API are polled.
    let data_dir_for_rss = shared_config.load().general.data_dir.clone();
    let monitor = crate::rss_monitor::RssMonitor::new(
        Arc::clone(&shared_config),
        Arc::clone(&queue_manager),
        data_dir_for_rss,
    );
    tokio::spawn(async move { monitor.run().await });

    // Build shared application state
    let state = Arc::new(AppState::new(
        shared_config,
        config_path,
        Arc::clone(&queue_manager),
        log_buffer.clone(),
        token_store,
        credential_store,
    ));

    Ok(StartupResult {
        state,
        queue_manager,
        log_buffer,
    })
}
