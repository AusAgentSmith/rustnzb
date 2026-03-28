#!/usr/bin/env bash
#
# Sonarr + rustnzb E2E test harness
#
# Spins up rustnzb + Sonarr, configures Sonarr to use rustnzb as download client
# (SABnzbd compat) with Prowlarr Newznab indexers, adds a show, triggers download,
# and monitors completion.
#
# Usage:
#   ./run.sh              # full run (up → test → teardown)
#   ./run.sh --no-down    # leave stack running after test
#   ./run.sh --down-only  # just tear down a previous run
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# ─── Load .env ───────────────────────────────────────────────────────────────
if [[ -f .env ]]; then
    set -a; source .env; set +a
elif [[ -f .env.example ]]; then
    echo "INFO: No .env found, copying .env.example → .env"
    cp .env.example .env
    set -a; source .env; set +a
fi

# ─── Defaults ────────────────────────────────────────────────────────────────
RUSTNZB_PORT="${RUSTNZB_PORT:-9091}"
SONARR_PORT="${SONARR_PORT:-8989}"
PROWLARR_URL="${PROWLARR_URL:-https://prowlarr.example.com}"
PROWLARR_API_KEY="${PROWLARR_API_KEY:-}"
DOWNLOAD_TIMEOUT="${DOWNLOAD_TIMEOUT:-1800}"

NNTP_HOST="${NNTP_HOST:-}"
NNTP_PORT="${NNTP_PORT:-563}"
NNTP_USER="${NNTP_USER:-}"
NNTP_PASS="${NNTP_PASS:-}"
NNTP_CONNECTIONS="${NNTP_CONNECTIONS:-4}"

NNTP_BACKUP_HOST="${NNTP_BACKUP_HOST:-}"
NNTP_BACKUP_PORT="${NNTP_BACKUP_PORT:-563}"
NNTP_BACKUP_USER="${NNTP_BACKUP_USER:-}"
NNTP_BACKUP_PASS="${NNTP_BACKUP_PASS:-}"
NNTP_BACKUP_CONNECTIONS="${NNTP_BACKUP_CONNECTIONS:-4}"

RUSTNZB_URL="http://localhost:${RUSTNZB_PORT}"
SONARR_URL="http://localhost:${SONARR_PORT}"
SONARR_API_KEY=""  # populated after Sonarr boots

# ─── Colours ─────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'

log()  { echo -e "${CYAN}[$(date +%H:%M:%S)]${NC} $*"; }
ok()   { echo -e "${GREEN}[$(date +%H:%M:%S)] ✓${NC} $*"; }
warn() { echo -e "${YELLOW}[$(date +%H:%M:%S)] ⚠${NC} $*"; }
err()  { echo -e "${RED}[$(date +%H:%M:%S)] ✗${NC} $*"; }
die()  { err "$@"; exit 1; }

# ─── Helpers ─────────────────────────────────────────────────────────────────
sonarr_api() {
    local method="$1" endpoint="$2"; shift 2
    curl -sf -X "$method" \
        -H "X-Api-Key: ${SONARR_API_KEY}" \
        -H "Content-Type: application/json" \
        "${SONARR_URL}/api/v3${endpoint}" "$@"
}

rustnzb_api() {
    local endpoint="$1"; shift
    curl -sf "${RUSTNZB_URL}${endpoint}" "$@"
}

jq_or_die() {
    if ! command -v jq &>/dev/null; then
        die "jq is required. Install with: sudo apt install jq"
    fi
}

# ─── Teardown ────────────────────────────────────────────────────────────────
teardown() {
    log "Tearing down stack..."
    docker compose down -v --remove-orphans 2>/dev/null || true
    ok "Stack removed"
}

# ─── Parse args ──────────────────────────────────────────────────────────────
NO_DOWN=false
case "${1:-}" in
    --down-only) teardown; exit 0 ;;
    --no-down)   NO_DOWN=true ;;
    --help|-h)
        echo "Usage: $0 [--no-down|--down-only|--help]"
        exit 0
        ;;
esac

trap 'if [[ "$NO_DOWN" == false ]]; then teardown; fi' EXIT

