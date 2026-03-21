//! *arr-compatible API layer for Sonarr/Radarr integration.
//!
//! Implements the download client protocol that Sonarr/Radarr use:
//! addfile, addurl, queue, history, config, fullstatus, version,
//! pause, resume, delete, retry.

use std::sync::Arc;

use axum::extract::{Multipart, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};

use nzb_core::models::*;
use nzb_core::nzb_parser;

use crate::error::ApiError;
use crate::state::AppState;

/// Arr-compatible API request -- all parameters come as query strings.
#[derive(Deserialize, Default)]
pub struct SabApiRequest {
    pub mode: Option<String>,
    pub name: Option<String>,
    pub value: Option<String>,
    pub value2: Option<String>,
    pub apikey: Option<String>,
    pub output: Option<String>,
    pub cat: Option<String>,
    pub priority: Option<String>,
    pub start: Option<usize>,
    pub limit: Option<usize>,
}

/// Validate API key. Returns Err with JSON response on failure.
fn validate_api_key(
    state: &AppState,
    provided: Option<&str>,
) -> Result<(), Json<serde_json::Value>> {
    let config = state.config();
    if let Some(ref configured_key) = config.general.api_key {
        let provided_key = provided.unwrap_or("");
        if !crate::auth::constant_time_eq(provided_key.as_bytes(), configured_key.as_bytes()) {
            return Err(Json(serde_json::json!({
                "status": false,
                "error": "API Key Incorrect"
            })));
        }
    }
    Ok(())
}

/// GET /sabnzbd/api -- Handle GET requests.
pub async fn h_sabnzbd_api_get(
    State(state): State<Arc<AppState>>,
    Query(req): Query<SabApiRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if let Err(resp) = validate_api_key(&state, req.apikey.as_deref()) {
        return Ok(resp);
    }

    let mode = req.mode.as_deref().unwrap_or("");
    let result = dispatch_mode(&state, mode, &req);
    Ok(result)
}

