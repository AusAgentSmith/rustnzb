# Stress Test Fairness Issues

Findings from auditing the stress test methodology before publishing results.
Items marked **[FIXED]** have been addressed. Items marked **[OPEN]** need
further investigation — some may reflect genuine SABnzbd limitations rather
than test bugs.

---

## 1. Cleanup loop destroys SABnzbd in-progress downloads — **[FIXED]**

The cleanup loop runs `rm -rf /downloads/incomplete/*` inside the target
container. RustNZB tolerates this because it re-creates working directories
on demand, but SABnzbd stores its `__ADMIN__` metadata and partial article
data in `/downloads/incomplete/<jobname>/`. Deleting these mid-download
causes `FileNotFoundError` on every subsequent article write, permanently
stalling all queued jobs.

**Evidence:** SABnzbd downloaded at ~3.7 Gbps for 70 seconds, then
`active=0` for the remaining 3.5 minutes. Logs show repeated
`Disk error on creating file` and `FileNotFoundError`.

**Fix:** Only clean `/downloads/complete/*` for SABnzbd; let SABnzbd
manage its own incomplete directory.

---

## 2. SABnzbd pauses downloads during post-processing — **[OPEN]**

Even with `pp=0` (category post-processing disabled), SABnzbd still enters
a post-processing phase after each download completes. The logs show:

```
Starting Post-Processing on stress_000002 => Repair:False, Unpack:False
...
Post-processing finished, resuming download
```

SABnzbd **pauses all downloads** while post-processing runs (file moves,
history updates, notification dispatch). With `pp=0` this is brief (~1-2s
per job), but with 5 GB NZBs completing every few seconds it adds up.

RustNZB with `post_processing = 0` skips the pipeline entirely and (as of
commit 0061565) starts the next download before post-processing the
completed job.

**This may be a genuine architectural difference** — SABnzbd's downloader
is single-threaded and blocks on post-processing, while RustNZB handles
it concurrently. Worth documenting in results rather than "fixing".

---

## 3. SABnzbd `kbpersec` unit ambiguity — **[OPEN]**

SABnzbd's API returns `queue.kbpersec` as a string. The stress client
converts it with `kbps * 1024.0` (treating it as KiB/s). If SABnzbd
actually reports in decimal KB/s (1000-based), speed is inflated by 2.4%.

This only affects the charted speed metric, not actual download counts.
Needs verification against SABnzbd source or documentation.

---

## 4. Article cache size asymmetry — **[OPEN]**

RustNZB stress config sets `cache_size = 1073741824` (1 GB) for parsed
article header caching. SABnzbd stress config does not set
`article_cache` explicitly and uses the SABnzbd default (typically much
smaller).

This could affect throughput if disk I/O becomes a bottleneck. Consider
adding `article_cache = 1G` to sabnzbd-stress.ini for parity, or
documenting the difference.

---

## 5. Speed integration for byte totals — **[OPEN]**

Both clients track `total_bytes_downloaded` by integrating
`speed_bps * poll_interval` at each sample. This is an approximation —
actual bytes transferred may differ due to speed fluctuation between
samples, API averaging windows, and protocol overhead.

The metric is used for the summary "Total Downloaded" figure. For
published results, consider computing actual bytes from
`nzbs_completed * nzb_size` instead (which is exact for the synthetic
NNTP server since every article succeeds).

---

## 6. SABnzbd `Error importing <NzbFile>` on large files — **[OPEN]**

During the stress test, SABnzbd logged multiple `Error importing
<NzbFile>` errors for 5 GB files. This may be related to memory
pressure (SABnzbd parses NZB files into memory) or a SABnzbd limitation
with very large single-file NZBs.

**Not a test methodology issue** — this could be a genuine SABnzbd
limitation worth noting in results. Consider testing with smaller NZB
sizes (e.g. 500 MB) to isolate whether this is size-dependent.
