use std::sync::Arc;

use axum::extract::{Multipart, Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use http::StatusCode;
use serde::{Deserialize, Serialize};

use nzb_core::config::ServerConfig;
use nzb_core::models::*;
use nzb_core::nzb_parser;

use crate::error::ApiError;
use crate::log_buffer::LogEntry;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct QueueQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Deserialize, Default)]
pub struct HistoryQuery {
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct AddNzbQuery {
    pub category: Option<String>,
    pub priority: Option<i32>,
    pub name: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct LogQuery {
    pub job_id: Option<String>,
    pub after_seq: Option<u64>,
    pub level: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct PauseForQuery {
    pub duration_secs: u64,
}

#[derive(Deserialize, Serialize)]
pub struct HistoryRetentionBody {
    pub retention: Option<usize>,
}

#[derive(Deserialize, Serialize)]
pub struct MaxActiveDownloadsBody {
    pub max_active_downloads: usize,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct QueueResponse {
    pub jobs: Vec<NzbJob>,
    pub total: usize,
    pub speed_bps: u64,
    pub paused: bool,
}

#[derive(Serialize)]
pub struct HistoryResponse {
    pub entries: Vec<HistoryResponseEntry>,
    pub total: usize,
}

#[derive(Serialize)]
pub struct HistoryResponseEntry {
    pub id: String,
    pub name: String,
    pub category: String,
    pub status: JobStatus,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub added_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: chrono::DateTime<chrono::Utc>,
    pub output_dir: String,
    pub stages: Vec<StageResult>,
    pub error_message: Option<String>,
    pub server_stats: Vec<ServerArticleStats>,
    pub has_nzb_data: bool,
}

impl From<HistoryEntry> for HistoryResponseEntry {
    fn from(e: HistoryEntry) -> Self {
        let has_nzb = e.nzb_data.is_some();
        Self {
            id: e.id,
            name: e.name,
            category: e.category,
            status: e.status,
            total_bytes: e.total_bytes,
            downloaded_bytes: e.downloaded_bytes,
            added_at: e.added_at,
            completed_at: e.completed_at,
            output_dir: e.output_dir.to_string_lossy().to_string(),
            stages: e.stages,
            error_message: e.error_message,
            server_stats: e.server_stats,
            has_nzb_data: has_nzb,
        }
    }
}

#[derive(Serialize)]
pub struct AddNzbResponse {
    pub status: bool,
    pub nzo_ids: Vec<String>,
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub version: &'static str,
    pub paused: bool,
    pub speed_bps: u64,
    pub queue_size: usize,
    pub disk_space_free: u64,
    pub min_free_space_bytes: u64,
    pub pause_remaining_secs: Option<i64>,
}

#[derive(Serialize)]
pub struct SimpleResponse {
    pub status: bool,
}

#[derive(Serialize)]
pub struct LogResponse {
    pub entries: Vec<LogEntry>,
    pub latest_seq: u64,
}

// ---------------------------------------------------------------------------
// Queue handlers
// ---------------------------------------------------------------------------

/// GET /api/queue -- List all jobs in the download queue.
pub async fn h_queue_list(
    State(state): State<Arc<AppState>>,
    Query(_q): Query<QueueQuery>,
) -> Result<Json<QueueResponse>, ApiError> {
    let qm = &state.queue_manager;
    let jobs = qm.get_jobs();
    let total = jobs.len();
    let speed_bps = qm.get_speed();
    let paused = qm.is_paused();

    Ok(Json(QueueResponse {
        jobs,
        total,
        speed_bps,
        paused,
    }))
}

/// POST /api/queue/add -- Add an NZB file to the queue.
pub async fn h_queue_add(
    State(state): State<Arc<AppState>>,
    Query(q): Query<AddNzbQuery>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    let mut nzo_ids = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::from(anyhow::anyhow!("Multipart error: {e}")))?
    {
        let file_name = field
            .file_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown.nzb".into());

        let data = field
            .bytes()
            .await
            .map_err(|e| ApiError::from(anyhow::anyhow!("Read error: {e}")))?;

        let name = q.name.clone().unwrap_or_else(|| {
            file_name
                .strip_suffix(".nzb")
                .unwrap_or(&file_name)
                .to_string()
        });

        // Store the raw NZB data for later retry
        let nzb_data = data.to_vec();
        let mut job = nzb_parser::parse_nzb(&name, &data).map_err(ApiError::from)?;

        // Apply category
        if let Some(ref cat) = q.category {
            job.category = cat.clone();
        }

        // Apply priority
        if let Some(prio) = q.priority {
            job.priority = match prio {
                0 => Priority::Low,
                2 => Priority::High,
                3 => Priority::Force,
                _ => Priority::Normal,
            };
        }

        // Set working directories
        let qm = &state.queue_manager;
        job.work_dir = qm.incomplete_dir().join(&job.id);
        job.output_dir = qm.complete_dir().join(&job.category);

        // Create work directory
        std::fs::create_dir_all(&job.work_dir)
            .map_err(|e| ApiError::from(anyhow::anyhow!("Failed to create work dir: {e}")))?;

        let id = job.id.clone();

        tracing::info!(
            name = %job.name,
            id = %job.id,
            files = job.file_count,
            articles = job.article_count,
            "NZB added to queue"
        );

        // Add to the queue manager (persists to DB and starts downloading)
        qm.add_job(job, Some(nzb_data)).map_err(ApiError::from)?;
        nzo_ids.push(id);
    }

    Ok((
        StatusCode::OK,
        Json(AddNzbResponse {
            status: true,
            nzo_ids,
        }),
    ))
}

/// POST /api/queue/{id}/pause -- Pause a job.
pub async fn h_queue_pause(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SimpleResponse>, ApiError> {
    state.queue_manager.pause_job(&id).map_err(ApiError::from)?;
    Ok(Json(SimpleResponse { status: true }))
}

/// POST /api/queue/{id}/resume -- Resume a paused job.
pub async fn h_queue_resume(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SimpleResponse>, ApiError> {
    state
        .queue_manager
        .resume_job(&id)
        .map_err(ApiError::from)?;
    Ok(Json(SimpleResponse { status: true }))
}

/// DELETE /api/queue/{id} -- Remove a job from the queue.
pub async fn h_queue_delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SimpleResponse>, ApiError> {
    state
        .queue_manager
        .remove_job(&id)
        .map_err(ApiError::from)?;
    Ok(Json(SimpleResponse { status: true }))
}

/// POST /api/queue/pause -- Pause all downloads.
pub async fn h_queue_pause_all(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SimpleResponse>, ApiError> {
    state.queue_manager.pause_all();
    Ok(Json(SimpleResponse { status: true }))
}

/// POST /api/queue/resume -- Resume all downloads.
pub async fn h_queue_resume_all(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SimpleResponse>, ApiError> {
    state.queue_manager.resume_all();
    Ok(Json(SimpleResponse { status: true }))
}

/// POST /api/queue/pause-for -- Pause all downloads for a duration.
pub async fn h_queue_pause_for(
    State(state): State<Arc<AppState>>,
    Query(q): Query<PauseForQuery>,
) -> Result<Json<SimpleResponse>, ApiError> {
    state.queue_manager.pause_for(q.duration_secs);
    Ok(Json(SimpleResponse { status: true }))
}

// ---------------------------------------------------------------------------
// History handlers
// ---------------------------------------------------------------------------

/// GET /api/history -- List completed/failed jobs.
pub async fn h_history_list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<HistoryResponse>, ApiError> {
    let limit = q.limit.unwrap_or(50);
    let entries = state
        .queue_manager
        .history_list(limit)
        .map_err(ApiError::from)?;
    let total = entries.len();
    let entries: Vec<HistoryResponseEntry> = entries.into_iter().map(Into::into).collect();
    Ok(Json(HistoryResponse { entries, total }))
}

/// DELETE /api/history/{id} -- Remove a history entry.
pub async fn h_history_delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SimpleResponse>, ApiError> {
    state
        .queue_manager
        .history_remove(&id)
        .map_err(ApiError::from)?;
    Ok(Json(SimpleResponse { status: true }))
}

/// DELETE /api/history -- Clear all history.
pub async fn h_history_clear(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SimpleResponse>, ApiError> {
    state
        .queue_manager
        .history_clear()
        .map_err(ApiError::from)?;
    Ok(Json(SimpleResponse { status: true }))
}

/// POST /api/history/{id}/retry -- Re-add a failed/completed NZB from history.
pub async fn h_history_retry(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    // Get the history entry to get the name/category
    let entry = state
        .queue_manager
        .history_get(&id)
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::from(anyhow::anyhow!("History entry not found")))?;

    // Get the raw NZB data
    let nzb_data = state
        .queue_manager
        .history_get_nzb_data(&id)
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::from(anyhow::anyhow!("No NZB data stored for this entry")))?;

    // Re-parse the NZB
    let mut job = nzb_parser::parse_nzb(&entry.name, &nzb_data).map_err(ApiError::from)?;
    job.category = entry.category.clone();

    // Set working directories
    let qm = &state.queue_manager;
    job.work_dir = qm.incomplete_dir().join(&job.id);
    job.output_dir = qm.complete_dir().join(&job.category);

    std::fs::create_dir_all(&job.work_dir)
        .map_err(|e| ApiError::from(anyhow::anyhow!("Failed to create work dir: {e}")))?;

    let new_id = job.id.clone();

    tracing::info!(
        name = %job.name,
        id = %new_id,
        original_id = %id,
        "Retrying NZB from history"
    );

    qm.add_job(job, Some(nzb_data)).map_err(ApiError::from)?;

    Ok((
        StatusCode::OK,
        Json(AddNzbResponse {
            status: true,
            nzo_ids: vec![new_id],
        }),
    ))
}

// ---------------------------------------------------------------------------
// Status handler
// ---------------------------------------------------------------------------

/// GET /api/status -- Overall application status.
pub async fn h_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<StatusResponse>, ApiError> {
    let qm = &state.queue_manager;
    let config = state.config();
    Ok(Json(StatusResponse {
        version: env!("CARGO_PKG_VERSION"),
        paused: qm.is_paused(),
        speed_bps: qm.get_speed(),
        queue_size: qm.queue_size(),
        disk_space_free: get_disk_space_free(&config.general.complete_dir),
        min_free_space_bytes: qm.min_free_space(),
        pause_remaining_secs: qm.pause_remaining_secs(),
    }))
}

// ---------------------------------------------------------------------------
// Log handler
// ---------------------------------------------------------------------------

/// GET /api/logs -- Get log entries.
pub async fn h_logs(
    State(state): State<Arc<AppState>>,
    Query(q): Query<LogQuery>,
) -> Result<Json<LogResponse>, ApiError> {
    let limit = q.limit.unwrap_or(200);
    let entries = state.log_buffer.get_entries(
        q.job_id.as_deref(),
        q.after_seq,
        q.level.as_deref(),
        limit,
    );
    let latest_seq = state.log_buffer.latest_seq();
    Ok(Json(LogResponse {
        entries,
        latest_seq,
    }))
}

// ---------------------------------------------------------------------------
// Config handlers
// ---------------------------------------------------------------------------

/// GET /api/config -- Get current configuration.
pub async fn h_config_get(
    State(state): State<Arc<AppState>>,
) -> Result<Json<nzb_core::config::AppConfig>, ApiError> {
    Ok(Json((*state.config()).clone()))
}

/// GET /api/config/servers -- List configured servers.
pub async fn h_servers_list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ServerConfig>>, ApiError> {
    Ok(Json(state.config().servers.clone()))
}

/// POST /api/config/servers -- Add a new server.
pub async fn h_server_add(
    State(state): State<Arc<AppState>>,
    Json(mut server): Json<ServerConfig>,
) -> Result<impl IntoResponse, ApiError> {
    // Generate ID if empty
    if server.id.is_empty() {
        server.id = uuid::Uuid::new_v4().to_string();
    }

    let mut config = (*state.config()).clone();
    config.servers.push(server);
    state.update_config(config.clone()).map_err(ApiError::from)?;
    state.queue_manager.update_servers(config.servers);

    Ok((StatusCode::OK, Json(SimpleResponse { status: true })))
}

/// PUT /api/config/servers/{id} -- Update an existing server.
pub async fn h_server_update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(server): Json<ServerConfig>,
) -> Result<Json<SimpleResponse>, ApiError> {
    let mut config = (*state.config()).clone();

    let idx = config
        .servers
        .iter()
        .position(|s| s.id == id)
        .ok_or_else(|| ApiError::from(anyhow::anyhow!("Server not found: {id}")))?;

    config.servers[idx] = server;
    state.update_config(config.clone()).map_err(ApiError::from)?;
    state.queue_manager.update_servers(config.servers);

    Ok(Json(SimpleResponse { status: true }))
}

/// DELETE /api/config/servers/{id} -- Delete a server.
pub async fn h_server_delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SimpleResponse>, ApiError> {
    let mut config = (*state.config()).clone();
    let before = config.servers.len();
    config.servers.retain(|s| s.id != id);

    if config.servers.len() == before {
        return Err(ApiError::from(anyhow::anyhow!("Server not found: {id}")));
    }

    state.update_config(config.clone()).map_err(ApiError::from)?;
    state.queue_manager.update_servers(config.servers);

    Ok(Json(SimpleResponse { status: true }))
}

/// POST /api/config/servers/{id}/test -- Test a server connection.
pub async fn h_server_test(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ServerTestResponse>, ApiError> {
    let config = state.config();
    let server = config
        .servers
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| ApiError::from(anyhow::anyhow!("Server not found: {id}")))?
        .clone();

    // Test connection in a spawned task with timeout
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        test_server_connection(server),
    )
    .await;

    match result {
        Ok(Ok(msg)) => Ok(Json(ServerTestResponse {
            success: true,
            message: msg,
        })),
        Ok(Err(msg)) => Ok(Json(ServerTestResponse {
            success: false,
            message: msg,
        })),
        Err(_) => Ok(Json(ServerTestResponse {
            success: false,
            message: "Connection timed out after 15 seconds".into(),
        })),
    }
}

#[derive(Serialize)]
pub struct ServerTestResponse {
    pub success: bool,
    pub message: String,
}

/// POST /api/config/servers/test-config -- Test a server config without saving.
pub async fn h_server_test_inline(
    Json(server): Json<ServerConfig>,
) -> Result<Json<ServerTestResponse>, ApiError> {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        test_server_connection(server),
    )
    .await;

    match result {
        Ok(Ok(msg)) => Ok(Json(ServerTestResponse {
            success: true,
            message: msg,
        })),
        Ok(Err(msg)) => Ok(Json(ServerTestResponse {
            success: false,
            message: msg,
        })),
        Err(_) => Ok(Json(ServerTestResponse {
            success: false,
            message: "Connection timed out after 15 seconds".into(),
        })),
    }
}

async fn test_server_connection(server: ServerConfig) -> Result<String, String> {
    use nzb_nntp::connection::NntpConnection;

    let mut conn = NntpConnection::new(format!("test-{}", server.id));
    conn.connect(&server)
        .await
        .map_err(|e| format!("Connection failed: {e}"))?;
    let _ = conn.quit().await;
    Ok(format!(
        "Successfully connected to {}:{}",
        server.host, server.port
    ))
}

/// GET /api/history/{id}/logs -- Get persisted logs for a history entry.
pub async fn h_history_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<LogResponse>, ApiError> {
    let logs_json = state
        .queue_manager
        .history_get_logs(&id)
        .map_err(ApiError::from)?;

    let entries: Vec<LogEntry> = match logs_json {
        Some(json) if !json.is_empty() && json != "[]" => {
            serde_json::from_str(&json).unwrap_or_default()
        }
        _ => Vec::new(),
    };

    let latest_seq = entries.last().map(|e| e.seq).unwrap_or(0);
    Ok(Json(LogResponse {
        entries,
        latest_seq,
    }))
}

/// GET /api/config/categories -- List configured categories.
pub async fn h_categories_list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<nzb_core::config::CategoryConfig>>, ApiError> {
    Ok(Json(state.config().categories.clone()))
}

/// PUT /api/config/history-retention -- Update history retention setting.
pub async fn h_history_retention_set(
    State(state): State<Arc<AppState>>,
    Json(body): Json<HistoryRetentionBody>,
) -> Result<Json<SimpleResponse>, ApiError> {
    let mut config = (*state.config()).clone();
    config.general.history_retention = body.retention;
    state.update_config(config).map_err(ApiError::from)?;
    state.queue_manager.set_history_retention(body.retention);
    Ok(Json(SimpleResponse { status: true }))
}

/// GET /api/config/history-retention -- Get history retention setting.
pub async fn h_history_retention_get(
    State(state): State<Arc<AppState>>,
) -> Result<Json<HistoryRetentionBody>, ApiError> {
    let config = state.config();
    Ok(Json(HistoryRetentionBody {
        retention: config.general.history_retention,
    }))
}

/// PUT /api/config/max-active-downloads -- Update max concurrent downloads.
pub async fn h_max_active_downloads_set(
    State(state): State<Arc<AppState>>,
    Json(body): Json<MaxActiveDownloadsBody>,
) -> Result<Json<SimpleResponse>, ApiError> {
    let mut config = (*state.config()).clone();
    config.general.max_active_downloads = body.max_active_downloads;
    state.update_config(config).map_err(ApiError::from)?;
    state
        .queue_manager
        .set_max_active_downloads(body.max_active_downloads);
    Ok(Json(SimpleResponse { status: true }))
}

/// GET /api/config/max-active-downloads -- Get max concurrent downloads.
pub async fn h_max_active_downloads_get(
    State(state): State<Arc<AppState>>,
) -> Result<Json<MaxActiveDownloadsBody>, ApiError> {
    let config = state.config();
    Ok(Json(MaxActiveDownloadsBody {
        max_active_downloads: config.general.max_active_downloads,
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get free disk space for a path (returns 0 on error).
fn get_disk_space_free(path: &std::path::Path) -> u64 {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::mem::MaybeUninit;
        let c_path = match CString::new(path.to_string_lossy().as_bytes()) {
            Ok(p) => p,
            Err(_) => return 0,
        };
        unsafe {
            let mut stat = MaybeUninit::<libc::statvfs>::uninit();
            if libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) == 0 {
                let stat = stat.assume_init();
                return stat.f_bavail as u64 * stat.f_frsize as u64;
            }
        }
        0
    }
    #[cfg(not(unix))]
    {
        0
    }
}
