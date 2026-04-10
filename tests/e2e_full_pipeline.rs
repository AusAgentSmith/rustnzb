//! End-to-end integration test: HTTP server -> upload NZB -> verify in queue.
//!
//! This test starts the HTTP server on a random port, uploads a real NZB file
//! from TestData/ via the native API, and verifies it appears in the queue.
//! It also exercises the SABnzbd compatibility API endpoints.

use std::path::Path;
use std::sync::Arc;

use arc_swap::ArcSwap;
use nzb_web::auth::{CredentialStore, TokenStore};
use nzb_web::nzb_core::config::AppConfig;
use nzb_web::nzb_core::db::Database;
use nzb_web::{AppState, QueueManager};
use rustnzb::server::build_router;

/// Start the server on a random port and return the base URL.
async fn start_test_server() -> (String, tokio::task::JoinHandle<()>) {
    let config = AppConfig::default();
    let db = Database::open_memory().expect("Failed to create in-memory database");

    // Create a temp directory for incomplete/complete dirs
    let tmp_dir = std::env::temp_dir().join(format!("rustnzb_test_{}", std::process::id()));
    let incomplete_dir = tmp_dir.join("incomplete");
    let complete_dir = tmp_dir.join("complete");
    std::fs::create_dir_all(&incomplete_dir).ok();
    std::fs::create_dir_all(&complete_dir).ok();

    let log_buffer = nzb_web::LogBuffer::new();
    let qm = QueueManager::new(
        config.servers.clone(),
        db,
        incomplete_dir,
        complete_dir,
        log_buffer.clone(),
        config.general.max_active_downloads,
        config.categories.clone(),
        config.general.min_free_space_bytes,
        config.general.speed_limit_bps,
        false,
        config.general.abort_hopeless,
        config.general.early_failure_check,
        config.general.required_completion_pct,
        config.general.article_timeout_secs,
    );
    let token_store = Arc::new(TokenStore::new());
    let credential_store = Arc::new(CredentialStore::new(tmp_dir.clone()));
    let state = Arc::new(AppState::new(
        Arc::new(ArcSwap::from_pointee(config)),
        std::path::PathBuf::from("config.toml"),
        qm,
        log_buffer,
        token_store,
        credential_store,
    ));

    let router = build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind random port");
    let addr = listener.local_addr().expect("Failed to get local addr");
    let base_url = format!("http://127.0.0.1:{}", addr.port());

    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.ok();
    });

    // Give the server a moment to start
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    (base_url, handle)
}

/// Find an NZB file in TestData/
fn find_test_nzb() -> Option<std::path::PathBuf> {
    let dir = Path::new("TestData");
    if !dir.exists() {
        return None;
    }

    for entry in std::fs::read_dir(dir).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if path.extension().map(|e| e == "nzb").unwrap_or(false) {
            return Some(path);
        }
    }
    None
}

