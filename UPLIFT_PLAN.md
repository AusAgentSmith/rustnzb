# rustnzbd Uplift Plan: Add Newsreader & Angular UI

## Goal

Transform rustnzbd from a headless NZB downloader into a full **NZBGet competitor with built-in newsgroup browsing**. Two major workstreams:

1. **Add newsreader features** — Browse groups, download headers, search articles, select binaries to download
2. **Replace vanilla JS UI with Angular SPA** — Modern, maintainable, component-based frontend

## Current State

rustnzbd already has:
- Complete NZB download pipeline (queue, yEnc decode, file assembly, PAR2, extract)
- Multi-server NNTP with pooling, pipelining, failover
- SABnzbd API compatibility (Sonarr/Radarr ready)
- RSS feed monitoring with download rules
- Queue management (pause/resume/priority/categories)
- History, logging, JWT auth
- Vanilla JS web UI (3,087-line index.html)
- Tauri desktop wrapper
- 50+ REST API endpoints

The nzb-nntp crate already has GROUP, XOVER, LIST ACTIVE, ARTICLE commands — just not exposed in the web UI.

---

## Phase 1: Backend — Newsreader API Endpoints

Add endpoints for browsing Usenet directly from the app. No schema changes needed for the NNTP operations (they're live queries), but we need tables for subscribed groups and cached headers.

### 1.1 New Database Tables

```sql
-- Subscribed newsgroups
CREATE TABLE groups (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL UNIQUE,
    description TEXT,
    subscribed  INTEGER NOT NULL DEFAULT 0,
    article_count INTEGER NOT NULL DEFAULT 0,
    first_article INTEGER NOT NULL DEFAULT 0,
    last_article  INTEGER NOT NULL DEFAULT 0,
    last_scanned  INTEGER NOT NULL DEFAULT 0,
    last_updated  TEXT,
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Cached article headers from XOVER
CREATE TABLE headers (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id    INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    article_num INTEGER NOT NULL,
    subject     TEXT NOT NULL,
    author      TEXT NOT NULL,
    date        TEXT NOT NULL,
    message_id  TEXT NOT NULL,
    references_ TEXT NOT NULL DEFAULT '',
    bytes       INTEGER NOT NULL DEFAULT 0,
    lines       INTEGER NOT NULL DEFAULT 0,
    read        INTEGER NOT NULL DEFAULT 0,
    downloaded_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- FTS5 for header search
CREATE VIRTUAL TABLE headers_fts USING fts5(
    subject, author, content='headers', content_rowid='id',
    tokenize='porter unicode61'
);
-- + insert/delete/update triggers to keep FTS in sync
```

### 1.2 New API Endpoints

Add to the Axum router in `crates/nzb-web/src/server.rs`:

```
# Group browsing
GET    /api/groups                    # List groups (filter: subscribed, search)
POST   /api/groups/refresh            # Fetch LIST ACTIVE from server
GET    /api/groups/{id}               # Group details
GET    /api/groups/{id}/status        # Article count, new available, unread
POST   /api/groups/{id}/subscribe     # Subscribe
POST   /api/groups/{id}/unsubscribe   # Unsubscribe

# Header browsing
GET    /api/groups/{id}/headers       # Paginated headers (search via FTS5)
POST   /api/groups/{id}/headers/fetch # Trigger XOVER download (background)
GET    /api/groups/{id}/threads       # Threaded view (grouped by References)
GET    /api/groups/{id}/threads/{root_msg_id}  # Thread detail with depth
POST   /api/groups/{id}/headers/mark-read      # Bulk mark read
POST   /api/groups/{id}/headers/mark-all-read  # Mark all read

# Article reading
GET    /api/articles/{message_id}     # Fetch article from NNTP
GET    /api/articles/{message_id}/body # Body only

# NZB generation from selected headers
POST   /api/groups/{id}/headers/download  # Select headers → create NZB → add to queue
```

The last endpoint is the **key integration point**: user browses headers, selects binary posts, and the backend generates an NZB from those message-IDs and adds it to the download queue. This bridges the newsreader and downloader.

### 1.3 Implementation

All the code for these features already exists in rustNewsreader. Port from:

| Feature | Source (rustNewsreader) | Target (rustnzbd) |
|---------|------------------------|---------------------|
| Group list/subscribe | `nr-core/src/db.rs` (group CRUD) | `nzb-core/src/db.rs` |
| Header fetch + store | `nr-web/src/services/header.rs` | `nzb-web/src/handlers.rs` |
| Thread detection | `nr-core/src/db.rs` (list_threads) | `nzb-core/src/db.rs` |
| FTS5 search | `nr-core/src/db.rs` (FTS5 MATCH) | `nzb-core/src/db.rs` |
| Read/unread tracking | `nr-core/src/db.rs` (read_articles) | `nzb-core/src/db.rs` |
| LIST ACTIVE | `nzb-nntp` (already shared) | Already available |

Note: rustnzbd uses `rusqlite` (not sqlx). The SQL is the same, just different Rust API.

### 1.4 "Download Selected" Flow

This is the killer feature that connects newsreading to downloading:

1. User browses headers in a group
2. Selects articles (e.g., all parts of "Movie.2024.BluRay.1080p")
3. Clicks "Download Selected"
4. Backend:
   - Groups selected message-IDs by filename (parse subject for part numbers)
   - Generates an NZB XML in memory (reuse `nzb_generator.rs` from rustnzbindxer)
   - Calls `queue_manager.add_job()` with the generated NZB
5. Download appears in queue immediately

---

## Phase 2: Angular SPA Frontend

Replace the 3,087-line vanilla JS `index.html` with a proper Angular application.

### 2.1 Project Setup

```
rustnzbd/
  frontend/              # NEW: Angular workspace
    src/app/
      core/
        services/        # API, WebSocket, auth services
        models/          # TypeScript interfaces matching Rust models
        guards/          # Auth guard
      features/
        queue/           # Download queue (main view)
        history/         # Completed downloads
        groups/          # Newsgroup browser (NEW)
        headers/         # Article headers + threaded view (NEW)
        rss/             # RSS feeds + rules
        settings/        # Server, category, general config
        logs/            # Log viewer
      shared/
        components/      # Progress bar, speed graph, toolbar
```

### 2.2 Views (NZBGet-style layout)

**Tab-based navigation** matching NZBGet's proven UX:

| Tab | Content |
|-----|---------|
| **Queue** | Active downloads with progress bars, speed, ETA, pause/resume |
| **Groups** | Subscribed groups sidebar + header list + article preview (3-panel) |
| **History** | Completed/failed downloads, retry button |
| **RSS** | Feed items, download rules |
| **Settings** | Servers, categories, general config |
| **Logs** | Real-time log stream |

The **Groups** tab is the new newsreader view. Everything else maps 1:1 to existing API endpoints.

### 2.3 Queue View (primary)

```
┌─────────────────────────────────────────────────────────┐
│ [+Add NZB] [Pause All] [Resume]       ▼ 45.2 MB/s      │
├─────────────────────────────────────────────────────────┤
│ ▼ Movie.2024.1080p            DOWNLOADING  [████░░] 62% │
│   4.2 GB · ETA 3:42 · High · Movies                     │
│                                                          │
│ ● TV.Show.S01E01              UNPACKING    [██████] 100%│
│   1.1 GB · Completed · TV                               │
│                                                          │
│ ○ Software.Package            QUEUED                     │
│   650 MB · Normal · Software                             │
├─────────────────────────────────────────────────────────┤
│ Queue │ Groups │ History │ RSS │ Settings │ Logs         │
└─────────────────────────────────────────────────────────┘
```

### 2.4 Groups View (newsreader)

```
┌──────────────┬──────────────────────────────────────────┐
│ Groups       │ alt.binaries.multimedia                   │
│              │ ┌──────────────────────────────────────┐  │
│ alt.bin.*  9 │ │ Subject           Author    Size Date│  │
│ comp.*     2 │ │ □ Movie.2024 [1/50] post@  4.2G 3/28│  │
│              │ │ □ Movie.2024 [2/50] post@  4.2G 3/28│  │
│              │ │ ☑ TV.Show.S01 [1/20] up@   1.1G 3/27│  │
│              │ │ ...                                  │  │
│              │ ├──────────────────────────────────────┤  │
│              │ │ [Download Selected] [Mark All Read]  │  │
│              │ │ Article preview pane                 │  │
│              │ └──────────────────────────────────────┘  │
└──────────────┴──────────────────────────────────────────┘
```

### 2.5 Port from rustNewsreader

These Angular components already exist and can be ported:

| Component | Source (rustNewsreader) | Adaptation |
|-----------|------------------------|------------|
| Group sidebar | `newsreader-view.component.ts` (groups panel) | Extract, style for tab |
| Header list (flat) | Same component (header-table section) | Add checkbox selection |
| Thread view | Same component (threaded section) | Port as-is |
| Article preview | Same component (article panel) | Port as-is |
| Compose dialog | `compose-dialog.component.ts` | Port as-is |
| Server form | `server-form-dialog.component.ts` | Port as-is |
| API service | `api.service.ts` | Add auth headers |
| WebSocket service | `websocket.service.ts` | Port as-is |
| Models | `*.model.ts` | Extend with download models |

**New components to build:**

| Component | Purpose |
|-----------|---------|
| Queue list | Download queue with progress bars |
| History list | Completed downloads |
| RSS feed manager | Feed CRUD + item browser |
| Settings page | Tabbed config editor |
| Log viewer | Real-time log stream |
| Speed graph | Canvas-based speed chart |
| Auth screens | Login + setup |

### 2.6 Build Integration

Same approach as rustNewsreader:
- `build.rs` runs `ng build --configuration=production`
- `rust-embed` embeds `frontend/dist/` into binary
- Single binary serves both API and SPA
- Dev mode: `ng serve` with proxy to `:9090`

---

## Phase 3: Polish & Differentiation

### 3.1 Features NZBGet doesn't have
- **Built-in newsgroup browser** — Browse, search, and download directly from groups
- **Article preview** — Read text articles inline
- **Header threading** — Conversation view for group discussions
- **FTS5 search** — Fast full-text search across cached headers
- **Generate NZB from headers** — Select articles → instant download

### 3.2 Feature parity with NZBGet
- [ ] Speed graph (canvas chart)
- [ ] Per-server download statistics
- [ ] Scheduler (speed limits by time of day)
- [ ] Notification system
- [ ] Custom post-processing scripts

---

## Implementation Order

1. **Backend: Add group/header tables + API endpoints** (1-2 days)
   - Port SQL and handler code from rustNewsreader
   - Adapt from sqlx to rusqlite
   - Add "download selected" endpoint

2. **Angular: Set up frontend project** (1 day)
   - ng new, Angular Material, proxy config
   - Auth service + guards
   - Core services (API, WebSocket)

3. **Angular: Queue + History views** (1-2 days)
   - Port existing vanilla JS functionality to Angular components
   - Progress bars, speed display, pause/resume

4. **Angular: Groups tab** (1 day)
   - Port from rustNewsreader's newsreader-view component
   - Add checkbox selection + "Download Selected" button

5. **Angular: Settings + RSS + Logs** (1-2 days)
   - Server/category CRUD dialogs
   - RSS feed manager
   - Log viewer

6. **Testing: Playwright E2E** (1 day)
   - Port test infrastructure from rustNewsreader
   - Adapt for rustnzbd's data model

7. **Desktop: Update Tauri wrapper** (0.5 day)
   - Point at new Angular build output

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/nzb-core/src/db.rs` | Add groups, headers, headers_fts tables + CRUD |
| `crates/nzb-core/src/models.rs` | Add GroupRow, HeaderRow, ThreadSummary models |
| `crates/nzb-web/src/server.rs` | Add ~15 new routes |
| `crates/nzb-web/src/handlers.rs` | Add group/header/article handler functions |
| `crates/nzb-web/static/` | Replace with `frontend/dist/` via rust-embed |
| `Cargo.toml` | No changes needed (dependencies already present) |
| `build.rs` | NEW: Run ng build before cargo build |
| `frontend/` | NEW: Entire Angular project |
