#!/usr/bin/env bash
# 72h 监控脚本：收集内存、CPU、连接状态、日志大小

set -euo pipefail

PID_A="${1:-}"
PID_B="${2:-}"
LOG_DIR="${3:-/tmp/syncthing-72h/logs}"
SYNC_DIR="${4:-/tmp/syncthing-72h/sync}"
INTERVAL=60

mkdir -p "$LOG_DIR/metrics"

echo "[monitor] Starting (interval: ${INTERVAL}s)"
echo "timestamp,node_a_rss_mb,node_b_rss_mb,node_a_cpu,node_b_cpu,log_a_mb,log_b_mb,a_files,b_files,conn_state" > "$LOG_DIR/metrics/timeseries.csv"

while true; do
    ts=$(date +%Y-%m-%d_%H:%M:%S)

    # RSS 内存 (MB)
    rss_a=0
    rss_b=0
    cpu_a=0
    cpu_b=0
    if [[ -n "$PID_A" && -f /proc/$PID_A/status ]]; then
        rss_a=$(awk '/VmRSS/{print int($2/1024)}' /proc/$PID_A/status 2>/dev/null || echo 0)
        cpu_a=$(ps -p "$PID_A" -o %cpu= 2>/dev/null | tr -d ' ' || echo 0)
    fi
    if [[ -n "$PID_B" && -f /proc/$PID_B/status ]]; then
        rss_b=$(awk '/VmRSS/{print int($2/1024)}' /proc/$PID_B/status 2>/dev/null || echo 0)
        cpu_b=$(ps -p "$PID_B" -o %cpu= 2>/dev/null | tr -d ' ' || echo 0)
    fi

    # 日志大小 (MB)
    log_a=0
    log_b=0
    [[ -f "$LOG_DIR/node_a.log" ]] && log_a=$(stat -c%s "$LOG_DIR/node_a.log" 2>/dev/null | awk '{print int($1/1048576)}')
    [[ -f "$LOG_DIR/node_b.log" ]] && log_b=$(stat -c%s "$LOG_DIR/node_b.log" 2>/dev/null | awk '{print int($1/1048576)}')

    # 文件数量
    a_files=$(find "$SYNC_DIR/a" -type f 2>/dev/null | wc -l)
    b_files=$(find "$SYNC_DIR/b" -type f 2>/dev/null | wc -l)

    # 连接状态（从日志推断）
    conn_state="unknown"
    if [[ -f "$LOG_DIR/node_a.log" ]]; then
        if grep -q "Connection manager stopped" "$LOG_DIR/node_a.log" 2>/dev/null; then
            conn_state="stopped"
        elif grep -q "BEP connection established" "$LOG_DIR/node_a.log" 2>/dev/null; then
            conn_state="connected"
        fi
    fi

    echo "$ts,$rss_a,$rss_b,$cpu_a,$cpu_b,$log_a,$log_b,$a_files,$b_files,$conn_state" >> "$LOG_DIR/metrics/timeseries.csv"

    # 告警检查
    if [[ "$rss_a" -gt 512 ]] || [[ "$rss_b" -gt 512 ]]; then
        echo "[ALERT] $ts RSS > 512MB: A=${rss_a}MB B=${rss_b}MB" >> "$LOG_DIR/metrics/alerts.log"
    fi
    if [[ "$log_a" -gt 1024 ]] || [[ "$log_b" -gt 1024 ]]; then
        echo "[ALERT] $ts Log > 1GB: A=${log_a}MB B=${log_b}MB" >> "$LOG_DIR/metrics/alerts.log"
    fi
    if [[ "$a_files" -ne "$b_files" ]]; then
        echo "[WARN] $ts File count mismatch: A=$a_files B=$b_files" >> "$LOG_DIR/metrics/alerts.log"
    fi

    sleep $INTERVAL
done
