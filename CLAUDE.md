# CLAUDE.md — rustnzb

## Project Overview

**rustnzb** is a high-performance Usenet NZB download client written in Rust. It provides a REST API, embedded web UI, and SABnzbd-compatible API for integration with *arr applications (Sonarr, Radarr, etc.). The project is a single-binary Cargo workspace (no local sub-crates), deployed as a Docker container. All nzb-* library crates are consumed as external git dependencies (see `~/Working/libs/`), with `[patch]` overrides for local dev.

## Repository Layout

```
rustnzb/
├── src/main.rs                    # Binary entry point (CLI, config, tracing, startup)
├── frontend/                      # Angular 21 SPA (Material, dark theme, tab-based UI)
├── e2e/                           # Playwright E2E tests (15 tests)
├── build.rs                       # Auto-runs ng build during cargo build
├── benchnzb/                      # Benchmark suite: rustnzb vs SABnzbd (excluded from workspace)
├── desktop/                       # Desktop app (excluded from workspace)
├── tests/                         # Integration tests (e2e download, NNTP, post-processing)
├── config.example.toml            # Configuration reference
├── root/                          # s6-overlay service definitions (copied into container)
├── Dockerfile                     # Multi-stage build (rust:1.88-alpine → linuxserver/baseimage-alpine)
├── docker-compose.yml             # Production deployment (with optional Promtail sidecar)
└── .github/workflows/
    └── docker-deploy.yml          # CI/CD: build → smoke test → deploy
```

### External Library Dependencies

All nzb-* crates live in `~/Working/libs/` and are referenced via git tags in `Cargo.toml`. Local `[patch]` sections redirect to the local checkouts for dev builds.

| Crate | Git Tag | Purpose |
|-------|---------|---------|
| nzb-web | v0.2.4 | Axum HTTP server, REST API, queue manager, download engine |
| nzb-nntp | v0.2.0 | Async NNTP client, connection pool, pipelined downloader |
| nzb-core | v0.2.0 | Shared models, config, NZB parser, SQLite database |
| nzb-decode | v0.1.0 | yEnc decoder (SIMD via yenc-simd), file assembler |
| nzb-postproc | v0.2.0 | Post-processing: par2 verify/repair, RAR/7z/ZIP extraction |
| rust-par2 | 0.1.1 | PAR2 repair (crates.io) |
| yenc-simd | 0.1 | SIMD yEnc decoder (crates.io) |

## Architecture

```
                      ┌─────────────┐
                      │   main.rs   │  CLI args, config, tracing, startup
                      └──────┬──────┘
                             │
                      ┌──────▼──────┐
                      │   nzb-web   │  Axum server, REST API, SABnzbd compat
                      │             │  QueueManager (state machine + persistence)
                      │             │  DownloadEngine (orchestrates per-job)
                      └──┬───┬───┬──┘
                         │   │   │
              ┌──────────┘   │   └──────────┐
              ▼              ▼              ▼
        ┌──────────┐  ┌──────────┐  ┌───────────┐
        │ nzb-nntp │  │nzb-decode│  │nzb-postproc│
        │ NNTP pool│  │ yEnc+asm │  │ par2/unrar │
        └──────────┘  └──────────┘  └───────────┘
              │              │              │
              └──────────────┴──────────────┘
                             │
                      ┌──────▼──────┐
                      │  nzb-core   │  Models, Config, NZB parser, SQLite DB
                      └─────────────┘
```

### Key Components

- **QueueManager** (`nzb-web/src/queue_manager.rs`): Central hub. Manages job lifecycle (Queued → Downloading → Verifying → Repairing → Extracting → Completed/Failed), database persistence, download speed tracking, history retention.
- **DownloadEngine** (`nzb-web/src/download_engine.rs`): Per-job orchestrator. Fetches articles via NNTP, decodes yEnc, assembles files, triggers post-processing.
- **Downloader** (`nzb-nntp/src/downloader.rs`): Multi-server article fetcher with priority-based failover, request pipelining, and bandwidth limiting.
- **ConnectionPool** (`nzb-nntp/src/pool.rs`): Per-server async NNTP connection pool with health checks.
- **SABnzbd Compat** (`nzb-web/src/sabnzbd_compat.rs`): Implements the SABnzbd API protocol so Sonarr/Radarr/etc. can use rustnzb as a drop-in replacement.
- **Par2**: Uses native Rust `rust-par2` library — no external binary or subprocess needed.

### Background Services