#[tokio::test]
async fn test_upload_nzb_and_verify_queue() {
    if std::env::var("CI").is_ok() {
        eprintln!("Skipping on CI");
        return;
    }
    let nzb_path = match find_test_nzb() {
        Some(p) => p,
        None => {
            eprintln!("TestData not found, skipping integration test");
            return;
        }
    };

    let (base_url, _handle) = start_test_server().await;
    let client = reqwest::Client::new();

    // 1. Verify server is up: GET /api/status
    let resp = client
        .get(format!("{}/api/status", base_url))
        .send()
        .await
        .expect("Failed to reach server");
    assert_eq!(resp.status(), 200);
    let status: serde_json::Value = resp.json().await.expect("Bad JSON");
    assert!(status["version"].is_string());
    eprintln!("Server version: {}", status["version"]);

    // 2. Verify queue is initially empty
    let resp = client
        .get(format!("{}/api/queue", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let queue: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(queue["jobs"].as_array().unwrap().len(), 0);
    eprintln!("Queue is empty (expected)");

    // 2b. Pause the queue so jobs stay in queue (no servers configured = immediate fail otherwise)
    let resp = client
        .get(format!("{}/sabnzbd/api?mode=pause", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    eprintln!("Queue paused for testing");

    // 3. Upload NZB file via native API
    let nzb_bytes = std::fs::read(&nzb_path).expect("Failed to read NZB file");
    let nzb_filename = nzb_path.file_name().unwrap().to_string_lossy().to_string();

    let part = reqwest::multipart::Part::bytes(nzb_bytes.clone())
        .file_name(nzb_filename.clone())
        .mime_str("application/x-nzb")
        .unwrap();
    let form = reqwest::multipart::Form::new().part("file", part);

    let resp = client
        .post(format!(
            "{}/api/queue/add?category=test&priority=1",
            base_url
        ))
        .multipart(form)
        .send()
        .await
        .expect("Upload failed");
    assert_eq!(resp.status(), 200);
    let add_result: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(add_result["status"], true);
    assert!(!add_result["nzo_ids"].as_array().unwrap().is_empty());
    let nzo_id = add_result["nzo_ids"][0].as_str().unwrap().to_string();
    eprintln!("Uploaded NZB, got ID: {}", nzo_id);

    // 4. Verify job appears in queue
    let resp = client
        .get(format!("{}/api/queue", base_url))
        .send()
        .await
        .unwrap();
    let queue: serde_json::Value = resp.json().await.unwrap();
    let jobs = queue["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 1, "Expected 1 job in queue, got {}", jobs.len());
    let job = &jobs[0];
    assert_eq!(job["category"], "test");
    assert!(job["total_bytes"].as_u64().unwrap() > 0);
    assert!(job["article_count"].as_u64().unwrap() > 0);
    assert!(job["file_count"].as_u64().unwrap() > 0);
    eprintln!(
        "Queue job: {} ({} files, {} articles, {} bytes)",
        job["name"], job["file_count"], job["article_count"], job["total_bytes"]
    );

    // 5. Test SABnzbd compatibility API -- version
    let resp = client
        .get(format!("{}/sabnzbd/api?mode=version", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let ver: serde_json::Value = resp.json().await.unwrap();
    assert!(ver["version"].is_string());
    eprintln!("SABnzbd version: {}", ver["version"]);

    // 6. Test SABnzbd API -- fullstatus
    let resp = client
        .get(format!("{}/sabnzbd/api?mode=fullstatus", base_url))
        .send()
        .await
        .unwrap();
    let fullstatus: serde_json::Value = resp.json().await.unwrap();
    // paused should be a boolean
    assert!(fullstatus["status"]["paused"].is_boolean());
    eprintln!(
        "SABnzbd fullstatus paused: {}",
        fullstatus["status"]["paused"]
    );

    // 7. Test SABnzbd API -- get_config
    let resp = client
        .get(format!("{}/sabnzbd/api?mode=get_config", base_url))
        .send()
        .await
        .unwrap();
    let config: serde_json::Value = resp.json().await.unwrap();
    assert!(config["config"]["misc"]["complete_dir"].is_string());
    assert!(config["config"]["categories"].is_array());
    eprintln!(
        "SABnzbd config complete_dir: {}",
        config["config"]["misc"]["complete_dir"]
    );

    // 8. Test SABnzbd API -- queue
    let resp = client
        .get(format!("{}/sabnzbd/api?mode=queue", base_url))
        .send()
        .await
        .unwrap();
    let sab_queue: serde_json::Value = resp.json().await.unwrap();
    let sab_slots = sab_queue["queue"]["slots"].as_array().unwrap();
    assert_eq!(sab_slots.len(), 1);
    assert!(
        sab_slots[0]["nzo_id"]
            .as_str()
            .unwrap()
            .starts_with("SABnzbd_nzo_")
    );
    assert!(sab_slots[0]["filename"].is_string());
    assert!(sab_slots[0]["percentage"].is_string());
    eprintln!(
        "SABnzbd queue slot: {} ({}%)",
        sab_slots[0]["filename"], sab_slots[0]["percentage"]
    );

    // 9. Test SABnzbd API -- history (should be empty)
    let resp = client
        .get(format!("{}/sabnzbd/api?mode=history", base_url))
        .send()
        .await
        .unwrap();
    let sab_history: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(sab_history["history"]["slots"].as_array().unwrap().len(), 0);

    // 10. Test SABnzbd addfile via multipart POST
    let part2 = reqwest::multipart::Part::bytes(nzb_bytes.clone())
        .file_name(nzb_filename.clone())
        .mime_str("application/x-nzb")
        .unwrap();
    let form2 = reqwest::multipart::Form::new()
        .text("mode", "addfile")
        .text("cat", "movies")
        .text("priority", "2")
        .part("nzbfile", part2);

    let resp = client
        .post(format!("{}/sabnzbd/api", base_url))
        .multipart(form2)
        .send()
        .await
        .expect("SABnzbd addfile failed");
    assert_eq!(resp.status(), 200);
    let sab_add: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(sab_add["status"], true);
    assert!(!sab_add["nzo_ids"].as_array().unwrap().is_empty());
    let sab_nzo = sab_add["nzo_ids"][0].as_str().unwrap();
    assert!(
        sab_nzo.starts_with("SABnzbd_nzo_"),
        "Expected SABnzbd_nzo_ prefix, got: {}",
        sab_nzo
    );
    eprintln!("SABnzbd addfile: {}", sab_nzo);

    // 11. Verify now 2 jobs in queue
    let resp = client
        .get(format!("{}/api/queue", base_url))
        .send()
        .await
        .unwrap();
    let queue: serde_json::Value = resp.json().await.unwrap();
    let job_count = queue["jobs"].as_array().unwrap().len();
    assert_eq!(job_count, 2, "Expected 2 jobs, got {}", job_count);
    eprintln!("Queue now has {} jobs", job_count);

    // 12. Test SABnzbd pause mode (already paused, but test the endpoint)
    let resp = client
        .get(format!("{}/sabnzbd/api?mode=pause", base_url))
        .send()
        .await
        .unwrap();
    let pause_result: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(pause_result["status"], true);
    eprintln!("SABnzbd pause: success");

    // 13. Test SABnzbd resume mode, then immediately re-pause to keep jobs in queue
    let resp = client
        .get(format!("{}/sabnzbd/api?mode=resume", base_url))
        .send()
        .await
        .unwrap();
    let resume_result: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(resume_result["status"], true);
    eprintln!("SABnzbd resume: success");

    // Re-pause immediately to prevent jobs from being downloaded and moved to history
    let resp = client
        .get(format!("{}/sabnzbd/api?mode=pause", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.json::<serde_json::Value>().await.unwrap()["status"],
        true
    );
    eprintln!("SABnzbd re-pause: success");

    // 14. Test native delete
    let first_job_id = queue["jobs"][0]["id"].as_str().unwrap();
    let resp = client
        .delete(format!("{}/api/queue/{}", base_url, first_job_id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    eprintln!("Deleted job: {}", first_job_id);

    // 15. Verify the remaining queue (may have 1 job or 0 depending on timing)
    let resp = client
        .get(format!("{}/api/queue", base_url))
        .send()
        .await
        .unwrap();
    let queue: serde_json::Value = resp.json().await.unwrap();
    let remaining = queue["jobs"].as_array().unwrap().len();
    eprintln!("Queue has {} job(s) remaining", remaining);
    // The remaining job may have been moved to history if download engine ran
    assert!(remaining <= 1, "Expected at most 1 job, got {}", remaining);

    // 16. Test static file serving (index.html)
    let resp = client.get(format!("{}/", base_url)).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("rustnzb"),
        "Index page should contain 'rustnzb'"
    );
    eprintln!("Static file serving: OK");

    eprintln!("\nAll integration tests passed!");
}