jq_or_die

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 1: Start the stack
# ═════════════════════════════════════════════════════════════════════════════
log "${BOLD}Phase 1: Starting Docker Compose stack${NC}"

# Create data dirs (avoid permission issues)
mkdir -p data/rustnzb/downloads/{incomplete,complete} data/sonarr/tv

docker compose pull --ignore-buildable 2>/dev/null || docker compose pull sonarr 2>/dev/null || true
docker compose up -d

# ─── Wait for rustnzb health ─────────────────────────────────────────────────
log "Waiting for rustnzb to be healthy..."
for i in $(seq 1 60); do
    if rustnzb_api "/api/health" &>/dev/null; then
        ok "rustnzb is ready (attempt $i)"
        break
    fi
    [[ $i -eq 60 ]] && die "rustnzb failed to start within 60s"
    sleep 2
done

# ─── Wait for Sonarr health ─────────────────────────────────────────────────
log "Waiting for Sonarr to be healthy..."
for i in $(seq 1 90); do
    if curl -sf "${SONARR_URL}/ping" &>/dev/null; then
        ok "Sonarr is ready (attempt $i)"
        break
    fi
    [[ $i -eq 90 ]] && die "Sonarr failed to start within 180s"
    sleep 2
done

# ─── Extract Sonarr API key from container ───────────────────────────────────
log "Extracting Sonarr API key..."
for i in $(seq 1 30); do
    SONARR_API_KEY=$(docker exec sonarr-e2e-sonarr \
        cat /config/config.xml 2>/dev/null \
        | grep -oP '(?<=<ApiKey>)[^<]+' || true)
    [[ -n "$SONARR_API_KEY" ]] && break
    sleep 2
done
[[ -z "$SONARR_API_KEY" ]] && die "Could not extract Sonarr API key"
ok "Sonarr API key: ${SONARR_API_KEY:0:8}..."

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 2: Configure Sonarr
# ═════════════════════════════════════════════════════════════════════════════
log "${BOLD}Phase 2: Configuring Sonarr${NC}"

# ─── 2a: Ensure rustnzb has required categories ──────────────────────────────
log "Ensuring rustnzb has 'tv' and 'movies' categories..."

EXISTING_CATS=$(rustnzb_api "/api/config/categories" 2>/dev/null || echo "[]")
for CAT_NAME in tv movies; do
    if echo "$EXISTING_CATS" | jq -e --arg n "$CAT_NAME" '.[] | select(.name == $n)' &>/dev/null; then
        ok "  Category '${CAT_NAME}' already exists"
    else
        CAT_RESULT=$(rustnzb_api "/api/config/categories" \
            -X POST -H "Content-Type: application/json" \
            -d "{\"name\": \"${CAT_NAME}\", \"output_dir\": \"${CAT_NAME}\", \"post_processing\": 3}" 2>&1 || true)
        if echo "$CAT_RESULT" | jq -e '.status == true or .name' &>/dev/null 2>&1; then
            ok "  Category '${CAT_NAME}' created"
        else
            warn "  Failed to create category '${CAT_NAME}': ${CAT_RESULT}"
        fi
    fi
done

# ─── 2a-ii: Configure NNTP servers on rustnzb ────────────────────────────────
if [[ -z "$NNTP_HOST" ]]; then
    die "NNTP_HOST is not set — rustnzb needs at least one NNTP server to download"
fi

EXISTING_SERVERS=$(rustnzb_api "/api/config/servers" 2>/dev/null || echo "[]")

# Add primary server
if echo "$EXISTING_SERVERS" | jq -e '.[] | select(.host == "'"${NNTP_HOST}"'")' &>/dev/null; then
    ok "Primary NNTP server '${NNTP_HOST}' already configured"
else
    log "Adding primary NNTP server: ${NNTP_HOST}..."
    SRV_RESULT=$(rustnzb_api "/api/config/servers" \
        -X POST -H "Content-Type: application/json" \
        -d "{
            \"id\": \"\",
            \"name\": \"Primary\",
            \"host\": \"${NNTP_HOST}\",
            \"port\": ${NNTP_PORT},
            \"ssl\": true,
            \"ssl_verify\": true,
            \"username\": \"${NNTP_USER}\",
            \"password\": \"${NNTP_PASS}\",
            \"connections\": ${NNTP_CONNECTIONS},
            \"priority\": 0,
            \"enabled\": true,
            \"retention\": 0,
            \"pipelining\": 1,
            \"optional\": false
        }" 2>&1 || true)
    if echo "$SRV_RESULT" | jq -e '.status' &>/dev/null 2>&1; then
        ok "Primary NNTP server added"
    else
        die "Failed to add primary server: ${SRV_RESULT}"
    fi