/// POST /sabnzbd/api -- Handle POST requests (addfile multipart, or form-encoded).
pub async fn h_sabnzbd_api_post(
    State(state): State<Arc<AppState>>,
    Query(query_req): Query<SabApiRequest>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    // Extract fields from multipart form data
    let mut mode = query_req.mode.clone().unwrap_or_default();
    let mut apikey = query_req.apikey.clone();
    let mut cat = query_req.cat.clone();
    let mut priority = query_req.priority.clone();
    let mut name = query_req.name.clone();
    let mut nzb_data: Option<(String, Vec<u8>)> = None;
    let mut nzb_url: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::from(anyhow::anyhow!("Multipart error: {e}")))?
    {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "mode" => {
                if let Ok(text) = field.text().await {
                    if !text.is_empty() {
                        mode = text;
                    }
                }
            }
            "apikey" => {
                if let Ok(text) = field.text().await {
                    apikey = Some(text);
                }
            }
            "cat" => {
                if let Ok(text) = field.text().await {
                    cat = Some(text);
                }
            }
            "priority" => {
                if let Ok(text) = field.text().await {
                    priority = Some(text);
                }
            }
            "name" => {
                if let Ok(text) = field.text().await {
                    name = Some(text);
                }
            }
            "nzbfile" => {
                let file_name = field
                    .file_name()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown.nzb".into());
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| ApiError::from(anyhow::anyhow!("Read error: {e}")))?;
                nzb_data = Some((file_name, data.to_vec()));
            }
            "value" | "url" => {
                if let Ok(text) = field.text().await {
                    nzb_url = Some(text);
                }
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    // Validate API key
    if let Err(resp) = validate_api_key(&state, apikey.as_deref()) {
        return Ok(resp);
    }

    match mode.as_str() {
        "addfile" => {
            let (file_name, data) = match nzb_data {
                Some(d) => d,
                None => {
                    return Ok(Json(serde_json::json!({
                        "status": false,
                        "error": "No NZB file provided"
                    })));
                }
            };

            let job_name = name.clone().unwrap_or_else(|| {
                file_name
                    .strip_suffix(".nzb")
                    .unwrap_or(&file_name)
                    .to_string()
            });

            match nzb_parser::parse_nzb(&job_name, &data) {
                Ok(mut job) => {
                    if let Some(ref c) = cat {
                        if !c.is_empty() {
                            job.category = c.clone();
                        }
                    }
                    if let Some(ref p) = priority {
                        job.priority = sab_priority_to_priority(p);
                    }

                    let qm = &state.queue_manager;
                    job.work_dir = qm.incomplete_dir().join(&job.id);
                    job.output_dir = qm.complete_dir().join(&job.category);

                    let nzo_id =
                        format!("SABnzbd_nzo_{}", &job.id[..12.min(job.id.len())]);

                    tracing::info!(
                        name = %job.name,
                        id = %job.id,
                        files = job.file_count,
                        "NZB added to queue via arr API"
                    );

                    let nzb_bytes = data.clone();
                    qm.add_job(job, Some(nzb_bytes)).map_err(ApiError::from)?;

                    Ok(Json(serde_json::json!({
                        "status": true,
                        "nzo_ids": [nzo_id]
                    })))
                }
                Err(e) => Ok(Json(serde_json::json!({
                    "status": false,
                    "error": format!("Failed to parse NZB: {e}")
                }))),
            }
        }

        "addurl" => {
            let url = nzb_url.or(name.clone()).unwrap_or_default();

            if url.is_empty() {
                return Ok(Json(serde_json::json!({
                    "status": false,
                    "error": "No URL provided"
                })));
            }

            tracing::info!(url = %url, "Fetching NZB from URL via arr API");

            // Fetch the NZB from the URL
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| ApiError::from(anyhow::anyhow!("HTTP client error: {e}")))?;

            let response = client
                .get(&url)
                .send()
                .await
                .map_err(|e| ApiError::from(anyhow::anyhow!("Failed to fetch URL: {e}")))?;

            if !response.status().is_success() {
                return Ok(Json(serde_json::json!({
                    "status": false,
                    "error": format!("URL returned HTTP {}", response.status())
                })));
            }

            let data = response
                .bytes()
                .await
                .map_err(|e| ApiError::from(anyhow::anyhow!("Failed to read response: {e}")))?;

            // Derive job name from URL filename if not provided
            let job_name = name.clone().unwrap_or_else(|| {
                url.rsplit('/')
                    .next()
                    .and_then(|s| s.split('?').next())
                    .unwrap_or("unknown")
                    .strip_suffix(".nzb")
                    .unwrap_or(
                        url.rsplit('/')
                            .next()
                            .and_then(|s| s.split('?').next())
                            .unwrap_or("unknown"),
                    )
                    .to_string()
            });

            match nzb_parser::parse_nzb(&job_name, &data) {
                Ok(mut job) => {
                    if let Some(ref c) = cat {
                        if !c.is_empty() {
                            job.category = c.clone();
                        }
                    }
                    if let Some(ref p) = priority {
                        job.priority = sab_priority_to_priority(p);
                    }

                    let qm = &state.queue_manager;
                    job.work_dir = qm.incomplete_dir().join(&job.id);
                    job.output_dir = qm.complete_dir().join(&job.category);

                    let nzo_id =
                        format!("SABnzbd_nzo_{}", &job.id[..12.min(job.id.len())]);

                    tracing::info!(
                        name = %job.name,
                        id = %job.id,
                        files = job.file_count,
                        "NZB added to queue via URL (arr API)"
                    );

                    let nzb_bytes = data.to_vec();
                    qm.add_job(job, Some(nzb_bytes)).map_err(ApiError::from)?;

                    Ok(Json(serde_json::json!({
                        "status": true,
                        "nzo_ids": [nzo_id]
                    })))
                }
                Err(e) => Ok(Json(serde_json::json!({
                    "status": false,
                    "error": format!("Failed to parse NZB: {e}")
                }))),
            }
        }

        _ => {
            let req = SabApiRequest {
                mode: Some(mode),
                name,
                value: None,
                value2: None,
                apikey,
                output: None,
                cat,
                priority,
                start: query_req.start,
                limit: query_req.limit,
            };
            Ok(dispatch_mode(
                &state,
                req.mode.as_deref().unwrap_or(""),
                &req,
            ))
        }
    }
}