- **Speed tracker**: Rolling window speed measurement (spawned from QueueManager)
- **Directory watcher** (`nzb-web/src/dir_watcher.rs`): Auto-enqueue `.nzb` files from a watch directory
- **RSS monitor** (`nzb-web/src/rss_monitor.rs`): Poll RSS feeds, filter by regex, auto-enqueue

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Language | Rust 2024 edition |
| Async runtime | Tokio (full features) |
| HTTP framework | Axum 0.8 + Tower middleware |
| TLS | rustls 0.23 (ring provider) |
| Database | SQLite via rusqlite (bundled, WAL mode) |
| Serialization | serde + serde_json, toml, bincode |
| NNTP | Custom async implementation (RFC 3977) |
| Decoding | yenc-simd (SIMD-accelerated yEnc) + crc32fast |
| Post-processing | par2cmdline-turbo (embedded), unrar, 7z (system) |
| Observability | tracing + optional OpenTelemetry (OTLP gRPC) |
| API docs | utoipa + Swagger UI |
| Web UI | Angular 21 SPA (rust-embed, zoneless change detection) |

## Build & Run

### Prerequisites

- Rust toolchain (1.88+, edition 2024)
- System tools for post-processing: `unrar` (or `unrar-free`), `7z` (`p7zip-full`)
- par2 is bundled automatically — no system install needed

### Local Development

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run tests
cargo test --workspace

# Run with custom config
cargo run -- --config config.toml --port 8080

# Smoke test (verify par2, unrar, 7z are available)
cargo run -- --smoke-test
```

### Docker

```bash
# Build image
docker build -t rustnzb:local .

# Run
docker run -p 9090:9090 \
  -e PUID=1000 -e PGID=1000 \
  -v ./config:/config \
  -v ./data:/data \
  -v /path/to/downloads:/downloads \
  rustnzb:local
```

### Docker Compose (Production)

```bash
docker compose up -d

# With Loki logging
LOKI_URL=http://your-loki:3100 HOSTNAME=$(hostname) COMPOSE_PROFILES=logging docker compose up -d
```

## Configuration

Configuration is loaded from TOML with CLI and environment variable overrides.

**Priority order:** CLI args > environment variables > TOML file > defaults

### Key Environment Variables

| Variable | Purpose |
|----------|---------|
| `RUSTNZB_CONFIG` | Config file path (default: `config.toml`) |
| `RUSTNZB_PORT` | Listen port |
| `RUSTNZB_LISTEN_ADDR` | Listen address |
| `RUSTNZB_DATA_DIR` | Data directory |
| `RUSTNZB_LOG_LEVEL` | Log level (trace/debug/info/warn/error) |
| `RUST_LOG` | tracing env filter (overrides log level) |
| `OTEL_ENABLED` | Enable OpenTelemetry (`true`/`1`) |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP gRPC endpoint |
| `OTEL_SERVICE_NAME` | Service name for telemetry |

See `config.example.toml` for the full configuration reference including servers, categories, RSS feeds, and OpenTelemetry settings.

## API Endpoints

### Native API (`/api/`)

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/api/health` | Health check (Docker HEALTHCHECK) |
| GET | `/api/status` | Speed, queue size, disk space |
| GET | `/api/queue` | List all jobs |
| POST | `/api/queue/add` | Upload NZB file (multipart) |
| POST | `/api/queue/add-url` | Add NZB from URL |
| POST | `/api/queue/pause` / `resume` | Pause/resume all downloads |
| DELETE | `/api/queue/{id}` | Remove a job |
| GET | `/api/history` | Completed/failed jobs |
| POST | `/api/history/{id}/retry` | Retry a failed job |
| GET/PUT | `/api/config/*` | Read/update servers, categories, RSS feeds, settings |
| GET | `/swagger-ui` | Interactive API documentation |

### Newsgroup Browsing API (`/api/groups`)

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/api/groups` | List groups (subscribed, search) |
| POST | `/api/groups/refresh` | Fetch LIST ACTIVE from NNTP server |
| GET | `/api/groups/{id}/status` | Group stats (new available, unread) |
| POST | `/api/groups/{id}/subscribe` | Subscribe to group |
| GET | `/api/groups/{id}/headers` | List headers (FTS5 search) |
| POST | `/api/groups/{id}/headers/fetch` | Background XOVER fetch |
| POST | `/api/groups/{id}/headers/download` | Download selected → NZB → queue |
| GET | `/api/groups/{id}/threads` | Threaded conversation view |
| GET | `/api/articles/{message_id}` | Fetch article from NNTP |

### SABnzbd Compatible API (`/sabnzbd/api`)

Supports modes: `addfile`, `addurl`, `queue`, `history`, `config`, `fullstatus`, `version`, `pause`, `resume`, `delete`, `retry`

## CI/CD Pipeline

### GitHub Actions (`.github/workflows/docker-deploy.yml`)

Triggers on push to `main` or manual `workflow_dispatch`. Runs on a **self-hosted runner**.

```
Push to main
    │
    ▼