fi

# Add backup server (if configured)
if [[ -n "$NNTP_BACKUP_HOST" ]]; then
    if echo "$EXISTING_SERVERS" | jq -e '.[] | select(.host == "'"${NNTP_BACKUP_HOST}"'")' &>/dev/null; then
        ok "Backup NNTP server '${NNTP_BACKUP_HOST}' already configured"
    else
        log "Adding backup NNTP server: ${NNTP_BACKUP_HOST}..."
        SRV_RESULT=$(rustnzb_api "/api/config/servers" \
            -X POST -H "Content-Type: application/json" \
            -d "{
                \"id\": \"\",
                \"name\": \"Backup\",
                \"host\": \"${NNTP_BACKUP_HOST}\",
                \"port\": ${NNTP_BACKUP_PORT},
                \"ssl\": true,
                \"ssl_verify\": true,
                \"username\": \"${NNTP_BACKUP_USER}\",
                \"password\": \"${NNTP_BACKUP_PASS}\",
                \"connections\": ${NNTP_BACKUP_CONNECTIONS},
                \"priority\": 1,
                \"enabled\": true,
                \"retention\": 0,
                \"pipelining\": 1,
                \"optional\": true
            }" 2>&1 || true)
        if echo "$SRV_RESULT" | jq -e '.status' &>/dev/null 2>&1; then
            ok "Backup NNTP server added"
        else
            warn "Failed to add backup server: ${SRV_RESULT}"
        fi
    fi
fi

# ─── 2b: Add rustnzb as download client (SABnzbd compat) ─────────────────────
log "Adding rustnzb as download client (SABnzbd compat)..."

# Check if already configured
EXISTING_DC=$(sonarr_api GET "/downloadclient" | jq -r '.[] | select(.name == "rustnzb") | .id // empty' 2>/dev/null || true)

if [[ -n "$EXISTING_DC" ]]; then
    ok "Download client 'rustnzb' already exists (id=$EXISTING_DC)"
    DC_ID="$EXISTING_DC"
else
    # rustnzb is on the same docker network, reachable via container name
    DOWNLOAD_CLIENT_PAYLOAD=$(cat <<'DCEOF'
{
  "enable": true,
  "protocol": "usenet",
  "priority": 1,
  "removeCompletedDownloads": true,
  "removeFailedDownloads": true,
  "name": "rustnzb",
  "implementation": "Sabnzbd",
  "configContract": "SabnzbdSettings",
  "fields": [
    {"name": "host",           "value": "sonarr-e2e-rustnzb"},
    {"name": "port",           "value": 9090},
    {"name": "useSsl",         "value": false},
    {"name": "urlBase",        "value": ""},
    {"name": "apiKey",         "value": "rustnzb"},
    {"name": "tvCategory",     "value": "tv"},
    {"name": "recentTvPriority",  "value": 0},
    {"name": "olderTvPriority",   "value": 0}
  ],
  "tags": []
}
DCEOF
    )

    DC_RESULT=$(sonarr_api POST "/downloadclient" -d "$DOWNLOAD_CLIENT_PAYLOAD" 2>&1 || true)
    DC_ID=$(echo "$DC_RESULT" | jq -r '.id // empty')
    [[ -n "$DC_ID" ]] || die "Failed to add download client. Response: $DC_RESULT"
    ok "Download client added (id=$DC_ID)"
fi

# ─── 2c: Add Prowlarr indexers ───────────────────────────────────────────────
log "Fetching indexer list from Prowlarr..."

PROWLARR_INDEXERS=$(curl -sf \
    -H "X-Api-Key: ${PROWLARR_API_KEY}" \
    "${PROWLARR_URL}/api/v1/indexer" 2>/dev/null || true)

