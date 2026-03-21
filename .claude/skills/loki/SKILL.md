---
name: loki
description: Query rustnzb logs from the centralized Loki instance
disable-model-invocation: true
allowed-tools: Bash(curl *)
user-invocable: true
argument-hint: "[filter] [--since duration] [--limit N]"
---

# Query rustnzb Logs from Loki

Query rustnzb logs from the centralized Loki stack. Logs are shipped via a Promtail sidecar.

Loki endpoint and host labels are in `DEPLOY.local.md` (gitignored).

## Usage

- `/loki` — Recent rustnzb logs (last 10 minutes)
- `/loki ERROR` — Error lines only
- `/loki "download complete"` — Filter for specific text
- `/loki --since 1h` — Last hour
- `/loki --since 30m --limit 100` — Last 30 min, up to 100 lines

## Steps

1. Read `DEPLOY.local.md` to get the Loki endpoint URL and host label.

2. Parse `$ARGUMENTS`:
   - Text words → filter (`|=` or `|~` for multiple terms)
   - `--since <duration>` → time range (default `10m`)
   - `--limit <N>` → max lines (default `50`)

3. Build LogQL query — always scoped to rustnzb:
   ```logql
   {container="rustnzb", host="<HOST_LABEL>"}
   ```
   Add `|= "<filter>"` if filter text provided.

4. Execute:
   ```bash
   curl -s -G '<LOKI_URL>/loki/api/v1/query_range' \
     --data-urlencode 'query={container="rustnzb", host="<HOST_LABEL>"} |= "<filter>"' \
     --data-urlencode 'limit=<N>' \
     --data-urlencode 'since=<duration>'
   ```

5. Format output:
   ```bash
   | python3 -c "
   import json, sys
   data = json.load(sys.stdin)
   results = data.get('data', {}).get('result', [])
   lines = []
   for stream in results:
       for ts, line in stream.get('values', []):
           lines.append((int(ts), line[:300]))
   lines.sort()
   for ts, line in lines:
       print(line)
   if not lines:
       print('No results - is promtail running? Try: /deploy --logging')
   "
   ```

6. If no results, suggest:
   - Check promtail is running: `/logs` and look for promtail
   - Broaden time range with `--since`
   - Check Grafana (URL in DEPLOY.local.md)

## LogQL examples

```logql
# All rustnzb logs (replace HOST_LABEL from DEPLOY.local.md)
{container="rustnzb", host="<HOST_LABEL>"}

# Errors only
{container="rustnzb", host="<HOST_LABEL>"} |~ "ERROR|error|Error"

# Download activity
{container="rustnzb", host="<HOST_LABEL>"} |~ "download|Download"

# Connection issues
{container="rustnzb", host="<HOST_LABEL>"} |~ "connection|timeout|refused"
```