/// Dispatch an API mode to the appropriate handler.
fn dispatch_mode(
    state: &AppState,
    mode: &str,
    req: &SabApiRequest,
) -> Json<serde_json::Value> {
    match mode {
        "version" => Json(serde_json::json!({
            "version": env!("CARGO_PKG_VERSION")
        })),

        "queue" => handle_queue(state, req),

        "history" => handle_history(state, req),

        "get_config" => handle_get_config(state),

        "fullstatus" | "server_stats" => {
            let qm = &state.queue_manager;
            Json(serde_json::json!({
                "status": {
                    "version": env!("CARGO_PKG_VERSION"),
                    "paused": qm.is_paused(),
                    "speed": format!("{}", qm.get_speed()),
                }
            }))
        }

        "pause" => handle_pause(state, req),

        "resume" => handle_resume(state, req),

        "delete" => handle_delete(state, req),

        "retry" => handle_retry(state, req),

        _ => Json(serde_json::json!({
            "status": false,
            "error": format!("Unknown mode: {mode}")
        })),
    }
}

// ---------------------------------------------------------------------------
// Mode handlers
// ---------------------------------------------------------------------------

fn handle_queue(state: &AppState, req: &SabApiRequest) -> Json<serde_json::Value> {
    let qm = &state.queue_manager;
    let jobs = qm.get_jobs();
    let paused = qm.is_paused();
    let speed_bps = qm.get_speed();

    let slots: Vec<SabQueueSlot> = jobs.iter().map(SabQueueSlot::from_job).collect();

    let total_mb: f64 = jobs.iter().map(|j| j.total_bytes as f64).sum::<f64>() / 1_048_576.0;
    let left_mb: f64 = jobs
        .iter()
        .map(|j| (j.total_bytes.saturating_sub(j.downloaded_bytes)) as f64)
        .sum::<f64>()
        / 1_048_576.0;

    Json(serde_json::json!({
        "queue": {
            "status": if paused { "Paused" } else { "Downloading" },
            "speedlimit": "",
            "speed": format_speed(speed_bps),
            "kbpersec": format!("{:.2}", speed_bps as f64 / 1024.0),
            "mbleft": format!("{left_mb:.2}"),
            "mb": format!("{total_mb:.2}"),
            "noofslots_total": jobs.len(),
            "noofslots": slots.len(),
            "paused": paused,
            "limit": req.limit.unwrap_or(0),
            "start": req.start.unwrap_or(0),
            "timeleft": "0:00:00",
            "eta": "unknown",
            "slots": slots
        }
    }))
}

fn handle_history(state: &AppState, req: &SabApiRequest) -> Json<serde_json::Value> {
    let limit = req.limit.unwrap_or(50);
    let entries = state.queue_manager.history_list(limit).unwrap_or_default();
    let slots: Vec<SabHistorySlot> = entries.iter().map(SabHistorySlot::from_entry).collect();

    Json(serde_json::json!({
        "history": {
            "noofslots": entries.len(),
            "last_history_update": chrono::Utc::now().timestamp(),
            "slots": slots
        }
    }))
}

fn handle_get_config(state: &AppState) -> Json<serde_json::Value> {
    let config = state.config();
    let categories: Vec<serde_json::Value> = config
        .categories
        .iter()
        .map(|c| {
            serde_json::json!({
                "name": c.name,
                "dir": c.output_dir,
                "pp": c.post_processing.to_string(),
                "order": 0,
                "newzbin": "",
                "priority": 0,
            })
        })
        .collect();

    Json(serde_json::json!({
        "config": {
            "misc": {
                "complete_dir": config.general.complete_dir,
            },
            "categories": categories,
        }
    }))
}

