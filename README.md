# rustnzbd

A high-performance Usenet NZB download client written in Rust.

rustnzbd is a full-featured binary newsreader that downloads, decodes, verifies, repairs, and extracts files from Usenet. It provides a web UI, REST API, and a SABnzbd-compatible API so it works as a drop-in replacement with Sonarr, Radarr, and other *arr applications.

## Features

- **Fast async downloads** — Tokio-based async I/O with NNTP request pipelining and multi-server support
- **SIMD-accelerated decoding** — yEnc decoding via yenc-simd with per-segment CRC32 verification
- **Automatic post-processing** — par2 verify/repair, RAR/7z/ZIP extraction, cleanup
- **Multi-server failover** — Priority-based server selection with automatic failover and health tracking
- **SABnzbd API compatibility** — Works with Sonarr, Radarr, Lidarr, and other *arr applications out of the box
- **Web UI** — Embedded single-page application for managing downloads
- **REST API** — Full-featured API with Swagger/OpenAPI documentation at `/swagger-ui`
- **RSS feeds** — Automatic monitoring and downloading from RSS feeds with regex filtering
- **Directory watching** — Auto-enqueue `.nzb` files dropped into a watch directory
- **Bandwidth control** — Configurable speed limits with runtime adjustment via API
- **OpenTelemetry** — Optional metrics and log export via OTLP gRPC
- **Single binary** — par2cmdline-turbo is embedded at build time, no system par2 install needed
- **SQLite persistence** — Queue and history survive restarts
- **Docker-ready** — Multi-stage Dockerfile, health checks, non-root user

## Quick Start

### Docker (Recommended)

```bash
docker run -d \
  --name rustnzbd \
  -p 9090:9090 \
  -v ./config:/config \
  -v ./data:/data \
  -v /path/to/downloads:/downloads \
  ausagentsmith/rustnzbd:latest
```

Then open `http://localhost:9090` in your browser. Add your NNTP servers via the web UI.

### Docker Compose

```yaml
services:
  rustnzbd:
    image: ausagentsmith/rustnzbd:latest
    container_name: rustnzbd
    restart: unless-stopped
    ports:
      - "9090:9090"
    volumes:
      - ./config:/config
      - ./data:/data
      - /path/to/downloads:/downloads
    environment:
      - TZ=Your/Timezone
      - RUST_LOG=info
    healthcheck:
      test: ["CMD", "curl", "-sf", "http://localhost:9090/api/status"]
      interval: 30s
      timeout: 5s
      retries: 3
```

```bash
docker compose up -d
```

### From Source

**Prerequisites:**
- Rust 1.88+ (edition 2024)
- `unrar` or `unrar-free` — for RAR extraction
- `p7zip-full` — for 7z extraction
- par2 is bundled automatically, no system install needed

```bash
git clone https://github.com/AusAgentSmith/rustnzbd.git
cd rustnzbd
cargo build --release
./target/release/rustnzbd --config config.example.toml
```

Verify external tools work:

```bash
./target/release/rustnzbd --smoke-test
```

## Configuration

rustnzbd uses a TOML configuration file. Copy `config.example.toml` to get started:

```bash
cp config.example.toml config.toml
```

Servers and most settings can also be configured through the web UI.

### Configuration File

