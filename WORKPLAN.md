# rustnzb — v1 Work Plan

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                        rustnzb                              │
│                                                              │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────────┐  │
│  │ nzb-web  │  │ nzb-nntp │  │nzb-decode│  │nzb-postproc│  │
│  │          │  │          │  │          │  │            │  │
│  │ axum API │  │ async    │  │ yEnc     │  │ par2 (bin) │  │
│  │ swagger  │  │ NNTP+TLS │  │ CRC32    │  │ unrar(FFI) │  │
│  │ auth     │  │ pipeline │  │ assemble │  │ 7z (crate) │  │
│  │ SABnzbd  │  │ pool     │  │ cache    │  │            │  │
│  │ compat   │  │ bandwidth│  │          │  │            │  │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └─────┬──────┘  │
│       │              │              │               │        │
│  ┌────┴──────────────┴──────────────┴───────────────┴────┐   │
│  │                     nzb-core                           │   │
│  │  NzbJob, NzbFile, Article, Config, Category, Queue,    │   │
│  │  History (SQLite), State persistence, Error types       │   │
│  └────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────┘
```

## Reuse from rustTorrent (rqbit)

| Component | Source | Adaptation Needed |
|-----------|--------|-------------------|
| Auth system (tokens, credentials, login/refresh/logout) | `http_api/auth.rs` | Minimal — rename types |
| API error handling (ApiError, WithStatus traits) | `api_error.rs` | Replace torrent-specific variants |
| Rate limiting (governor-based bandwidth) | `limits.rs` | Remove upload limiter, keep download |
| Category manager | `category.rs` | Add post-processing settings |
| HTTP API setup (axum, CORS, middleware) | `http_api/mod.rs` | Swap handler routes |
| OpenAPI/Swagger (utoipa) | `http_api/mod.rs` | New schema definitions |
| Logging (tracing + SSE stream) | `tracing_subscriber_config_utils.rs` | Reuse as-is |
| WebUI React architecture | `webui/` | Adapt components for NZB domain |
| Frontend API client + auth store | `webui/src/http-api.ts`, `authStore.ts` | Change endpoints |

## Phase Plan

### Phase 1: Project Scaffold + Core Data Model
**Worktree:** main branch
**Files:**
- Workspace `Cargo.toml` with all crate members
- `crates/nzb-core/` — NzbJob, NzbFile, Article structs, Config, Category, error types
- `crates/nzb-core/src/db.rs` — SQLite schema (queue + history tables)
- `src/main.rs` — tokio entry point, config loading, component wiring

### Phase 2: Web API Layer
**Worktree:** `feature/web-api`
**Files:**
- `crates/nzb-web/` — axum server, auth (adapted from rqbit), error handling
- `crates/nzb-web/src/api.rs` — Native REST API endpoints
- `crates/nzb-web/src/sabnzbd_compat.rs` — SABnzbd API compatibility (Sonarr/Radarr)
- `crates/nzb-web/src/openapi.rs` — Swagger/OpenAPI spec
- Queue CRUD endpoints, history query, server config, NZB upload

### Phase 3: NNTP Client
**Worktree:** `feature/nntp`
**Files:**
- `crates/nzb-nntp/src/connection.rs` — Async NNTP state machine
- `crates/nzb-nntp/src/pool.rs` — Per-server connection pool
- `crates/nzb-nntp/src/server.rs` — Server config, health, penalties
- `crates/nzb-nntp/src/pipeline.rs` — Request pipelining
- `crates/nzb-nntp/src/bandwidth.rs` — Speed measurement + limiting (from rqbit)

### Phase 4: Decode + Assembly
**Worktree:** `feature/decode`
**Files:**
- `crates/nzb-decode/src/yenc.rs` — yEnc decoder
- `crates/nzb-decode/src/cache.rs` — Bounded article cache
- `crates/nzb-decode/src/assembler.rs` — Article → file assembly

### Phase 5: Download Orchestrator
**Worktree:** `feature/downloader`
**Files:**
- `crates/nzb-nntp/src/downloader.rs` — Main download loop
- Integrates: queue → NNTP fetch → decode → cache → assemble → completion
- Channel-based pipeline with backpressure

### Phase 6: Post-Processing
**Worktree:** `feature/postproc`
**Files:**
- `crates/nzb-postproc/src/par2.rs` — Shell out to par2 binary
- `crates/nzb-postproc/src/unpack.rs` — RAR (FFI), 7z, ZIP extraction
- `crates/nzb-postproc/src/pipeline.rs` — Orchestrate: verify → repair → extract → cleanup

### Phase 7: Integration + WebUI
**Worktree:** `feature/webui`
**Files:**
- `crates/nzb-web/webui/` — React/Vite SPA (adapted from rqbit)
- End-to-end: upload NZB → download → post-process → history
- SABnzbd API compatibility testing with Sonarr/Radarr

## Implementation Order

```
Phase 1 ──► Phase 2 ──► Phase 3 ──► Phase 4 ──► Phase 5 ──► Phase 6 ──► Phase 7
  core        web        nntp       decode      download    postproc     webui
  model       API        client     yEnc        orchestr.   par2/unrar   SPA
  config      auth       pool       cache       pipeline    7z/zip       e2e
  DB          swagger    pipeline   assembler              cleanup
```

Phases 1-2 can be built and tested independently (mock download data).
Phase 3 can be tested against a real NNTP server independently.
Phase 4 can be tested with captured yEnc article data.
Phase 5 wires 3+4 together.
Phase 6 can be tested independently with sample archives.
Phase 7 is the UI layer on top of everything.
