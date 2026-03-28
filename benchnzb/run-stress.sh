#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

cat <<'BANNER'
+=====================================================+
|  benchnzb v2: Stress Test                           |
+=====================================================+
BANNER

usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --client CLIENT      rustnzb or sabnzbd (default: rustnzb)"
    echo "  --duration TIME      How long to run (default: 1h)"
    echo "                       Examples: 30m, 1h, 4h30m, 8h"
    echo "  --nzb-size SIZE      Size of each NZB (default: 5gb)"
    echo "                       Examples: 500mb, 1gb, 5gb, 10gb"
    echo "  --concurrency N      NZBs to keep queued (default: 5)"
    echo "  --connections N      NNTP connections (default: 50)"
    echo "  --no-cleanup         Keep containers after run"
    echo "  --help               Show this help"
    echo ""
    echo "Examples:"
    echo "  $0                                        # 1h rustnzb test"
    echo "  $0 --client sabnzbd --duration 1h         # 1h sabnzbd test"
    echo "  $0 --duration 4h --nzb-size 10gb"
    echo "  $0 --duration 8h --concurrency 10 --connections 100"
    exit 0
}

CLIENT="${CLIENT:-rustnzb}"
DURATION="${DURATION:-1h}"
NZB_SIZE="${NZB_SIZE:-5gb}"
CONCURRENCY="${CONCURRENCY:-5}"
CONNECTIONS="${CONNECTIONS:-50}"
CLEANUP=1

while [[ $# -gt 0 ]]; do
    case "$1" in
        --client)        CLIENT="$2"; shift 2 ;;
        --duration)      DURATION="$2"; shift 2 ;;
        --nzb-size)      NZB_SIZE="$2"; shift 2 ;;
        --concurrency)   CONCURRENCY="$2"; shift 2 ;;
        --connections)   CONNECTIONS="$2"; shift 2 ;;
        --no-cleanup)    CLEANUP=0; shift ;;
        --help|-h)       usage ;;
        *) echo "Unknown: $1"; usage ;;
    esac
done

export CLIENT DURATION NZB_SIZE CONCURRENCY

mkdir -p results

# Determine profile and seed config based on client
if [[ "$CLIENT" == "sabnzbd" ]]; then
    COMPOSE_PROFILES="sabnzbd"

    rm -rf state/sabnzbd
    mkdir -p state/sabnzbd

    # Seed SABnzbd config with requested connection count
    sed "s/connections = 50/connections = ${CONNECTIONS}/" \
        configs/sabnzbd-stress.ini > state/sabnzbd/sabnzbd.ini
else
    COMPOSE_PROFILES="rustnzb"

    rm -rf state/rustnzb
    mkdir -p state/rustnzb

    # Seed RustNZB config with requested connection count
    sed "s/connections = 50/connections = ${CONNECTIONS}/" \
        configs/rustnzb-stress.toml > state/rustnzb/config.toml
fi

export COMPOSE_PROFILES

LOGFILE="results/stress_$(date +%Y%m%d_%H%M%S).log"

docker compose -f docker-compose.stress.yml down -v 2>/dev/null || true

echo "[*] Client:        $CLIENT"
echo "[*] Duration:      $DURATION"
echo "[*] NZB size:      $NZB_SIZE"
echo "[*] Concurrency:   $CONCURRENCY NZBs queued"
echo "[*] Connections:   $CONNECTIONS NNTP connections"
echo "[*] Log file:      $LOGFILE"
echo "[*] Building (first run compiles Rust)..."
echo ""

docker compose -f docker-compose.stress.yml up \
    --build \
    --abort-on-container-exit \
    --exit-code-from stress-runner 2>&1 | tee "$LOGFILE"
EXIT_CODE=${PIPESTATUS[0]}

[[ "$CLEANUP" == "1" ]] && docker compose -f docker-compose.stress.yml down -v 2>/dev/null || true

echo ""
echo "========================================"
echo "  Results: $(pwd)/results/"
echo "  Log:     $LOGFILE"
ls -lh results/ 2>/dev/null | tail -5
echo "========================================"
exit "${EXIT_CODE}"
