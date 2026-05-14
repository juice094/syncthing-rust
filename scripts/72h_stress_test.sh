#!/usr/bin/env bash
# 72h 压力测试部署脚本
# 运行方式: ./scripts/72h_stress_test.sh [node_a_dir] [node_b_dir] [sync_folder]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

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
    cat > "$dir/config.json" <> CONFIG
{
  "device_name": "$(basename $dir)",
  "listen_addr": "127.0.0.1:$port",
  "gui": { "enabled": false },
  "options": { "relays_enabled": false },
  "devices": [
    { "id": "$peer_id", "addresses": ["tcp://127.0.0.1:$((port == 22000 ? 22001 : 22000))"] }
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

create_config "$NODE_A_DIR" 22000 "$SYNC_DIR/a" "$DEVICE_B"
create_config "$NODE_B_DIR" 22001 "$SYNC_DIR/b" "$DEVICE_A"

echo "Device A: $DEVICE_A"
echo "Device B: $DEVICE_B"

# 启动节点
echo "[3/5] Starting daemons..."
nohup "$BINARY" daemon --config-dir "$NODE_A_DIR" --listen "127.0.0.1:22000" > "$LOG_DIR/node_a.log" 2>&1 &
echo $! > "$LOG_DIR/node_a.pid"

nohup "$BINARY" daemon --config-dir "$NODE_B_DIR" --listen "127.0.0.1:22001" > "$LOG_DIR/node_b.log" 2>&1 &
echo $! > "$LOG_DIR/node_b.pid"

sleep 5

# 启动文件变更生成器
echo "[4/5] Starting file churn generator..."
nohup bash "$SCRIPT_DIR/72h_churn.sh" "$SYNC_DIR/a" > "$LOG_DIR/churn.log" 2>&1 &
echo $! > "$LOG_DIR/churn.pid"

# 启动监控
echo "[5/5] Starting monitor..."
nohup bash "$SCRIPT_DIR/72h_monitor.sh" \
    "$LOG_DIR/node_a.pid" "$LOG_DIR/node_b.pid" \
    "$LOG_DIR" "$SYNC_DIR" > "$LOG_DIR/monitor.log" 2>&1 &
echo $! > "$LOG_DIR/monitor.pid"

echo ""
echo "=== 72h test started ==="
echo "Logs: $LOG_DIR"
echo "Monitor: tail -f $LOG_DIR/monitor.log"
echo "Stop: kill \$(cat $LOG_DIR/node_a.pid) \$(cat $LOG_DIR/node_b.pid)"
echo ""