if [[ -z "$PROWLARR_INDEXERS" ]]; then
    die "Cannot reach Prowlarr at ${PROWLARR_URL}"
fi

# Check existing indexers
EXISTING_INDEXERS=$(sonarr_api GET "/indexer" | jq -r '.[].name // empty' 2>/dev/null || true)

# Only add Usenet/Newznab indexers (we have a Usenet download client)
echo "$PROWLARR_INDEXERS" | jq -c '.[] | select(.protocol == "usenet")' | while read -r idx; do
    IDX_ID=$(echo "$idx" | jq -r '.id')
    IDX_NAME=$(echo "$idx" | jq -r '.name')
    FULL_NAME="${IDX_NAME} (Prowlarr)"

    if echo "$EXISTING_INDEXERS" | grep -qF "$FULL_NAME"; then
        ok "  Indexer '${FULL_NAME}' already exists, skipping"
        continue
    fi

    log "  Adding indexer: ${FULL_NAME} (usenet/Newznab)..."

    INDEXER_PAYLOAD=$(cat <<EOF
{
  "enableRss": true,
  "enableAutomaticSearch": true,
  "enableInteractiveSearch": true,
  "protocol": "usenet",
  "priority": 25,
  "seasonSearchMaximumSingleEpisodeAge": 0,
  "downloadClientId": 0,
  "name": "${FULL_NAME}",
  "implementation": "Newznab",
  "configContract": "NewznabSettings",
  "fields": [
    {"name": "baseUrl",    "value": "${PROWLARR_URL}/${IDX_ID}/"},
    {"name": "apiPath",    "value": "/api"},
    {"name": "apiKey",     "value": "${PROWLARR_API_KEY}"},
    {"name": "categories", "value": [5000, 5030, 5040, 5020]}
  ],
  "tags": []
}
EOF
    )

    RESULT=$(sonarr_api POST "/indexer" -d "$INDEXER_PAYLOAD" 2>&1 || true)
    if echo "$RESULT" | jq -e '.id' &>/dev/null 2>&1; then
        ok "  Indexer '${IDX_NAME}' added (id=$(echo "$RESULT" | jq -r '.id'))"
    else
        warn "  Failed to add '${IDX_NAME}': $(echo "$RESULT" | jq -r '.[0].errorMessage // .message // "unknown"' 2>/dev/null || echo "$RESULT")"
    fi
done

INDEXER_LIST=$(sonarr_api GET "/indexer")
INDEXER_TOTAL=$(echo "$INDEXER_LIST" | jq 'length')
ok "Total indexers configured: ${INDEXER_TOTAL}"

# ─── 2d: Configure root folder ──────────────────────────────────────────────
EXISTING_RF=$(sonarr_api GET "/rootfolder" | jq -r '.[].path // empty' 2>/dev/null || true)
if echo "$EXISTING_RF" | grep -qF "/tv"; then
    ok "Root folder /tv already exists"
else
    log "Adding root folder for TV shows..."
    ROOT_FOLDER_PAYLOAD='{"path": "/tv", "qualityProfileId": 1, "metadataProfileId": 1}'
    RF_RESULT=$(sonarr_api POST "/rootfolder" -d "$ROOT_FOLDER_PAYLOAD" 2>&1 || true)
    if echo "$RF_RESULT" | jq -e '.id' &>/dev/null 2>&1; then
        ok "Root folder /tv added"
    else
        warn "Root folder response: $(echo "$RF_RESULT" | jq -r '.[0].errorMessage // .message // "already exists?"' 2>/dev/null || echo "$RF_RESULT")"
    fi
fi

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 3: Add series
# ═════════════════════════════════════════════════════════════════════════════
log "${BOLD}Phase 3: Adding series${NC}"

# ─── 3a: Look up the series ─────────────────────────────────────────────────
SERIES_TVDB="${SERIES_TVDB:-457770}"
SERIES_SEARCH="${SERIES_SEARCH:-rooster}"

log "Looking up '${SERIES_SEARCH}' on Sonarr..."
LOOKUP=$(sonarr_api GET "/series/lookup?term=tvdb:${SERIES_TVDB}" 2>&1 || true)
SHOW=$(echo "$LOOKUP" | jq -c '.[0] // empty')

