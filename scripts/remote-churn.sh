#!/bin/bash
# Linux file churn script for remote node stress testing.
# Functionally equivalent to churn-files.ps1.
#
# Usage: ./remote-churn.sh <sync-dir> [duration] [log-file]
#   duration: e.g. 72h, 30m, 3600s (default: 72h)

set -euo pipefail

SYNC_DIR="${1:-}"
DURATION="${2:-72h}"
LOG_FILE="${3:-}"

if [[ -z "$SYNC_DIR" ]]; then
    echo "Usage: $0 <sync-dir> [duration] [log-file]" >&2
    exit 1
fi

mkdir -p "$SYNC_DIR"

if [[ -z "$LOG_FILE" ]]; then
    LOG_FILE="$(dirname "$SYNC_DIR")/churn.log"
fi

# Parse duration to seconds
parse_duration() {
    local d="$1"
    local num unit
    num="${d%[hms]}"
    unit="${d: -1}"
    case "$unit" in
        h) echo $((num * 3600)) ;;
        m) echo $((num * 60)) ;;
        s) echo "$num" ;;
        *)
            # If no suffix, try to interpret as seconds; if that fails, default to 72h
            if [[ "$d" =~ ^[0-9]+$ ]]; then
                echo "$d"
            else
                echo $((72 * 3600))
            fi
            ;;
    esac
}

MAX_SECS=$(parse_duration "$DURATION")
START_EPOCH=$(date +%s)

sizes=(1024 $((64 * 1024)) $((1024 * 1024)) $((10 * 1024 * 1024)))
counter=0

log() {
    local msg="$1"
    local ts
    ts=$(date '+%Y-%m-%dT%H:%M:%SZ')
    echo "[$ts] $msg" | tee -a "$LOG_FILE"
}

cleanup() {
    log "Received interrupt, stopping churn."
    exit 0
}
trap cleanup INT TERM

log "Starting churn in $SYNC_DIR, duration=${DURATION} (${MAX_SECS}s)"

while true; do
    NOW=$(date +%s)
    ELAPSED=$((NOW - START_EPOCH))
    if [[ $ELAPSED -ge $MAX_SECS ]]; then
        log "Duration reached, stopping."
        break
    fi

    counter=$((counter + 1))
    size="${sizes[$(((counter - 1) % ${#sizes[@]}))]}"
    file="$SYNC_DIR/$(printf 'file_%04d.dat' $counter)"

    # CREATE: write random bytes
    if command -v dd >/dev/null 2>&1 && [[ -e /dev/urandom ]]; then
        dd if=/dev/urandom of="$file" bs=1 count="$size" status=none 2>/dev/null
    else
        # Fallback: use head + openssl or /dev/urandom via head
        head -c "$size" /dev/urandom > "$file" 2>/dev/null || true
    fi
    log "CREATE $(basename "$file") (${size} bytes)"

    # MODIFY: rewrite file from 3 iterations ago
    if [[ $counter -gt 3 ]]; then
        old_file="$SYNC_DIR/$(printf 'file_%04d.dat' $((counter - 3)))"
        if [[ -f "$old_file" ]]; then
            old_size="${sizes[$(((counter - 4) % ${#sizes[@]}))]}"
            if command -v dd >/dev/null 2>&1; then
                dd if=/dev/urandom of="$old_file" bs=1 count="$old_size" status=none 2>/dev/null
            else
                head -c "$old_size" /dev/urandom > "$old_file" 2>/dev/null || true
            fi
            log "MODIFY $(basename "$old_file")"
        fi
    fi

    # DELETE: remove file from 6 iterations ago
    if [[ $counter -gt 6 ]]; then
        old_file="$SYNC_DIR/$(printf 'file_%04d.dat' $((counter - 6)))"
        if [[ -f "$old_file" ]]; then
            rm -f "$old_file"
            log "DELETE $(basename "$old_file")"
        fi
    fi

    sleep 30
done
