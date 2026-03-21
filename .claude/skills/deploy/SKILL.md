---
name: deploy
description: Deploy rustnzbd (pull latest image, restart container)
disable-model-invocation: true
allowed-tools: Bash(ssh *), Bash(curl *)
user-invocable: true
argument-hint: "[--build] [--down] [--logging] [--status]"
---

# Deploy rustnzbd

Manage the rustnzbd Docker deployment on the deploy host.

Host and network details are in `DEPLOY.local.md` (gitignored).
Default deploy host: set DEPLOY_HOST env var or see DEPLOY.local.md.

## Usage

- `/deploy` — Pull latest image and restart
- `/deploy --build` — Build from source and restart
- `/deploy --down` — Stop the stack
- `/deploy --logging` — Deploy with Promtail logging to Loki enabled
- `/deploy --status` — Check current deployment status without changes

## Steps

1. SSH to deploy host: `ssh -o ConnectTimeout=10 $DEPLOY_HOST`
   (Default from DEPLOY.local.md — check that file for the actual host)
2. Working directory: `cd ~/rustnzbd`

3. If `--status` (read-only check):
   ```bash
   docker compose ps
   curl -sf http://localhost:9095/api/status | python3 -m json.tool
   docker images --format '{{.Repository}}:{{.Tag}} {{.Size}} {{.CreatedAt}}' | grep rustnzbd
   ```
   Stop here — no changes made.

4. If `--down`:
   ```bash
   docker compose down
   ```

5. If `--logging`:
   ```bash
   LOKI_URL=$LOKI_URL HOSTNAME=$(hostname) COMPOSE_PROFILES=logging docker compose pull
   LOKI_URL=$LOKI_URL HOSTNAME=$(hostname) COMPOSE_PROFILES=logging docker compose up -d
   ```
   (Set LOKI_URL from DEPLOY.local.md)

6. If `--build` (build from source — requires repo on host):
   ```bash
   docker compose up -d --build
   ```

7. Default (pull latest published image):
   ```bash
   docker compose pull
   docker compose up -d
   ```

8. Post-deploy checks:
   - Wait 5s
   - Health check: `curl -sf http://localhost:9095/api/status`
   - Container status: `docker compose ps`
   - If logging enabled: `docker logs rustnzbd-promtail-1 --tail 5`

## CI/CD

rustnzbd has a GitHub Actions workflow (`.github/workflows/docker-deploy.yml`) that:
1. Builds and pushes to GHCR + Docker Hub on push to `main`
2. Runs a smoke test (`--smoke-test`) against the built image to verify par2/unrar/7z
3. Auto-deploys via self-hosted runner

So `/deploy` is mainly for manual re-deploys or config changes.

## Ports

| Port | Service |
|------|---------|
| 9095 (host) → 9090 (container) | rustnzbd web UI + API |