┌─────────────────────┐
│  build-and-publish   │  Docker Buildx, push to GHCR + Docker Hub
│  (self-hosted)       │  Tags: branch, commit SHA, latest (on main)
│                      │  Cache: GitHub Actions cache
└─────────┬───────────┘
          │
          ▼
┌─────────────────────┐
│  container-test      │  Pull image by SHA, run --smoke-test
│  (self-hosted)       │  Verifies par2, unrar, 7z work in runtime image
│                      │
└─────────┬───────────┘
          │
          ▼
┌─────────────────────┐
│  deploy              │  docker compose pull → up -d → prune
│  (self-hosted)       │  Health check via docker compose ps
└─────────────────────┘
```

### Container Registries

| Registry | Image |
|----------|-------|
| GHCR | `ghcr.io/ausagentsmith-org/rustnzb` |
| Docker Hub | `ausagentsmith-org/rustnzb` |

### Dockerfile

Two-stage build:
1. **Builder** (`rust:1.88-alpine3.21`): `cargo build --release` (musl-linked static binary)
2. **Runtime** (`lscr.io/linuxserver/baseimage-alpine:3.21`): Copies binary, installs `unrar`, `7zip`. Uses s6-overlay for process management with native PUID/PGID support.

Exposes port 9090. Volumes: `/config`, `/data`, `/downloads`.

## Deployment

See `DEPLOY.local.md` (gitignored) for environment-specific deployment details including host IPs, Tailscale mesh, Loki/Grafana endpoints, and reverse proxy configuration.

The generic deployment flow is:
- Container exposes port 9090
- Volumes: `/config`, `/data`, `/downloads`
- Optional Promtail sidecar for centralized logging (see `docker-compose.yml` logging profile)
- Self-hosted GitHub Actions runner builds, tests, and deploys automatically

## Benchmarking

The `benchnzb/` directory contains a comprehensive benchmark suite comparing rustnzb vs SABnzbd.

```bash
cd benchnzb

# Quick benchmark (5 GB, ~5 min)
./run.sh --scenarios quick

# Full benchmark (all 9 scenarios)
./run.sh --scenarios full
```

**Scenarios:** 5GB/10GB/50GB x raw download/par2 repair/archive extraction

Uses a mock NNTP server, Docker Compose orchestration, and generates JSON/CSV/SVG results.

## Claude Code Skills

Available slash commands for this project:

| Command | Description |
|---------|-------------|
| `/build` | Build locally (`--release`, `--docker`, `--check`, `--clippy`) |
| `/test` | Run test suite (`cargo test --workspace`) |
| `/deploy` | Deploy container (`--build`, `--down`, `--logging`, `--status`) |
| `/logs` | View Docker logs from deployment host |
| `/loki` | Query centralized Loki logs |
| `/bench` | Run benchmark suite (`--scenarios quick\|medium\|full`) |
| `/status` | Check deployment health, queue, and history |

## Coding Conventions

- **Rust edition 2024**, workspace resolver v3
- **Async everywhere** — all I/O uses Tokio async/await
- **Error handling**: `thiserror` for library error types, `anyhow` for application-level errors
- **Lock-free reads**: `ArcSwap` for hot config, `parking_lot` for mutexes where needed
- **Logging**: `tracing` macros (`info!`, `warn!`, `error!`) — never `println!`
- **Config changes via API** are persisted back to the TOML file
- **Database**: SQLite in WAL mode, job data stored as bincode-encoded blobs
- **TLS**: rustls with ring crypto provider (installed once at startup before any TLS use)
- **No system par2 needed**: uses pure-Rust `rust-par2` library

## Testing

```bash
# All tests
cargo test --workspace

# Specific crate
cargo test -p nzb-decode

# Integration tests
cargo test --test e2e_download_test
cargo test --test parse_real_nzbs

# With output
cargo test -- --nocapture
```

Integration tests are in `tests/`:
- `e2e_download_test.rs` — Full download pipeline
- `nntp_connection_test.rs` — NNTP protocol
- `e2e_postproc_detection.rs` — Post-processing detection
- `e2e_full_pipeline.rs` — End-to-end workflow
- `parse_real_nzbs.rs` — NZB XML parsing with real files