if [[ -z "$SHOW" || "$SHOW" == "null" ]]; then
    log "  TVDB lookup failed, trying text search..."
    LOOKUP=$(sonarr_api GET "/series/lookup?term=${SERIES_SEARCH}" 2>&1 || true)
    SHOW=$(echo "$LOOKUP" | jq -c '[.[] | select(.title | test("'"${SERIES_SEARCH}"'"; "i"))][0] // empty')
fi

[[ -z "$SHOW" || "$SHOW" == "null" ]] && die "Could not find series"

SHOW_TITLE=$(echo "$SHOW" | jq -r '.title')
SHOW_TVDBID=$(echo "$SHOW" | jq -r '.tvdbId')
SHOW_YEAR=$(echo "$SHOW" | jq -r '.year')
ok "Found: ${SHOW_TITLE} (${SHOW_YEAR}) — TVDB: ${SHOW_TVDBID}"

# ─── 3b: Get quality profile ────────────────────────────────────────────────
QUALITY_PROFILE_ID=$(sonarr_api GET "/qualityprofile" | jq -r '.[0].id // 1')

# ─── 3c: Determine latest season ────────────────────────────────────────────
LATEST_SEASON=$(echo "$SHOW" | jq '[.seasons[].seasonNumber] | max')
log "Latest season: ${LATEST_SEASON}"

# ─── 3d: Build the series add payload ───────────────────────────────────────
# Monitor only the latest season
SEASONS_JSON=$(echo "$SHOW" | jq --argjson latest "$LATEST_SEASON" '
    [.seasons[] | {
        seasonNumber: .seasonNumber,
        monitored: (.seasonNumber == $latest)
    }]
')

ADD_SERIES_PAYLOAD=$(cat <<EOF
{
  "title": "${SHOW_TITLE}",
  "tvdbId": ${SHOW_TVDBID},
  "qualityProfileId": ${QUALITY_PROFILE_ID},
  "rootFolderPath": "/tv",
  "monitored": true,
  "seasonFolder": true,
  "seriesType": "standard",
  "monitorNewItems": "all",
  "addOptions": {
    "monitor": "latestSeason",
    "searchForMissingEpisodes": false,
    "searchForCutoffUnmetEpisodes": false
  },
  "seasons": ${SEASONS_JSON}
}
EOF
)

log "Adding series to Sonarr..."
ADD_RESULT=$(sonarr_api POST "/series" -d "$ADD_SERIES_PAYLOAD" 2>&1 || true)
SERIES_ID=$(echo "$ADD_RESULT" | jq -r '.id // empty')

if [[ -z "$SERIES_ID" ]]; then
    # Maybe already exists
    EXISTING=$(sonarr_api GET "/series" | jq -r --argjson tvdb "$SHOW_TVDBID" '.[] | select(.tvdbId == $tvdb) | .id // empty')
    if [[ -n "$EXISTING" ]]; then
        SERIES_ID="$EXISTING"
        warn "Series already existed (id=$SERIES_ID)"
    else
        die "Failed to add series. Response: $ADD_RESULT"
    fi
fi
ok "Series added (id=${SERIES_ID})"

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 4: Trigger single episode search
# ═════════════════════════════════════════════════════════════════════════════
log "${BOLD}Phase 4: Searching for a single episode${NC}"

# Get episodes for the latest season and pick the first aired one
EPISODES_LIST=$(sonarr_api GET "/episode?seriesId=${SERIES_ID}")
NOW_ISO=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TARGET_EP=$(echo "$EPISODES_LIST" | jq -c --argjson s "$LATEST_SEASON" --arg now "$NOW_ISO" '
    [.[] | select(.seasonNumber == $s and .hasFile == false and .airDateUtc != null and .airDateUtc < $now)]
    | sort_by(.episodeNumber) | last // empty
')

if [[ -z "$TARGET_EP" || "$TARGET_EP" == "null" ]]; then
    die "No downloadable episodes found for season ${LATEST_SEASON}"
fi

EPISODE_ID=$(echo "$TARGET_EP" | jq -r '.id')
EP_NUM=$(echo "$TARGET_EP" | jq -r '.episodeNumber')
EP_TITLE=$(echo "$TARGET_EP" | jq -r '.title // "TBA"')
ok "Target: S${LATEST_SEASON}E$(printf '%02d' "$EP_NUM") — ${EP_TITLE} (episode id=${EPISODE_ID})"

SEARCH_PAYLOAD=$(cat <<EOF
{
  "name": "EpisodeSearch",
  "episodeIds": [${EPISODE_ID}]
}
EOF
)

SEARCH_RESULT=$(sonarr_api POST "/command" -d "$SEARCH_PAYLOAD")
COMMAND_ID=$(echo "$SEARCH_RESULT" | jq -r '.id // empty')
[[ -n "$COMMAND_ID" ]] || die "Failed to trigger search. Response: $SEARCH_RESULT"
ok "Episode search triggered (command=$COMMAND_ID)"

# ─── Wait for search to complete ─────────────────────────────────────────────
log "Waiting for search command to complete..."
for i in $(seq 1 60); do
    CMD_STATUS=$(sonarr_api GET "/command/${COMMAND_ID}" | jq -r '.status // "unknown"')
    case "$CMD_STATUS" in
        completed) ok "Search completed"; break ;;
        failed)    die "Search command failed" ;;
        *)         ;;
    esac
    [[ $i -eq 60 ]] && die "Search timed out after 120s"
    sleep 2