```toml
[general]
listen_addr = "0.0.0.0"
port = 8080
# api_key = "your-secret-api-key"
incomplete_dir = "downloads/incomplete"
complete_dir = "downloads/complete"
data_dir = "data"
speed_limit_bps = 0              # 0 = unlimited
cache_size = 524288000           # 500 MB article cache
log_level = "info"
# watch_dir = "watch"            # Auto-enqueue NZBs from this directory
# history_retention = 100        # Max history entries (omit = keep all)

[[servers]]
id = "primary"
name = "My Usenet Server"
host = "news.example.com"
port = 563
ssl = true
ssl_verify = true
username = "user"
password = "pass"
connections = 8
priority = 0                     # 0 = highest priority
enabled = true
retention = 3000                 # days
pipelining = 1

[[servers]]
id = "backup"
name = "Backup Server"
host = "backup.example.com"
port = 563
ssl = true
connections = 4
priority = 1                     # Lower priority = tried after primary
optional = true

[[categories]]
name = "Default"
post_processing = 3              # 0=none, 1=repair, 2=unpack, 3=repair+unpack

[[categories]]
name = "movies"
output_dir = "movies"            # Relative to complete_dir
post_processing = 3

[[categories]]
name = "tv"
output_dir = "tv"
post_processing = 3

# RSS feed monitoring
# [[rss_feeds]]
# name = "My Indexer"
# url = "https://example.com/rss?t=5000"
# poll_interval_secs = 900
# category = "tv"
# filter_regex = "1080p"
# enabled = true

# OpenTelemetry (optional)
[otel]
enabled = false
endpoint = "http://localhost:4317"
service_name = "rustnzbd"
```

### Environment Variables

All CLI arguments can be set via environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `RUSTNZBD_CONFIG` | `config.toml` | Config file path |
| `RUSTNZBD_PORT` | from config | Listen port |
| `RUSTNZBD_LISTEN_ADDR` | from config | Listen address |
| `RUSTNZBD_DATA_DIR` | from config | Data directory |
| `RUSTNZBD_LOG_LEVEL` | `info` | Log level |
| `RUSTNZBD_LOG_FILE` | — | Log file path |
| `RUST_LOG` | — | tracing env filter (advanced) |
| `OTEL_ENABLED` | `false` | Enable OpenTelemetry |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | — | OTLP gRPC endpoint |
| `OTEL_SERVICE_NAME` | `rustnzbd` | Telemetry service name |

## Usage

### CLI

```
rustnzbd [OPTIONS]

Options:
  -c, --config <PATH>        Config file path [default: config.toml]
  -p, --port <PORT>          Listen port
      --listen-addr <ADDR>   Listen address
      --data-dir <PATH>      Data directory
      --log-level <LEVEL>    Log level [default: info]
      --log-file <PATH>      Log file path
      --smoke-test           Verify external tools work, then exit
  -h, --help                 Print help
  -V, --version              Print version
```

### Web UI

Open `http://localhost:9090` (or your configured port) in a browser. The web UI provides:

- Queue management (add, pause, resume, reorder, delete jobs)
- History view with retry for failed downloads
- Server management (add, edit, test, monitor health)
- Category configuration
- RSS feed management
- Speed limiting controls
- Real-time log viewer

### API

Full API documentation is available at `/swagger-ui` when the server is running.

#### Example: Add an NZB by URL

```bash
curl -X POST http://localhost:9090/api/queue/add-url \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/file.nzb", "category": "movies"}'
```

#### Example: Upload an NZB file

```bash
curl -X POST http://localhost:9090/api/queue/add \
  -F "file=@/path/to/file.nzb" \
  -F "category=tv"
```

#### Example: Check status

```bash
curl http://localhost:9090/api/status
```

### SABnzbd API

rustnzbd implements the SABnzbd API, so it works with any application that supports SABnzbd as a download client.

**In Sonarr/Radarr/etc.:**
- **Host:** your rustnzbd host
- **Port:** your rustnzbd port
- **API Key:** your configured `api_key` (if set)
- **Use SSL:** as applicable
- **Category:** as configured

The SABnzbd API is available at `/sabnzbd/api`.

## Architecture

rustnzbd is organized as a Cargo workspace with 6 focused crates:

| Crate | Purpose |
|-------|---------|
| **nzb-core** | Shared models, TOML config, NZB XML parser, SQLite database |
| **nzb-web** | Axum HTTP server, REST API, queue manager, download engine, SABnzbd compat |
| **nzb-nntp** | Async NNTP client (RFC 3977), connection pool, pipelined downloader |
| **nzb-decode** | SIMD yEnc decoding, CRC32 verification, file assembly |
| **nzb-postproc** | Post-processing pipeline: par2 verify/repair, archive extraction |
| **par2-sys** | Build-time download and runtime embedding of par2cmdline-turbo |

