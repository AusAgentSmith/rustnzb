# SABnzbd Config Importer — Design Document

## Overview

A first-boot setup wizard in the web UI that lets users import their SABnzbd configuration into rustnzbd. When rustnzbd starts with no servers configured (fresh install), the UI presents a setup flow with the option to "Import from SABnzbd" by uploading their `sabnzbd.ini` file.

---

## Current State

- **No first-boot flow exists.** rustnzbd creates a default config (no servers, one "Default" category) and drops the user straight into the main UI.
- **No INI parsing** exists in the codebase. Config is TOML-only (`nzb-core/src/config.rs`).
- **Web UI** is an embedded React SPA served via `rust-embed` from `crates/nzb-web/static/`.
- **Config is loaded** in `AppConfig::load()` — if no file exists, defaults are written and returned.
- **Config updates** are handled by existing REST endpoints (`PUT /api/config/*`) and persisted back to TOML via `AppConfig::save()`.

---

## Architecture

### Backend: `POST /api/setup/import-sabnzbd`

New endpoint in `nzb-web/src/handlers.rs` that:

1. Accepts a multipart upload of `sabnzbd.ini`
2. Parses the INI file
3. Maps fields to `AppConfig` structs
4. Returns a **preview** JSON (not yet applied) so the UI can show what will be imported
5. User confirms → `POST /api/setup/apply` writes the config

Also needed:

- `GET /api/setup/status` — returns `{ "needs_setup": bool }` based on whether `servers` is empty (first-boot detection)

### Frontend: Setup Wizard

When the UI loads and `/api/setup/status` returns `needs_setup: true`, redirect to a setup wizard with two paths:

1. **Manual setup** — go straight to server config (existing UI)
2. **Import from SABnzbd** — upload `sabnzbd.ini`, preview mapped config, confirm

---

## SABnzbd INI Format → rustnzbd TOML Mapping

### `[misc]` → `[general]`

| SABnzbd INI Field | rustnzbd TOML Field | Notes |
|---|---|---|
| `api_key` | `general.api_key` | Direct copy |
| `complete_dir` | `general.complete_dir` | Path — may need adjustment for Docker vs bare-metal |
| `download_dir` | `general.incomplete_dir` | SABnzbd calls it "download_dir" |
| `bandwidth_limit` | `general.speed_limit_bps` | SABnzbd uses human-readable strings ("50M"), convert to bytes/sec |
| `max_art_tries` | — | No direct equivalent (rustnzbd retries 3x per server, then fails over) |
| `max_connections` | — | Global cap in SABnzbd; rustnzbd manages per-server |
| `enable_unrar` | — | Always enabled in rustnzbd |
| `enable_unzip` | — | Always enabled |
| `enable_7zip` | — | Always enabled |
| `enable_par_cleanup` | — | Always enabled (part of post-proc pipeline) |
| `pause_on_post_processing` | — | Not applicable (rustnzbd queues post-proc naturally) |
| `no_dupes` | — | Duplicate detection not yet implemented (v2) |

### `[servers]` → `[[servers]]`

SABnzbd format:
```ini
[servers]
[[server-name]]
name = My Server
host = news.example.com
port = 563
connections = 20
ssl = 1
ssl_verify = 2
username = user
password = pass
enable = 1
optional = 0
retention = 3000
priority = 0
```

Mapping:

| SABnzbd Field | rustnzbd Field | Notes |
|---|---|---|
| `name` / `displayname` | `name` | Use `displayname` if present, fall back to section key |
| `host` | `host` | Direct |
| `port` | `port` | Direct |
| `ssl` | `ssl` | `0`/`1` → `bool` |
| `ssl_verify` | `ssl_verify` | SABnzbd uses `0`/`1`/`2`; map `0` → `false`, else `true` |
| `username` | `username` | Direct (empty string → `None`) |
| `password` | `password` | Direct (empty string → `None`) |
| `connections` | `connections` | Direct |
| `enable` | `enabled` | `0`/`1` → `bool` |
| `optional` | `optional` | `0`/`1` → `bool` |
| `retention` | `retention` | Direct (days) |
| `priority` | `priority` | Direct (0 = highest, same semantics) |
| `timeout` | — | No equivalent (rustnzbd uses connection pool timeouts) |
| `required` | — | No direct equivalent |
| `expire_date` | — | Not tracked |
| — | `id` | Generate UUID |
| — | `pipelining` | Default to `1` |

### `[categories]` → `[[categories]]`

