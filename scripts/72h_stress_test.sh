#!/usr/bin/env bash
# 72h 压力测试部署脚本
# 运行方式: ./scripts/72h_stress_test.sh [node_a_dir] [node_b_dir] [sync_folder] [log_dir]
# 非 localhost 测试: ./scripts/72h_stress_test.sh --peer-addr 192.168.1.100:22001 ...

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Parse optional --peer-addr before positional args
PEER_ADDR=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --peer-addr)
            PEER_ADDR="$2"
            shift 2
            ;;
        --)
            shift
            break
            ;;
        *)
            break
            ;;
    esac
done

NODE_A_DIR="${1:-$HOME/syncthing-72h/node_a}"
NODE_B_DIR="${2:-$HOME/syncthing-72h/node_b}"
SYNC_DIR="${3:-$HOME/syncthing-72h/sync}"
LOG_DIR="${4:-$HOME/syncthing-72h/logs}"
DURATION_HOURS=72
DURATION_SECS=$((DURATION_HOURS * 3600))

echo "=== Syncthing-Rust 72h Stress Test ==="
echo "Node A: $NODE_A_DIR"
echo "Node B: $NODE_B_DIR"
echo "Sync folder: $SYNC_DIR"
echo "Log dir: $LOG_DIR"
echo "Duration: ${DURATION_HOURS}h"
echo ""

mkdir -p "$NODE_A_DIR" "$NODE_B_DIR" "$SYNC_DIR/a" "$SYNC_DIR/b" "$LOG_DIR"

# 编译 release 版本
echo "[1/5] Compiling syncthing-rust (release)..."
cd "$REPO_ROOT"
cargo build --release --bin syncthing 2>&1 | tail -5
BINARY="$REPO_ROOT/target/release/syncthing"

# 生成证书（如果缺失）
generate_cert() {
    local dir=$1
    local name=$2
    if [[ ! -f "$dir/cert.pem" ]]; then
        echo "Generating cert for $name..."
        "$BINARY" --generate-cert --config-dir "$dir" --device-name "$name" 2>/dev/null || true
    fi
}

# 创建配置文件
create_config() {
    local dir=$1
    local port=$2
    local sync_path=$3
    local peer_id=$4
    local peer_addr="${5:-tcp://127.0.0.1:$((port == 22000 ? 22001 : 22000))}"
    cat > "$dir/config.json" <> CONFIG
{
  "device_name": "$(basename $dir)",
  "listen_addr": "0.0.0.0:$port",
  "gui": { "enabled": false },
  "options": { "relays_enabled": false },
  "devices": [
    { "id": "$peer_id", "addresses": ["$peer_addr"] }
  ],
  "folders": [
    { "id": "stress", "path": "$sync_path", "devices": [{"id": "$peer_id"}] }
  ]
}
CONFIG
}

# 获取 device ID
get_device_id() {
    local dir=$1
    openssl x509 -in "$dir/cert.pem" -noout -pubkey 2>/dev/null | \
        openssl pkey -pubin -outform DER 2>/dev/null | \
        sha256sum | cut -c1-64
}

echo "[2/5] Setting up nodes..."
generate_cert "$NODE_A_DIR" "node-a"
generate_cert "$NODE_B_DIR" "node-b"

DEVICE_A="$(get_device_id "$NODE_A_DIR")"
DEVICE_B="$(get_device_id "$NODE_B_DIR")"

# Determine bind/listen addresses
if [[ -n "$PEER_ADDR" ]]; then
    LISTEN_A="0.0.0.0:22000"
    LISTEN_B="0.0.0.0:22001"
    # Node B's config points to the external peer; Node A accepts any incoming.
    create_config "$NODE_A_DIR" 22000 "$SYNC_DIR/a" "$DEVICE_B" "tcp://$PEER_ADDR"
    create_config "$NODE_B_DIR" 22001 "$SYNC_DIR/b" "$DEVICE_A" "tcp://$PEER_ADDR"
else
    LISTEN_A="127.0.0.1:22000"
    LISTEN_B="127.0.0.1:22001"
    create_config "$NODE_A_DIR" 22000 "$SYNC_DIR/a" "$DEVICE_B"
    create_config "$NODE_B_DIR" 22001 "$SYNC_DIR/b" "$DEVICE_A"
fi

echo "Device A: $DEVICE_A"
echo "Device B: $DEVICE_B"

# 启动节点
echo "[3/5] Starting daemons..."
nohup "$BINARY" daemon --config-dir "$NODE_A_DIR" --listen "$LISTEN_A" > "$LOG_DIR/node_a.log" 2>&1 &
echo $! > "$LOG_DIR/node_a.pid"

nohup "$BINARY" daemon --config-dir "$NODE_B_DIR" --listen "$LISTEN_B" > "$LOG_DIR/node_b.log" 2>&1 &
echo $! > "$LOG_DIR/node_b.pid"

sleep 5

# 启动文件变更生成器
echo "[4/5] Starting file churn generator..."
nohup bash "$SCRIPT_DIR/72h_churn.sh" "$SYNC_DIR/a" > "$LOG_DIR/churn.log" 2>&1 &
echo $! > "$LOG_DIR/churn.pid"

# 启动监控 (优先使用跨平台 Rust monitor，回退到 bash 脚本)
echo "[5/5] Starting monitor..."
MONITOR_BINARY="$REPO_ROOT/target/release/syncthing-monitor"
if [[ -x "$MONITOR_BINARY" ]]; then
    nohup "$MONITOR_BINARY" \
        --proc "$(cat "$LOG_DIR/node_a.pid")" --proc "$(cat "$LOG_DIR/node_b.pid")" \
        --log "$LOG_DIR/node_a.log" --log "$LOG_DIR/node_b.log" \
        --sync-dir "$SYNC_DIR/a" --sync-dir "$SYNC_DIR/b" \
        --interval 60s --output "$LOG_DIR/metrics/timeseries.csv" \
        --alerts "$LOG_DIR/metrics/alerts.log" \
        > "$LOG_DIR/monitor.log" 2>&1 &
else
    echo "Warning: syncthing-monitor not found at $MONITOR_BINARY, falling back to bash monitor"
    nohup bash "$SCRIPT_DIR/72h_monitor.sh" \
        "$(cat "$LOG_DIR/node_a.pid")" "$(cat "$LOG_DIR/node_b.pid")" \
        "$LOG_DIR" "$SYNC_DIR" > "$LOG_DIR/monitor.log" 2>&1 &
fi
echo $! > "$LOG_DIR/monitor.pid"

echo ""
echo "=== 72h test started ==="
echo "Logs: $LOG_DIR"
echo "Monitor: tail -f $LOG_DIR/monitor.log"
echo "Stop: kill \$(cat $LOG_DIR/node_a.pid) \$(cat $LOG_DIR/node_b.pid)"
echo ""
