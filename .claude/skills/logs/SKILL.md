---
name: logs
description: View Docker logs for rustnzbd on the deploy host
disable-model-invocation: true
allowed-tools: Bash(ssh *), Bash(docker *)
user-invocable: true
argument-hint: "[--tail N] [--follow]"
---

# View rustnzbd Logs

Tail Docker logs for the rustnzbd container on the deploy host.

Host details are in `DEPLOY.local.md` (gitignored).

## Usage

- `/logs` — Last 100 lines
- `/logs --tail 50` — Last 50 lines
- `/logs --tail 200` — Last 200 lines

## Steps

1. Parse `$ARGUMENTS` for `--tail N` (default: 100)
2. SSH to deploy host and tail logs (get host from DEPLOY.local.md):
   ```bash
   ssh -o ConnectTimeout=10 $DEPLOY_HOST \
     "docker logs rustnzbd --tail <N> 2>&1"
   ```
3. Highlight any errors or warnings in the output
4. If promtail is running, also show its status:
   ```bash
   ssh -o ConnectTimeout=10 $DEPLOY_HOST \
     "docker logs rustnzbd-promtail-1 --tail 5 2>&1"
   ```
   (Only if container exists — don't error if not running)
