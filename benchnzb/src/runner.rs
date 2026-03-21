use crate::clients::sabnzbd::SabnzbdClient;
use crate::clients::rustnzb::RustnzbClient;
use crate::config::{self, Scenario, MB, GB};
use crate::metrics::{MetricSample, MetricsCollector};
use crate::{charts, datagen, docker, report};
use anyhow::Result;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ClientResult {
    pub client: String,
    pub scenario: String,
    pub scenario_description: String,
    pub test_type: String,
    pub total_bytes: u64,
    pub download_sec: f64,
    pub par2_sec: f64,
    pub unpack_sec: f64,
    pub total_sec: f64,
    pub avg_speed_mbps: f64,
    pub peak_speed_mbps: f64,
    pub cpu_avg: f64,
    pub cpu_peak: f64,
    pub mem_avg_mb: f64,
    pub mem_peak_mb: f64,
    pub net_rx_avg_mbps: f64,
    pub net_rx_peak_mbps: f64,
    pub disk_write_avg_mbps: f64,
    pub disk_write_peak_mbps: f64,
    pub iowait_avg: f64,
    pub iowait_peak: f64,
    pub timeseries: Vec<MetricSample>,
    /// Internal metrics from the client's own API (rustnzb only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internal_metrics: Option<InternalMetrics>,
}