### Download Pipeline

```
NZB file parsed
    │
    ▼
QueueManager creates job (Queued)
    │
    ▼
DownloadEngine starts (Downloading)
    │
    ├── Downloader fetches articles via NNTP (multi-server, pipelined)
    │       │
    │       ▼
    ├── yEnc decoder extracts data + CRC32 verify
    │       │
    │       ▼
    └── FileAssembler writes segments to disk at correct offsets
            │
            ▼
Post-processing pipeline
    ├── par2 verify (skip if 0 failures and no par2 files)
    ├── par2 repair (if verification fails)
    ├── Extract archives (RAR → unrar, 7z → 7z, ZIP → zip crate)
    └── Cleanup par2 + archive files
            │
            ▼
Move to complete_dir/{category}/ (Completed)
```

### Server Failover

Articles are fetched from the highest-priority server first. On failure:
1. Retry on the same server (up to 3 attempts)
2. On 430 (Article Not Found) → immediately try next server
3. On connection error → reconnect, re-queue
4. Only mark article as failed after all servers exhausted
5. Failed articles are recoverable via par2 repair

## Docker Image

The Docker image is built with a two-stage Dockerfile:

1. **Builder stage** (`rust:1.88-bookworm`): Compiles the release binary. The par2-sys crate automatically downloads par2cmdline-turbo and embeds it.
2. **Runtime stage** (`debian:bookworm-slim`): Minimal image with `ca-certificates`, `curl`, `unrar-free`, `p7zip-full`. Runs as non-root user.

### Volumes

| Path | Purpose |
|------|---------|
| `/config` | Configuration files (`config.toml`) |
| `/data` | Database, RSS state, credentials |
| `/downloads` | Incomplete and completed downloads |

### Ports

| Port | Purpose |
|------|---------|
| 9090 | Web UI + REST API |

### Multi-Platform

par2cmdline-turbo binaries are available for:
- Linux: x86_64, aarch64, ARMv7
- macOS: x86_64, Apple Silicon
- Windows: x86_64, aarch64
- FreeBSD: x86_64, aarch64

## Benchmarking

The `benchnzb/` directory contains a benchmark suite that compares rustnzbd against SABnzbd using a mock NNTP server and Docker Compose.

```bash
cd benchnzb

# Quick test (5 GB download, ~5 minutes)
./run.sh --scenarios quick

# Medium test suite (~30 minutes)
./run.sh --scenarios medium

# Full benchmark (all 9 scenarios)
./run.sh --scenarios full
```

**Scenario matrix:** 5GB / 10GB / 50GB x raw download / par2 repair / archive extraction

Results are saved to `benchnzb/results/` as JSON, CSV, and SVG charts.

## Observability

### Logging

rustnzbd uses the `tracing` crate. Control log level via:
- `--log-level` CLI flag
- `RUST_LOG` environment variable (e.g., `RUST_LOG=rustnzbd=debug,nzb_nntp=trace`)
- `log_level` in config.toml

Logs are also available via the web UI log viewer and the `/api/logs` endpoint.

### Loki Integration

A Promtail sidecar is included in `docker-compose.yml` behind the `logging` profile:

```bash
LOKI_URL=http://your-loki:3100 COMPOSE_PROFILES=logging docker compose up -d
```

### OpenTelemetry

Enable OTLP export for metrics and logs:

```toml
[otel]
enabled = true
endpoint = "http://your-collector:4317"
service_name = "rustnzbd"
```

Or via environment variables: `OTEL_ENABLED=true`, `OTEL_EXPORTER_OTLP_ENDPOINT=...`

Exported metrics include `download.speed_bps` and `queue.depth`.

## Development

```bash
# Build (debug)
cargo build

# Build (release)
cargo build --release

# Run all tests
cargo test --workspace

# Run specific crate tests
cargo test -p nzb-decode

# Run integration tests
cargo test --test e2e_download_test

# Run with debug logging
RUST_LOG=debug cargo run -- --config config.example.toml

# Build Docker image locally
docker build -t rustnzbd:local .
```

## License

MIT
