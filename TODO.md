# rustnzbd — Remaining TODO

Status check against sabnzbd/review.md (March 2026). Items ordered by impact.

---

## Functional Gaps

1. [x] **URL import (addurl)** — SABnzbd compat handler is a stub (returns fake nzo_id, never fetches). Sonarr/Radarr use `addurl` for NZB indexer links. Wire up `reqwest` to actually download the NZB and enqueue it.
2. [x] **Queue reordering** — UI has move-up/move-down buttons that toast "not yet supported". Add `POST /api/queue/{id}/move` (or similar) and implement in `queue_manager.rs`.
3. [x] **Priority change after enqueue** — No endpoint to change a job's priority once it's in the queue. Add `PUT /api/queue/{id}/priority`.
4. [x] **Category CRUD via API** — Only `GET /api/config/categories` exists. Add create/update/delete so the UI and API consumers can manage categories without editing TOML.
5. [x] **Download resume on restart** — Queue persists across restarts but unfinished articles restart from scratch. Consider checkpointing per-file segment progress so partially-downloaded jobs don't re-fetch everything.

## Performance

6. [x] **SIMD yEnc decoder** — Replaced with published `yenc-simd` crate.

## API / Integration

7. [x] **Swagger UI wiring** — `utoipa` is in deps but verify the `/swagger-ui` route is actually mounted and working. If not, wire it up.
8. [x] **SABnzbd compat coverage** — Audit which `mode=` values Sonarr, Radarr, and Lidarr actually call. The compat layer covers the basics but may be missing edge cases (e.g. `mode=config`, `mode=get_cats`, `mode=change_cat`). Test with real arr instances.

## Operational

9. [x] **Graceful shutdown** — Verify that in-flight downloads are cleanly stopped and queue state is flushed to SQLite on SIGTERM/SIGINT. Important for Docker deployments.
10. [x] **Disk space checks** — Pre-flight check for available disk space before starting a download. Alert or pause if disk is critically low.
11. [x] **Docker health check** — Add a `/api/health` endpoint (or use `/api/status`) for `HEALTHCHECK` in Docker.

## Nice-to-Have (v2 territory per review.md)

These are explicitly deferred in review.md but worth tracking:

12. [x] Directory watching (watch folder for NZB files)
13. [x] RSS feed monitoring
14. [ ] File sorting / media renaming (guessit equivalent)
15. [ ] Notification system (apprise or similar)
16. [ ] External post-processing scripts
17. [x] Per-job bandwidth limiting
18. [ ] Scheduling (speed limits by time, pause/resume on schedule)
