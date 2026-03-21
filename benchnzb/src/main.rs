mod charts;
mod clients;
mod config;
mod datagen;
mod docker;
mod metrics;
mod mock_nntp;
mod nzb;
mod report;
mod runner;
mod yenc;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "benchnzb", about = "Usenet client benchmark: SABnzbd vs rustnzb")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the benchmark orchestrator
    Run {
        #[arg(long, default_value = "quick", env = "SCENARIOS")]
        scenarios: String,
        #[arg(long, default_value = "/data")]
        data_dir: PathBuf,
        #[arg(long, default_value = "/results")]
        results_dir: PathBuf,
    },
    /// Run the mock NNTP server
    MockNntp {
        #[arg(long, default_value = "119")]
        port: u16,
        #[arg(long, default_value = "/data")]
        data_dir: PathBuf,
        #[arg(long, default_value = "8080")]
        health_port: u16,
    },
    /// Regenerate charts from existing benchmark JSON
    RegenCharts {
        #[arg(long)]
        json: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    eprintln!(
        "[benchnzb] PID={} args={:?}",
        std::process::id(),
        std::env::args().collect::<Vec<_>>()
    );

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let result = rt.block_on(async {
        match cli.command {
            Command::Run {
                scenarios,
                data_dir,
                results_dir,
            } => runner::run(scenarios, data_dir, results_dir).await,
            Command::MockNntp {
                port,
                data_dir,
                health_port,
            } => mock_nntp::run(port, data_dir, health_port).await,
            Command::RegenCharts { json, out } => {
                let data = std::fs::read_to_string(&json)?;
                let items: Vec<serde_json::Value> = serde_json::from_str(&data)?;
                let mut results = Vec::new();
                for item in &items {
                    let sab: runner::ClientResult =
                        serde_json::from_value(item["sabnzbd"].clone())?;
                    let rnzb: runner::ClientResult =
                        serde_json::from_value(item["rustnzb"].clone())?;
                    results.push((sab, rnzb));
                }
                std::fs::create_dir_all(&out)?;
                charts::generate_all(&results, &out)?;
                tracing::info!("Charts written to {}", out.display());
                Ok(())
            }
        }
    });

    if let Err(ref e) = result {
        tracing::error!("Fatal error: {e:#}");
    }
    result
}
