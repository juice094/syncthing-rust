#!/usr/bin/env bash
# 72h 测试报告生成器

set -euo pipefail

LOG_DIR="${1:-$HOME/syncthing-72h/logs}"
REPORT_FILE="$LOG_DIR/report.md"

echo "=== Generating 72h Stress Test Report ==="

cat > "$REPORT_FILE" <> REPORT
# Syncthing-Rust 72h Stress Test Report

**Generated:** $(date -Iseconds)

## 1. 进程存活检查

REPORT

for node in node_a node_b; do
    pid_file="$LOG_DIR/${node}.pid"
    if [[ -f "$pid_file" ]]; then
        pid=$(cat "$pid_file")
        if kill -0 "$pid" 2>/dev/null; then
            echo "- $node: PID $pid **running** ✅" >> "$REPORT_FILE"
        else
            echo "- $node: PID $pid **dead** ❌" >> "$REPORT_FILE"
        fi
    else
        echo "- $node: PID file missing ❌" >> "$REPORT_FILE"
    fi
done

echo "" >> "$REPORT_FILE"
echo "## 2. 内存与 CPU 统计" >> "$REPORT_FILE"
if [[ -f "$LOG_DIR/metrics/timeseries.csv" ]]; then
    echo "```" >> "$REPORT_FILE"
    echo "Metric | Node A | Node B" >> "$REPORT_FILE"
    echo "---|---|---" >> "$REPORT_FILE"
    echo "Max RSS (MB) | $(awk -F, 'NR>1 && $2>max{max=$2} END{print max+0}' "$LOG_DIR/metrics/timeseries.csv") | $(awk -F, 'NR>1 && $3>max{max=$3} END{print max+0}' "$LOG_DIR/metrics/timeseries.csv")" >> "$REPORT_FILE"
    echo "Avg RSS (MB) | $(awk -F, 'NR>1{sum+=$2; n++} END{print int(sum/n)}' "$LOG_DIR/metrics/timeseries.csv") | $(awk -F, 'NR>1{sum+=$3; n++} END{print int(sum/n)}' "$LOG_DIR/metrics/timeseries.csv")" >> "$REPORT_FILE"
    echo "Max CPU (%) | $(awk -F, 'NR>1 && $4>max{max=$4} END{print max+0}' "$LOG_DIR/metrics/timeseries.csv") | $(awk -F, 'NR>1 && $5>max{max=$5} END{print max+0}' "$LOG_DIR/metrics/timeseries.csv")" >> "$REPORT_FILE"
    echo "```" >> "$REPORT_FILE"
else
    echo "No metrics data found." >> "$REPORT_FILE"
fi

echo "" >> "$REPORT_FILE"
echo "## 3. 日志与告警" >> "$REPORT_FILE"
if [[ -f "$LOG_DIR/metrics/alerts.log" ]]; then
    alert_count=$(wc -l < "$LOG_DIR/metrics/alerts.log")
    echo "- Total alerts: $alert_count" >> "$REPORT_FILE"
    echo "- Last 10 alerts:" >> "$REPORT_FILE"
    echo "```" >> "$REPORT_FILE"
    tail -10 "$LOG_DIR/metrics/alerts.log" >> "$REPORT_FILE"
    echo "```" >> "$REPORT_FILE"
else
    echo "No alerts." >> "$REPORT_FILE"
fi

echo "" >> "$REPORT_FILE"
echo "## 4. 文件同步一致性" >> "$REPORT_FILE"
if [[ -d "$LOG_DIR/../sync/a" && -d "$LOG_DIR/../sync/b" ]]; then
    diff -rq "$LOG_DIR/../sync/a" "$LOG_DIR/../sync/b" > /tmp/sync_diff.txt 2>&1 || true
    diff_count=$(wc -l < /tmp/sync_diff.txt)
    if [[ "$diff_count" -eq 0 ]]; then
        echo "- Sync folders are **identical** ✅" >> "$REPORT_FILE"
    else
        echo "- Differences found: $diff_count ❌" >> "$REPORT_FILE"
        echo "```" >> "$REPORT_FILE"
        head -20 /tmp/sync_diff.txt >> "$REPORT_FILE"
        echo "```" >> "$REPORT_FILE"
    fi
else
    echo "Sync folders not found." >> "$REPORT_FILE"
fi

echo "" >> "$REPORT_FILE"
echo "## 5. 关键错误日志" >> "$REPORT_FILE"
for node in node_a node_b; do
    log_file="$LOG_DIR/${node}.log"
    if [[ -f "$log_file" ]]; then
        error_count=$(grep -c "ERROR\|panic\|deadlock" "$log_file" 2>/dev/null || echo 0)
        echo "- $node errors/panics: $error_count" >> "$REPORT_FILE"
    fi
done

echo "" >> "$REPORT_FILE"
echo "---" >> "$REPORT_FILE"
echo "Report saved to: $REPORT_FILE"
