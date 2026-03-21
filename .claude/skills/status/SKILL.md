---
name: status
description: Check rustnzb deployment status, health, and queue
disable-model-invocation: true
allowed-tools: Bash(ssh *), Bash(curl *)
user-invocable: true
argument-hint: ""
---

# Check rustnzb Status

Quick health check of the rustnzb deployment.

Host and port details are in `DEPLOY.local.md` (gitignored).

## Usage

- `/status` — Show container status, health, queue, and speed

## Steps

1. Read `DEPLOY.local.md` to get DEPLOY_HOST and API_URL.

2. Check container status on deploy host:
   ```bash
   ssh -o ConnectTimeout=10 $DEPLOY_HOST \
     "cd ~/rustnzb && docker compose ps"
   ```

3. Query the API for live status:
   ```bash
   curl -sf $API_URL/api/status 2>/dev/null | python3 -m json.tool
   ```

4. Query the queue:
   ```bash
   curl -sf $API_URL/api/queue 2>/dev/null | python3 -c "
   import json, sys
   data = json.load(sys.stdin)
   jobs = data if isinstance(data, list) else data.get('jobs', data.get('queue', []))
   if not jobs:
       print('Queue: empty')
   else:
       for j in jobs:
           name = j.get('name', j.get('filename', '?'))
           status = j.get('status', '?')
           pct = j.get('percentage', j.get('progress', '?'))
           print(f'  {status:15s} {pct:>5}%  {name}')
   "
   ```

5. Check recent history (last 5):
   ```bash
   curl -sf $API_URL/api/history 2>/dev/null | python3 -c "
   import json, sys
   data = json.load(sys.stdin)
   entries = data if isinstance(data, list) else data.get('history', [])
   for e in entries[:5]:
       name = e.get('name', e.get('filename', '?'))
       status = e.get('status', '?')
       print(f'  {status:15s}  {name}')
   if not entries:
       print('History: empty')
   "
   ```

6. Summarize: container up/down, current speed, queue depth, recent completions/failures
