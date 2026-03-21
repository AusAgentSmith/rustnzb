---
name: bench
description: Run the benchnzb benchmark suite (rustnzbd vs SABnzbd)
disable-model-invocation: true
allowed-tools: Bash(cd *), Bash(./run.sh *), Bash(docker *), Bash(ls *), Bash(cat *)
user-invocable: true
argument-hint: "[--scenarios quick|medium|speed|postproc|full] [--no-cleanup]"
---

# Run Benchmarks

Run the benchnzb performance benchmark suite comparing rustnzbd vs SABnzbd.

## Usage

- `/bench` — Quick benchmark (5 GB raw download, ~5 min)
- `/bench --scenarios medium` — 5+10 GB, all test types (~30 min)
- `/bench --scenarios speed` — 5+10+50 GB raw download (~60 min)
- `/bench --scenarios postproc` — 5+10 GB par2+unpack (~45 min)
- `/bench --scenarios full` — All 9 scenarios (5/10/50 x raw/par2/unpack)
- `/bench --no-cleanup` — Keep containers after run for inspection

## Scenario Groups

| Group | Scenarios | Est. Duration |
|-------|-----------|---------------|
| quick | 5 GB raw | ~5 min |
| medium | 5+10 GB, all types | ~30 min |
| speed | 5+10+50 GB raw | ~60 min |
| postproc | 5+10 GB par2+unpack | ~45 min |
| full | All 9 combinations | ~2 hours |

## Steps

1. Change to benchnzb directory:
   ```bash
   cd /home/sprooty/rustnzbd/benchnzb
   ```

2. Parse `$ARGUMENTS` for `--scenarios` (default: `quick`) and `--no-cleanup`

3. Run the benchmark:
   ```bash
   ./run.sh --scenarios <group>
   ```

4. Show results:
   ```bash
   ls -lh results/
   cat results/benchmark_*.json | python3 -m json.tool | head -100
   ```

## Architecture

The benchmark uses Docker Compose with 4 services:
- **mock-nntp**: Generates yEnc-encoded articles on the fly
- **sabnzbd**: LinuxServer SABnzbd image for comparison
- **rustnzbd**: Built from the parent Dockerfile
- **orchestrator**: benchnzb binary that drives scenarios and collects metrics

Results include JSON, CSV, and SVG charts in `benchnzb/results/`.
