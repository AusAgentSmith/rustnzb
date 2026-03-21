---
name: build
description: Build rustnzbd locally with cargo
disable-model-invocation: true
allowed-tools: Bash(cargo *), Bash(docker *), Bash(ls *)
user-invocable: true
argument-hint: "[--release] [--docker] [--check] [--clippy]"
---

# Build rustnzbd

Build the rustnzbd project.

## Usage

- `/build` — Debug build
- `/build --release` — Release build
- `/build --docker` — Build Docker image locally
- `/build --check` — Type-check without building (fast)
- `/build --clippy` — Run clippy lints

## Steps

1. If `--docker`:
   ```bash
   docker build -t rustnzbd:local .
   ```

2. If `--check`:
   ```bash
   cargo check --workspace
   ```

3. If `--clippy`:
   ```bash
   cargo clippy --workspace -- -D warnings
   ```

4. If `--release`:
   ```bash
   cargo build --release
   ```

5. Default (debug):
   ```bash
   cargo build
   ```

6. Report build result — if errors, show the first error clearly
7. On success, show binary size for release builds:
   ```bash
   ls -lh target/release/rustnzbd
   ```

## Notes

- First build downloads par2cmdline-turbo (~2.9 MB) via the par2-sys crate build script
- Workspace has 6 crates: nzb-core, nzb-web, nzb-nntp, nzb-decode, nzb-postproc, par2-sys
- Rust edition 2024, resolver v3
- Docker build uses rust:1.88-bookworm and produces a debian:bookworm-slim runtime image