SABnzbd format:
```ini
[categories]
[[*]]
name = *
order = 0
pp = 3
script = Default
dir = ""
newzbin = ""
priority = -100
```

Mapping:

| SABnzbd Field | rustnzbd Field | Notes |
|---|---|---|
| `name` | `name` | `*` maps to `"Default"` |
| `pp` | `post_processing` | Direct — same 0-3 scale |
| `dir` | `output_dir` | Empty string → `None` |
| `script` | — | External scripts not yet supported (v2, TODO #16) |
| `order` | — | Not applicable |
| `priority` | — | Per-category priority not supported (applied per-job) |
| `newzbin` | — | Legacy field, ignore |

### `[rss]` → `[[rss_feeds]]` (if present)

SABnzbd RSS is more complex (per-feed filters, multiple filter sets). Best-effort mapping:

| SABnzbd Field | rustnzbd Field | Notes |
|---|---|---|
| Feed URL | `url` | Direct |
| Feed name | `name` | Direct |
| `enable` | `enabled` | Bool |
| `pp` | — | Category determines post-processing |
| `cat` | `category` | Map to category name |
| `filter` entries | `filter_regex` | Best-effort: take first include filter regex |

RSS migration should be flagged as "review recommended" in the preview since the filter model differs.

---

## Fields That Cannot Be Migrated

These should be listed in the preview UI under a "Not imported" section with explanations:

| SABnzbd Feature | Reason |
|---|---|
| Notification settings | Not yet implemented (TODO #15) |
| Sorting/renaming rules | Not yet implemented (TODO #14) |
| Post-processing scripts | Not yet implemented (TODO #16) |
| Scheduling rules | Not yet implemented (TODO #18) |
| Duplicate detection settings | Not yet implemented |
| Web UI theme/language | rustnzbd has its own UI |
| `nzb_key` | Not applicable |
| Server `expire_date` | Not tracked |

---

## API Design

### `GET /api/setup/status`

```json
{
  "needs_setup": true,
  "has_servers": false,
  "has_categories": true,
  "version": "0.1.0"
}
```

### `POST /api/setup/import-sabnzbd`

**Request:** multipart form with `file` field containing `sabnzbd.ini`

**Response:**

```json
{
  "servers": [
    {
      "name": "My Usenet Provider",
      "host": "news.example.com",
      "port": 563,
      "ssl": true,
      "ssl_verify": true,
      "username": "user",
      "connections": 20,
      "priority": 0,
      "enabled": true,
      "retention": 3000,
      "optional": false
    }
  ],
  "categories": [
    { "name": "Default", "output_dir": null, "post_processing": 3 },
    { "name": "movies", "output_dir": "movies", "post_processing": 3 },
    { "name": "tv", "output_dir": "tv", "post_processing": 3 }
  ],
  "general": {
    "api_key": "abc123...",
    "complete_dir": "/downloads/complete",
    "incomplete_dir": "/downloads/incomplete",
    "speed_limit_bps": 0
  },
  "rss_feeds": [],
  "warnings": [
    "Sorting rules cannot be imported (not yet supported)",
    "Post-processing script 'cleanup.sh' on category 'tv' cannot be imported",
    "RSS feed 'My Feed' has complex filters — only first include filter was imported, review recommended"
  ],
  "skipped_fields": [
    "misc.no_dupes (duplicate detection not yet supported)",
    "misc.auto_sort (file sorting not yet supported)"
  ]
}
```

### `POST /api/setup/apply`

**Request:** The same JSON structure (possibly modified by the user in the preview UI)

**Response:** `200 OK` — config written, app reloads config via `ArcSwap`

---

## UI Flow

```
┌──────────────────────────────────────┐
│         Welcome to rustnzbd          │
│                                      │
│  ┌──────────────┐ ┌───────────────┐  │
│  │  Fresh Start  │ │ Import from   │  │
│  │              │ │  SABnzbd      │  │
│  └──────────────┘ └───────────────┘  │
└──────────────────────────────────────┘
          │                  │
          ▼                  ▼
   Go to main UI    ┌──────────────────┐
   (add servers     │ Upload sabnzbd.ini│
    manually)       └────────┬─────────┘
                             │
                             ▼
                    ┌──────────────────┐
                    │ Preview Import    │
                    │                  │
                    │ ✅ 2 servers      │
                    │ ✅ 4 categories   │
                    │ ✅ General config  │
                    │ ⚠️  1 warning     │
                    │ ❌ 3 skipped      │
                    │                  │
                    │ [Edit] [Apply]   │
                    └────────┬─────────┘
                             │
                             ▼
                    ┌──────────────────┐
                    │ Config applied!   │
                    │ Redirecting to    │
                    │ main UI...        │
                    └──────────────────┘
```

### Preview Screen Details

The preview should show editable cards for:

- **Servers** — show host, port, connections, SSL status; allow toggling enabled/disabled; allow editing credentials (in case passwords need re-entry)
- **Categories** — show name, output dir, post-processing level
- **General** — show paths (important: user may need to adjust for Docker mount points vs bare-metal paths), speed limit, API key
- **Warnings** — yellow banner listing features that were partially imported
- **Skipped** — collapsible section listing SABnzbd features that couldn't be imported at all

---

## Implementation Plan

### Phase 1: Backend INI Parser + API (Rust)

1. Add `configparser` or `rust-ini` crate to `nzb-core/Cargo.toml`
2. Create `crates/nzb-core/src/sabnzbd_import.rs`:
   - `parse_sabnzbd_ini(content: &str) -> Result<SabnzbdImport>`
   - Structs for the parsed result + warnings
   - Field mapping logic per the tables above
   - Bandwidth string parser ("50M" → bytes/sec)
3. Add handler endpoints in `nzb-web/src/handlers.rs`:
   - `GET /api/setup/status`
   - `POST /api/setup/import-sabnzbd`
   - `POST /api/setup/apply`
4. Wire routes in `server.rs`

### Phase 2: Frontend Setup Wizard (React)

1. Add setup status check on app load — if `needs_setup`, show wizard instead of main UI
2. Build wizard component with two paths (fresh start / import)
3. Build file upload component for `sabnzbd.ini`
4. Build preview/edit screen for imported config
5. Build apply + redirect flow

### Phase 3: Polish

1. Handle edge cases in INI parsing (SABnzbd has evolved its INI format across versions)
2. Path translation hints (e.g., "SABnzbd paths may differ from your rustnzbd Docker mount points")
3. Password masking in preview
4. "Re-import" option accessible from Settings (not just first boot)

---

## Estimated Scope

| Component | Effort |
|---|---|
| INI parser + mapping logic | ~300 lines Rust |
| API endpoints (3 handlers) | ~150 lines Rust |
| Setup wizard UI | ~400 lines React |
| Tests (INI parsing, edge cases) | ~200 lines Rust |
| **Total** | **~1,050 lines** |

---

## Open Questions

1. **Should the import be accessible outside first-boot?** (e.g., Settings → "Import SABnzbd Config") — recommended yes, for users who set up rustnzbd first then decide to migrate
2. **Password handling** — SABnzbd stores passwords in plaintext in the INI. We can import them directly, but should the preview mask them?
3. **Path remapping** — Docker users will have different mount points. Should we offer a "path prefix replacement" (e.g., replace `/mnt/data` with `/downloads`)?
4. **Queue/history import** — SABnzbd stores active queue in a custom binary format (`queue.db` / `queue10.db`). This is significantly more complex and likely not worth the effort. Document as out of scope.

---

## SABnzbd INI Reference

Full reference INI (from `benchnzb/configs/sabnzbd.ini`):

```ini
[misc]
api_key = benchnzb0123456789abcdef01234567
nzb_key = benchnzb0123456789abcdef01234567
complete_dir = /downloads/complete
download_dir = /downloads/incomplete
bandwidth_limit = ""
bandwidth_perc = 100
pre_check = 0
max_art_tries = 3
top_only = 0
auto_sort = ""
check_new_rel = 0
auto_browser = 0
language = en
enable_https = 0
wizard_completed = 1
enable_unrar = 1
enable_unzip = 1
enable_7zip = 1
enable_par_cleanup = 1
max_connections = 50
no_dupes = 0
pause_on_post_processing = 0

[servers]
[[server-name]]
name = server-name
host = news.example.com
port = 119
timeout = 120
username = user
password = pass
connections = 20
ssl = 0
ssl_verify = 0
enable = 1
required = 0
optional = 0
retention = 0
expire_date = ""
priority = 0
displayname = My Server

[categories]
[[*]]
name = *
order = 0
pp = 3
script = Default
dir = ""
newzbin = ""
priority = -100
```

## rustnzbd Config Reference

See `config.example.toml` and `crates/nzb-core/src/config.rs` for the full target structure. Key structs: `AppConfig`, `GeneralConfig`, `ServerConfig`, `CategoryConfig`, `RssFeedConfig`.