done

# ─── Check if anything was grabbed ──────────────────────────────────────────
sleep 5  # give Sonarr a moment to process results and send grabs
QUEUE=$(sonarr_api GET "/queue?includeUnknownSeriesItems=true&includeSeries=true")
QUEUE_COUNT=$(echo "$QUEUE" | jq '.totalRecords // 0')

if [[ "$QUEUE_COUNT" -eq 0 ]]; then
    warn "No items in Sonarr queue — search may not have found matching releases."
    log "Checking Sonarr history for grab events..."
    HISTORY=$(sonarr_api GET "/history?seriesId=${SERIES_ID}&eventType=grabbed&pageSize=10")
    GRAB_COUNT=$(echo "$HISTORY" | jq '.totalRecords // 0')
    if [[ "$GRAB_COUNT" -eq 0 ]]; then
        warn "No grabs found in history either. This could mean:"
        warn "  - No indexer results matched quality/size profiles"
        warn "  - Indexers are unreachable from this network"
        warn "  - No NZBs available for the latest season"
        log ""
        log "You can check manually:"
        log "  Sonarr UI:  ${SONARR_URL}"
        log "  rustnzb UI: ${RUSTNZB_URL}"
        log "  API key:    ${SONARR_API_KEY}"
        if [[ "$NO_DOWN" == true ]]; then
            log "Stack is still running (--no-down). Investigate and re-run search manually."
            exit 0
        fi
        die "No downloads were triggered — cannot monitor completion"
    fi
    ok "Found ${GRAB_COUNT} grab(s) in history (may be processing)"
fi

ok "Queue has ${QUEUE_COUNT} item(s)"
echo "$QUEUE" | jq -r '.records[] | "  → \(.title // .sourceTitle // "unknown") [\(.status)]"'

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 5: Monitor rustnzb for download completion
# ═════════════════════════════════════════════════════════════════════════════
log "${BOLD}Phase 5: Monitoring rustnzb for download completion${NC}"

START_TIME=$(date +%s)
LAST_PRINT=0