/// Metrics captured from rustnzb's own REST API after job completion.
#[derive(Debug, Clone, Serialize, serde::Deserialize, Default)]
pub struct InternalMetrics {
    /// Per-server download statistics.
    pub server_stats: Vec<ServerStat>,
    /// Per-stage durations reported by the client's post-processing pipeline.
    pub stage_durations: Vec<StageDuration>,
    /// Download throughput reported by the download engine (MB/s).
    pub download_throughput_mbps: f64,
    /// Total articles downloaded.
    pub articles_downloaded: u64,
    /// Total articles failed.
    pub articles_failed: u64,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ServerStat {
    pub server_name: String,
    pub articles_downloaded: u64,
    pub articles_failed: u64,
    pub bytes_downloaded: u64,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct StageDuration {
    pub name: String,
    pub status: String,
    pub duration_secs: f64,
    pub message: Option<String>,
}

async fn wait_for_service(name: &str, url: &str, timeout_secs: u64) -> Result<()> {
    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!("{name} not ready after {timeout_secs}s");
        }
        if let Ok(resp) = client
            .get(url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            if resp.status().as_u16() < 500 {
                tracing::info!("  {name}: ready");
                return Ok(());
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

async fn trigger_mock_nntp_reload() -> Result<()> {
    let client = reqwest::Client::new();
    let resp = client
        .get("http://mock-nntp:8080/reload")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await?;
    let body: serde_json::Value = resp.json().await?;
    tracing::info!("Mock NNTP reloaded: {body}");
    Ok(())
}

async fn clean_download_dir(docker_client: &bollard::Docker, service: &str, dir: &str) {
    if let Some(cid) = docker::get_container_id(docker_client, service).await {
        match docker::exec_in_container(
            docker_client,
            &cid,
            vec![
                "sh",
                "-c",
                &format!("rm -rf {dir}/* {dir}/.[!.]* 2>/dev/null; echo ok"),
            ],
        )
        .await
        {
            Ok(_) => tracing::info!("  [{service}] Cleaned {dir}"),
            Err(e) => tracing::warn!("  [{service}] Failed to clean {dir}: {e}"),
        }
    }
}

pub async fn run(
    scenario_selector: String,
    data_dir: PathBuf,
    results_dir: PathBuf,
) -> Result<()> {
    tracing::info!("============================================================");
    tracing::info!("  Usenet Client Benchmark: SABnzbd vs rustnzb");
    tracing::info!("============================================================");

    let docker_client = docker::connect()?;
    let mut metrics = MetricsCollector::new(docker::connect()?);

    // Wait for services
    tracing::info!("Waiting for services...");
    wait_for_service("mock-nntp", "http://mock-nntp:8080/health", 120).await?;
    wait_for_service("sabnzbd", &format!("{}/", config::SABNZBD_API), 180).await?;
    wait_for_service(
        "rustnzb",
        &format!("{}/api/status", config::RUSTNZB_API),
        120,
    )
    .await?;

    let sab = SabnzbdClient::new(config::SABNZBD_API);
    let rnzb = RustnzbClient::new(config::RUSTNZB_API);

    // Resolve container IDs for metrics and log capture
    metrics.resolve_container_id("sabnzbd").await;
    metrics.resolve_container_id("rustnzb").await;

    let sab_container_id = docker::get_container_id(&docker_client, "sabnzbd").await;
    let rnzb_container_id = docker::get_container_id(&docker_client, "rustnzb").await;

    // Resolve scenarios
    let scenarios = config::resolve_scenarios(&scenario_selector);
    if scenarios.is_empty() {
        return Ok(());
    }

    let total_data: u64 = scenarios.iter().map(|s| s.total_size).sum();
    tracing::info!(
        "Running {} scenario(s), {:.1} GB total raw data",
        scenarios.len(),
        total_data as f64 / GB as f64
    );
    for s in &scenarios {
        tracing::info!(
            "  {:25} {:>5} GB  {:>6}  timeout={}s",
            s.name,
            s.total_size / GB,
            s.test_type,
            s.timeout_secs,
        );
    }

    // Generate test data
    tracing::info!("Generating test data...");
    datagen::prepare_data(&scenarios, &data_dir).await?;

    // Reload mock NNTP index
    trigger_mock_nntp_reload().await?;

    // Clear any stale history before starting
    sab.clear_all().await;
    rnzb.clear_all().await;

    let mut all_results: Vec<(ClientResult, ClientResult)> = Vec::new();
    let mut scenario_logs: Vec<(String, String, String)> = Vec::new(); // (scenario_name, sab_logs, rnzb_logs)

    for sc in &scenarios {
        tracing::info!("============================================================");
        tracing::info!("SCENARIO: {} — {}", sc.name, sc.description);
        tracing::info!("============================================================");

        let nzb_path = datagen::nzb_path(sc, &data_dir);

        // Clean download directories
        clean_download_dir(&docker_client, "sabnzbd", "/downloads").await;
        clean_download_dir(&docker_client, "rustnzb", "/downloads").await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // Run SABnzbd
        let sab_start = chrono::Utc::now().to_rfc3339();
        let sab_result = run_client("sabnzbd", sc, &nzb_path, &sab, &rnzb, &metrics).await;
        let sab_logs = if let Some(ref cid) = sab_container_id {
            docker::get_container_logs(&docker_client, cid, &sab_start)
                .await
                .unwrap_or_default()
        } else {
            String::new()
        };
        sab.clear_all().await;
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        // Clean between runs
        clean_download_dir(&docker_client, "sabnzbd", "/downloads").await;
        clean_download_dir(&docker_client, "rustnzb", "/downloads").await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // Run rustnzb
        let rnzb_start = chrono::Utc::now().to_rfc3339();
        let rnzb_result = run_client("rustnzb", sc, &nzb_path, &sab, &rnzb, &metrics).await;
        let rnzb_logs = if let Some(ref cid) = rnzb_container_id {
            docker::get_container_logs(&docker_client, cid, &rnzb_start)
                .await
                .unwrap_or_default()
        } else {
            String::new()
        };
        rnzb.clear_all().await;
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        scenario_logs.push((sc.name.clone(), sab_logs, rnzb_logs));
        all_results.push((sab_result, rnzb_result));
    }

    // Reports
    tokio::fs::create_dir_all(&results_dir).await?;
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();

    report::write_json(&all_results, &results_dir, &timestamp)?;
    report::write_csv(&all_results, &results_dir, &timestamp)?;
    let summary = report::build_summary(&all_results);
    println!("\n{summary}");
    report::write_summary(&summary, &results_dir, &timestamp)?;

    let charts_dir = results_dir.join(format!("charts_{timestamp}"));
    std::fs::create_dir_all(&charts_dir)?;
    charts::generate_all(&all_results, &charts_dir)?;

    // Write per-scenario container logs for tuning analysis
    let logs_dir = results_dir.join(format!("logs_{timestamp}"));
    std::fs::create_dir_all(&logs_dir)?;
    for (scenario_name, sab_logs, rnzb_logs) in &scenario_logs {
        if !sab_logs.is_empty() {
            let path = logs_dir.join(format!("{scenario_name}_sabnzbd.log"));
            std::fs::write(&path, sab_logs)?;
        }
        if !rnzb_logs.is_empty() {
            let path = logs_dir.join(format!("{scenario_name}_rustnzb.log"));
            std::fs::write(&path, rnzb_logs)?;
        }
    }
    tracing::info!("Logs: {} ({} scenario(s))", logs_dir.display(), scenario_logs.len());

    tracing::info!("Results: {}", results_dir.display());
    Ok(())
}

async fn run_client(
    client_name: &str,
    sc: &Scenario,
    nzb_path: &Path,
    sab: &SabnzbdClient,
    rnzb: &RustnzbClient,
    metrics: &MetricsCollector,
) -> ClientResult {
    let mut result = ClientResult {
        client: client_name.to_string(),
        scenario: sc.name.clone(),
        scenario_description: sc.description.clone(),
        test_type: sc.test_type.to_string(),
        total_bytes: sc.total_size,
        download_sec: 0.0,
        par2_sec: 0.0,
        unpack_sec: 0.0,
        total_sec: 0.0,
        avg_speed_mbps: 0.0,
        peak_speed_mbps: 0.0,
        cpu_avg: 0.0,
        cpu_peak: 0.0,
        mem_avg_mb: 0.0,
        mem_peak_mb: 0.0,
        net_rx_avg_mbps: 0.0,
        net_rx_peak_mbps: 0.0,
        disk_write_avg_mbps: 0.0,
        disk_write_peak_mbps: 0.0,
        iowait_avg: 0.0,
        iowait_peak: 0.0,
        timeseries: vec![],
        internal_metrics: None,
    };

    tracing::info!("  [{client_name}] Adding NZB...");
    let nzb_data = match tokio::fs::read(nzb_path).await {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("  [{client_name}] Failed to read NZB: {e}");
            return result;
        }
    };
    let nzb_filename = nzb_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();

    let add_result = if client_name == "sabnzbd" {
        sab.add_nzb(&nzb_data, &nzb_filename).await
    } else {
        rnzb.add_nzb(&nzb_data, &nzb_filename).await
    };
    if let Err(e) = add_result {
        tracing::error!("  [{client_name}] Failed to add NZB: {e}");
        return result;
    }

    let stats_handle = metrics.start_collecting(client_name);
    let start = tokio::time::Instant::now();
    let mut peak_speed: f64 = 0.0;
    let mut speeds = Vec::new();
    let deadline = start + std::time::Duration::from_secs(sc.timeout_secs);

    tracing::info!(
        "  [{client_name}] Downloading (timeout {}s)...",
        sc.timeout_secs
    );

    loop {
        if tokio::time::Instant::now() > deadline {
            tracing::warn!("  [{client_name}] TIMEOUT");
            break;
        }

        let (finished, progress, speed) = if client_name == "sabnzbd" {
            let fin = sab.all_finished().await.unwrap_or(false);
            let prog = sab.progress_fraction().await;
            let spd = sab.download_speed().await;
            (fin, prog, spd)
        } else {
            let fin = rnzb.all_finished().await.unwrap_or(false);
            let prog = rnzb.progress_fraction().await;
            let spd = rnzb.download_speed().await;
            (fin, prog, spd)
        };

        if speed > 0.0 {
            peak_speed = peak_speed.max(speed);
            speeds.push(speed);
        }

        if finished {
            tracing::info!("  [{client_name}] Complete!");
            break;
        }

        let bar_len = 30;
        let filled = (bar_len as f64 * progress) as usize;
        let bar: String = "#".repeat(filled) + &"-".repeat(bar_len - filled);
        let speed_mb = speed / MB as f64;
        eprint!(
            "\r  [{client_name}] [{bar}] {:5.1}% @ {:.1} MB/s",
            progress * 100.0,
            speed_mb
        );

        tokio::time::sleep(std::time::Duration::from_millis(config::POLL_INTERVAL_MS)).await;
    }
    eprintln!();

    result.total_sec = start.elapsed().as_secs_f64();
    // Use actual throughput (bytes/time) for avg speed — self-reported speeds aren't comparable
    if result.total_sec > 0.0 {
        result.avg_speed_mbps =
            sc.total_size as f64 * 8.0 / result.total_sec / 1_000_000.0;
    }
    result.peak_speed_mbps = peak_speed * 8.0 / 1_000_000.0;

    // Stage timing from history
    let stage_result = if client_name == "sabnzbd" {
        sab.get_stage_timing().await
    } else {
        rnzb.get_stage_timing().await
    };
    if let Ok(stages) = stage_result {
        result.par2_sec = stages.par2_sec;
        result.unpack_sec = stages.unpack_sec;
        // Derive download time from harness-measured total minus post-processing
        // stages.  Client-reported download_time is integer-second granularity
        // (SABnzbd API limitation), while total_sec has full precision.
        result.download_sec =
            (result.total_sec - stages.par2_sec - stages.unpack_sec).max(0.0);
    }
    if result.download_sec == 0.0 {
        result.download_sec = result.total_sec;
    }

    // Internal metrics (rustnzb only)
    if client_name == "rustnzb" {
        match rnzb.get_internal_metrics().await {
            Ok(metrics) => {
                tracing::info!(
                    "  [{client_name}] Internal: {} server(s), {} stage(s), {:.1} MB/s download throughput",
                    metrics.server_stats.len(),
                    metrics.stage_durations.len(),
                    metrics.download_throughput_mbps,
                );
                result.internal_metrics = Some(metrics);
            }
            Err(e) => {
                tracing::warn!("  [{client_name}] Failed to fetch internal metrics: {e}");
            }
        }
    }

    // Docker stats
    let samples = if let Some(handle) = stats_handle {
        handle.stop().await
    } else {
        vec![]
    };

    if !samples.is_empty() {
        let cpus: Vec<f64> = samples.iter().map(|s| s.cpu_pct).collect();
        let mems: Vec<u64> = samples.iter().map(|s| s.mem_bytes).collect();
        let rxs: Vec<f64> = samples.iter().map(|s| s.net_rx_bps).collect();
        let dws: Vec<f64> = samples.iter().map(|s| s.disk_write_bps).collect();
        let iow: Vec<f64> = samples.iter().map(|s| s.iowait_pct).collect();

        let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
        let max_f = |v: &[f64]| v.iter().cloned().fold(0.0f64, f64::max);
        let avg_u = |v: &[u64]| v.iter().sum::<u64>() as f64 / v.len().max(1) as f64;
        let max_u = |v: &[u64]| v.iter().cloned().max().unwrap_or(0);

        result.cpu_avg = avg(&cpus);
        result.cpu_peak = max_f(&cpus);
        result.mem_avg_mb = avg_u(&mems) / MB as f64;
        result.mem_peak_mb = max_u(&mems) as f64 / MB as f64;
        result.net_rx_avg_mbps = avg(&rxs) * 8.0 / 1e6;
        result.net_rx_peak_mbps = max_f(&rxs) * 8.0 / 1e6;
        result.disk_write_avg_mbps = avg(&dws) / MB as f64;
        result.disk_write_peak_mbps = max_f(&dws) / MB as f64;
        result.iowait_avg = avg(&iow);
        result.iowait_peak = max_f(&iow);
        result.timeseries = samples;
    }

    tracing::info!(
        "  [{client_name}] Done: {:.1}s total, {:.1} Mbps avg",
        result.total_sec,
        result.avg_speed_mbps
    );
    result
}