fn handle_pause(state: &AppState, req: &SabApiRequest) -> Json<serde_json::Value> {
    let qm = &state.queue_manager;

    // If `name` or `value` contains a specific nzo_id, pause just that job
    let target_id = req.name.as_deref().or(req.value.as_deref());

    if let Some(nzo_id) = target_id {
        if !nzo_id.is_empty() {
            let search_id = nzo_id
                .strip_prefix("SABnzbd_nzo_")
                .unwrap_or(nzo_id);

            // Try to find and pause the job
            let jobs = qm.get_jobs();
            for job in &jobs {
                if job.id == search_id || job.id.starts_with(search_id) {
                    let _ = qm.pause_job(&job.id);
                    tracing::info!(id = %job.id, "Job paused via arr API");
                    break;
                }
            }

            return Json(serde_json::json!({ "status": true }));
        }
    }

    // No specific ID -- pause all
    qm.pause_all();
    tracing::info!("All jobs paused via arr API");

    Json(serde_json::json!({ "status": true }))
}

fn handle_resume(state: &AppState, req: &SabApiRequest) -> Json<serde_json::Value> {
    let qm = &state.queue_manager;

    let target_id = req.name.as_deref().or(req.value.as_deref());

    if let Some(nzo_id) = target_id {
        if !nzo_id.is_empty() {
            let search_id = nzo_id
                .strip_prefix("SABnzbd_nzo_")
                .unwrap_or(nzo_id);

            let jobs = qm.get_jobs();
            for job in &jobs {
                if job.id == search_id || job.id.starts_with(search_id) {
                    let _ = qm.resume_job(&job.id);
                    tracing::info!(id = %job.id, "Job resumed via arr API");
                    break;
                }
            }

            return Json(serde_json::json!({ "status": true }));
        }
    }

    // Resume all
    qm.resume_all();
    tracing::info!("All jobs resumed via arr API");

    Json(serde_json::json!({ "status": true }))
}

fn handle_delete(state: &AppState, req: &SabApiRequest) -> Json<serde_json::Value> {
    let qm = &state.queue_manager;

    let target_id = req
        .name
        .as_deref()
        .or(req.value.as_deref())
        .unwrap_or("");

    if target_id.is_empty() {
        return Json(serde_json::json!({
            "status": false,
            "error": "No job ID provided"
        }));
    }

    let search_id = target_id
        .strip_prefix("SABnzbd_nzo_")
        .unwrap_or(target_id);

    // Try to remove from queue
    let jobs = qm.get_jobs();
    let mut found = false;
    for job in &jobs {
        if job.id == search_id || job.id.starts_with(search_id) {
            let _ = qm.remove_job(&job.id);
            tracing::info!(id = %job.id, "Job removed from queue via arr API");
            found = true;
            break;
        }
    }

    // Also try history if not found in queue
    if !found {
        let entries = qm.history_list(1000).unwrap_or_default();
        for entry in &entries {
            if entry.id == search_id || entry.id.starts_with(search_id) {
                let _ = qm.history_remove(&entry.id);
                tracing::info!(id = %entry.id, "Entry removed from history via arr API");
                found = true;
                break;
            }
        }
    }

    Json(serde_json::json!({ "status": found }))
}

fn handle_retry(_state: &AppState, _req: &SabApiRequest) -> Json<serde_json::Value> {
    // Retry is complex — requires re-parsing the NZB which we don't store.
    // For now, return a stub.
    Json(serde_json::json!({
        "status": false,
        "error": "Retry not yet implemented"
    }))
}

/// Convert arr-protocol priority string to our Priority enum.
fn sab_priority_to_priority(s: &str) -> Priority {
    match s.trim() {
        "-100" | "3" => Priority::Force,
        "2" => Priority::High,
        "1" => Priority::Normal,
        "0" => Priority::Low,
        _ => Priority::Normal,
    }
}

// ---------------------------------------------------------------------------
// Arr-compatible response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct SabQueueSlot {
    nzo_id: String,
    filename: String,
    cat: String,
    status: String,
    priority: String,
    mb: String,
    mbleft: String,
    percentage: String,
    timeleft: String,
    eta: String,
    avg_age: String,
    size: String,
    sizeleft: String,
}

