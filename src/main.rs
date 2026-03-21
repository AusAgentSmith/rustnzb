use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use nzb_core::config::AppConfig;
use nzb_core::db::Database;
use nzb_web::{AppState, LogBuffer, LogBufferLayer, QueueManager};

#[derive(Parser, Debug)]
#[command(name = "rustnzbd", version, about = "Usenet NZB download client")]
struct Args {
    /// Path to config file
    #[arg(short, long, default_value = "config.toml", env = "RUSTNZBD_CONFIG")]
    config: PathBuf,

    /// Override listen address
    #[arg(long, env = "RUSTNZBD_LISTEN_ADDR")]
    listen_addr: Option<String>,

    /// Override listen port
    #[arg(short, long, env = "RUSTNZBD_PORT")]
    port: Option<u16>,

    /// Override data directory
    #[arg(long, env = "RUSTNZBD_DATA_DIR")]
    data_dir: Option<PathBuf>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info", env = "RUSTNZBD_LOG_LEVEL")]
    log_level: String,

    /// Log file path
    #[arg(long, env = "RUSTNZBD_LOG_FILE")]
    log_file: Option<PathBuf>,
}

fn init_otel_logging(
    endpoint: &str,
    service_name: &str,
) -> Option<opentelemetry_sdk::logs::SdkLoggerProvider> {
    use opentelemetry_sdk::logs::SdkLoggerProvider;
    use opentelemetry_otlp::LogExporter;
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::Resource;
    use opentelemetry::KeyValue;

    let exporter = LogExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .ok()?;

    let provider = SdkLoggerProvider::builder()
        .with_resource(Resource::builder().with_attributes([
            KeyValue::new("service.name", service_name.to_string()),
        ]).build())
        .with_batch_exporter(exporter)
        .build();

    Some(provider)
}

fn init_otel_metrics(
    endpoint: &str,
    service_name: &str,
) -> Option<opentelemetry_sdk::metrics::SdkMeterProvider> {
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use opentelemetry_otlp::MetricExporter;
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::Resource;
    use opentelemetry::KeyValue;
    use opentelemetry_sdk::metrics::PeriodicReader;

    let exporter = MetricExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .ok()?;

    let reader = PeriodicReader::builder(exporter)
        .with_interval(std::time::Duration::from_secs(15))
        .build();

    let provider = SdkMeterProvider::builder()
        .with_resource(Resource::builder().with_attributes([
            KeyValue::new("service.name", service_name.to_string()),
        ]).build())
        .with_reader(reader)
        .build();

    Some(provider)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Create the log buffer for the GUI
    let log_buffer = LogBuffer::new();

    // Load configuration early to check OTEL settings
    let config_path = args.config.clone();
    let mut config = AppConfig::load(&config_path)?;

    // Apply CLI overrides
    if let Some(addr) = args.listen_addr {
        config.general.listen_addr = addr;
    }
    if let Some(port) = args.port {
        config.general.port = port;
    }
    if let Some(data_dir) = args.data_dir {
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

    // Initialize logging
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&args.log_level));

    let fmt_layer = tracing_subscriber::fmt::layer().with_target(true);
    let log_layer = LogBufferLayer::new(log_buffer.clone());

    // Initialize OpenTelemetry if enabled
    let _otel_log_provider;
    let _otel_meter_provider;

    if config.otel.enabled {
        eprintln!(
            "OpenTelemetry enabled: endpoint={}, service={}",
            config.otel.endpoint, config.otel.service_name
        );

        _otel_log_provider = init_otel_logging(&config.otel.endpoint, &config.otel.service_name);
        _otel_meter_provider =
            init_otel_metrics(&config.otel.endpoint, &config.otel.service_name);

        if let Some(ref mp) = _otel_meter_provider {
            opentelemetry::global::set_meter_provider(mp.clone());
        }

        // Build subscriber with OTEL log bridge
        if let Some(ref lp) = _otel_log_provider {
            let otel_log_layer =
                opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(lp);
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt_layer)
                .with(log_layer)
                .with(otel_log_layer)
                .init();
        } else {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt_layer)
                .with(log_layer)
                .init();
        }
    } else {
        _otel_log_provider = None;
        _otel_meter_provider = None;

        tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .with(log_layer)
            .init();
    }

    info!("rustnzbd v{}", env!("CARGO_PKG_VERSION"));

    // Ensure directories exist
    std::fs::create_dir_all(&config.general.data_dir)?;
    std::fs::create_dir_all(&config.general.incomplete_dir)?;
    std::fs::create_dir_all(&config.general.complete_dir)?;

    // Open database
    let db_path = config.general.data_dir.join("rustnzbd.db");
    let db = Database::open(&db_path)?;
    info!(path = %db_path.display(), "Database opened");

    // Create the queue manager with server configs
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

    // If OTEL metrics enabled, spawn a metrics reporter
    if config.otel.enabled && _otel_meter_provider.is_some() {
        let qm = Arc::clone(&queue_manager);
        tokio::spawn(async move {
            let meter = opentelemetry::global::meter("rustnzbd");
            let speed_gauge = meter.f64_gauge("download.speed_bps").build();
            let queue_gauge = meter.u64_gauge("queue.depth").build();
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                speed_gauge.record(qm.get_speed() as f64, &[]);
                queue_gauge.record(qm.queue_size() as u64, &[]);
            }
        });
        info!("OpenTelemetry metrics reporter started");
    }

    // Start directory watcher if configured
    if let Some(ref watch_dir) = config.general.watch_dir {
        let watcher = nzb_web::dir_watcher::DirWatcher::new(
            watch_dir.clone(),
            Arc::clone(&queue_manager),
        );
        tokio::spawn(async move { watcher.run().await });
        info!(dir = %watch_dir.display(), "Directory watcher started");
    }

    info!(servers = config.servers.len(), "Queue manager initialized");

    // Build shared application state
    let state = Arc::new(AppState::new(
        config,
        config_path,
        Arc::clone(&queue_manager),
        log_buffer,
    ));

    // Start HTTP server
    info!("Starting HTTP API server");
    nzb_web::run(state).await?;

    Ok(())
}