while true; do
    ELAPSED=$(( $(date +%s) - START_TIME ))
    if [[ $ELAPSED -ge $DOWNLOAD_TIMEOUT ]]; then
        die "Download timed out after ${DOWNLOAD_TIMEOUT}s"
    fi

    # Check rustnzb queue via SABnzbd compat API
    QUEUE_RESP=$(rustnzb_api "/sabnzbd/api?mode=queue&output=json" 2>/dev/null || echo '{"queue":{"slots":[]}}')
    SLOT_COUNT=$(echo "$QUEUE_RESP" | jq '.queue.slots | length')
    QUEUE_STATUS=$(echo "$QUEUE_RESP" | jq -r '.queue.status // "unknown"')
    QUEUE_SPEED=$(echo "$QUEUE_RESP" | jq -r '.queue.speed // "0"')

    # Also check history for completed items
    HIST_RESP=$(rustnzb_api "/sabnzbd/api?mode=history&output=json&limit=5" 2>/dev/null || echo '{"history":{"slots":[]}}')
    HIST_COUNT=$(echo "$HIST_RESP" | jq '.history.slots | length')
    COMPLETED_COUNT=$(echo "$HIST_RESP" | jq '[.history.slots[] | select(.status == "Completed")] | length')

    if [[ "$SLOT_COUNT" -gt 0 ]]; then
        # Report progress on active downloads
        SLOT_NAME=$(echo "$QUEUE_RESP" | jq -r '.queue.slots[0].filename // "unknown"')
        SLOT_PCT=$(echo "$QUEUE_RESP" | jq -r '.queue.slots[0].percentage // "0"')
        SLOT_SIZE=$(echo "$QUEUE_RESP" | jq -r '.queue.slots[0].size // "?"')
        SLOT_LEFT=$(echo "$QUEUE_RESP" | jq -r '.queue.slots[0].sizeleft // "?"')
        SLOT_STATUS=$(echo "$QUEUE_RESP" | jq -r '.queue.slots[0].status // "unknown"')

        if [[ $(( ELAPSED - LAST_PRINT )) -ge 10 ]]; then
            log "  [${SLOT_NAME}] ${SLOT_PCT}% of ${SLOT_SIZE} | left: ${SLOT_LEFT} | ${QUEUE_SPEED} | ${SLOT_STATUS}"
            LAST_PRINT=$ELAPSED
        fi
    elif [[ "$COMPLETED_COUNT" -gt 0 ]]; then
        # Downloads finished — check if our item completed
        LAST_NAME=$(echo "$HIST_RESP" | jq -r '.history.slots[0].name // "unknown"')
        LAST_STATUS=$(echo "$HIST_RESP" | jq -r '.history.slots[0].status // "unknown"')
        LAST_BYTES=$(echo "$HIST_RESP" | jq -r '.history.slots[0].bytes // 0')

        if [[ "$LAST_STATUS" == "Completed" ]]; then
            LAST_SIZE=$(numfmt --to=iec "$LAST_BYTES" 2>/dev/null || echo "${LAST_BYTES}")
            ok "Download complete: ${LAST_NAME} (${LAST_SIZE})"
            break
        elif [[ "$LAST_STATUS" == "Failed" ]]; then
            FAIL_MSG=$(echo "$HIST_RESP" | jq -r '.history.slots[0].fail_message // "unknown error"')
            die "Download failed: ${LAST_NAME} — ${FAIL_MSG}"
        fi
    else
        if [[ $(( ELAPSED - LAST_PRINT )) -ge 15 ]]; then
            log "  Waiting for download to appear in rustnzb... (${ELAPSED}s)"
            LAST_PRINT=$ELAPSED
        fi
    fi

    sleep 5
done

# ═════════════════════════════════════════════════════════════════════════════
# PHASE 6: Verify completed downloads
# ═════════════════════════════════════════════════════════════════════════════
log "${BOLD}Phase 6: Verifying completed downloads${NC}"

DOWNLOADS_DIR="${RUSTNZB_DOWNLOADS:-./data/rustnzb/downloads}"
# Give rustnzb a moment to finish post-processing and move files
for i in $(seq 1 24); do
    if [[ -n "$(ls -A "$DOWNLOADS_DIR/complete" 2>/dev/null)" ]]; then
        ok "File found in completed folder:"
        ls -lhR "$DOWNLOADS_DIR/complete" | head -20
        break
    fi
    [[ $i -eq 24 ]] && warn "Completed folder is empty — post-processing may still be running"
    sleep 5
done

ELAPSED_TOTAL=$(( $(date +%s) - START_TIME ))
echo ""
echo -e "${GREEN}${BOLD}═══════════════════════════════════════════════════${NC}"
echo -e "${GREEN}${BOLD}  E2E Test Complete  (${ELAPSED_TOTAL}s)${NC}"
echo -e "${GREEN}${BOLD}═══════════════════════════════════════════════════${NC}"
echo -e "  Sonarr UI:  ${SONARR_URL}"
echo -e "  rustnzb UI: ${RUSTNZB_URL}"
echo -e "  API key:    ${SONARR_API_KEY}"
echo ""