impl SabQueueSlot {
    fn from_job(job: &NzbJob) -> Self {
        let mb = job.total_bytes as f64 / 1_048_576.0;
        let mbleft =
            (job.total_bytes.saturating_sub(job.downloaded_bytes)) as f64 / 1_048_576.0;
        let pct = if job.total_bytes > 0 {
            (job.downloaded_bytes as f64 / job.total_bytes as f64 * 100.0) as u32
        } else {
            0
        };

        Self {
            nzo_id: format!("SABnzbd_nzo_{}", &job.id[..12.min(job.id.len())]),
            filename: job.name.clone(),
            cat: job.category.clone(),
            status: match job.status {
                JobStatus::Downloading => "Downloading".into(),
                JobStatus::Paused => "Paused".into(),
                JobStatus::Queued => "Queued".into(),
                _ => job.status.to_string(),
            },
            priority: match job.priority {
                Priority::Force => "Force".into(),
                Priority::High => "High".into(),
                Priority::Normal => "Normal".into(),
                Priority::Low => "Low".into(),
            },
            mb: format!("{mb:.2}"),
            mbleft: format!("{mbleft:.2}"),
            percentage: format!("{pct}"),
            timeleft: "0:00:00".into(),
            eta: "unknown".into(),
            avg_age: "0d".into(),
            size: format_size_human(job.total_bytes),
            sizeleft: format_size_human(
                job.total_bytes.saturating_sub(job.downloaded_bytes),
            ),
        }
    }
}

#[derive(Serialize)]
struct SabHistorySlot {
    nzo_id: String,
    name: String,
    category: String,
    status: String,
    bytes: u64,
    storage: String,
    completed: i64,
    fail_message: String,
    download_time: u64,
    pp: String,
    nzb_name: String,
    stage_log: Vec<SabStageLog>,
}

#[derive(Serialize)]
struct SabStageLog {
    name: String,
    actions: Vec<String>,
}

impl SabHistorySlot {
    fn from_entry(entry: &HistoryEntry) -> Self {
        let stage_log: Vec<SabStageLog> = entry
            .stages
            .iter()
            .map(|s| SabStageLog {
                name: s.name.clone(),
                actions: vec![s.message.clone().unwrap_or_default()],
            })
            .collect();

        Self {
            nzo_id: format!(
                "SABnzbd_nzo_{}",
                &entry.id[..12.min(entry.id.len())]
            ),
            name: entry.name.clone(),
            category: entry.category.clone(),
            status: match entry.status {
                JobStatus::Completed => "Completed".into(),
                JobStatus::Failed => "Failed".into(),
                _ => entry.status.to_string(),
            },
            bytes: entry.downloaded_bytes,
            storage: entry.output_dir.to_string_lossy().to_string(),
            completed: entry.completed_at.timestamp(),
            fail_message: entry.error_message.clone().unwrap_or_default(),
            download_time: (entry.completed_at - entry.added_at)
                .num_seconds()
                .max(0) as u64,
            pp: "D".into(),
            nzb_name: format!("{}.nzb", entry.name),
            stage_log,
        }
    }
}

/// Format bytes to human-readable size string.
fn format_size_human(bytes: u64) -> String {
    if bytes == 0 {
        return "0 B".into();
    }
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut val = bytes as f64;
    let mut i = 0;
    while val >= 1024.0 && i < units.len() - 1 {
        val /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{val:.0} {}", units[i])
    } else {
        format!("{val:.1} {}", units[i])
    }
}

/// Format speed as a human-readable string.
fn format_speed(bps: u64) -> String {
    if bps >= 1_073_741_824 {
        format!("{:.1} GB/s", bps as f64 / 1_073_741_824.0)
    } else if bps >= 1_048_576 {
        format!("{:.1} MB/s", bps as f64 / 1_048_576.0)
    } else if bps >= 1024 {
        format!("{:.1} KB/s", bps as f64 / 1024.0)
    } else {
        format!("{bps} B/s")
    }
}
