#!/usr/bin/env bash
# 跨版本互通测试：syncthing-rust (v0.2.6) vs syncthing-go (latest)
# 验证 BEP 协议兼容性、文件同步完整性

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TEST_DIR="${1:-/tmp/syncthing-cross-version}"
GO_SYNCTHING_VERSION="${2:-v1.27.0}"

RUST_DIR="$TEST_DIR/rust"
GO_DIR="$TEST_DIR/go"
SYNC_RUST="$TEST_DIR/sync_rust"
SYNC_GO="$TEST_DIR/sync_go"
LOG_DIR="$TEST_DIR/logs"

mkdir -p "$RUST_DIR" "$GO_DIR" "$SYNC_RUST" "$SYNC_GO" "$LOG_DIR"

echo "=== Cross-Version Interop Test ==="
echo "Rust node: $RUST_DIR"
echo "Go node: $GO_DIR"
echo "Go syncthing version: $GO_SYNCTHING_VERSION"
echo ""

# 1. 下载 Go syncthing
echo "[1/6] Downloading Go syncthing $GO_SYNCTHING_VERSION..."
GO_BIN="$GO_DIR/syncthing"
if [[ ! -f "$GO_BIN" ]]; then
    GO_URL="https://github.com/syncthing/syncthing/releases/download/${GO_SYNCTHING_VERSION}/syncthing-linux-amd64-${GO_SYNCTHING_VERSION}.tar.gz"
    curl -sL "$GO_URL" | tar -xzf - -C "$GO_DIR" --strip-components=1
fi
"$GO_BIN" --version

# 2. 编译 Rust syncthing
echo "[2/6] Building Rust syncthing (release)..."
cd "$REPO_ROOT"
cargo build --release --bin syncthing 2>&1 | tail -3
RUST_BIN="$REPO_ROOT/target/release/syncthing"

# 3. 生成配置
echo "[3/6] Generating configs..."

# Rust 节点配置
cat > "$RUST_DIR/config.json" <> RUST_CFG
{
  "device_name": "rust-node",
  "listen_addr": "127.0.0.1:22000",
  "gui": { "enabled": false },
  "options": { "relays_enabled": false, "global_discovery_enabled": false, "local_discovery_enabled": false }
}
RUST_CFG

# Go 节点配置（最小化）
cat > "$GO_DIR/config.xml" <> GO_CFG
<configuration version="37">
  <folder id="interop" path="$SYNC_GO" type="sendreceive">
    <device id="RUST-DEVICE-ID-PLACEHOLDER"/>
  </folder>
  <device id="RUST-DEVICE-ID-PLACEHOLDER" name="rust-node" compression="metadata">
    <address>127.0.0.1:22000</address>
  </device>
  <gui enabled="false"></gui>
  <options>
    <listenAddress>127.0.0.1:22001</listenAddress>
    <globalAnnounceEnabled>false</globalAnnounceEnabled>
    <localAnnounceEnabled>false</localAnnounceEnabled>
  </options>
</configuration>
GO_CFG

# 4. 启动节点
echo "[4/6] Starting nodes..."
nohup "$RUST_BIN" daemon --config-dir "$RUST_DIR" --listen "127.0.0.1:22000" > "$LOG_DIR/rust.log" 2>&1 &
echo $! > "$LOG_DIR/rust.pid"

# Go syncthing 使用 home 目录配置
export HOME="$GO_DIR"
nohup "$GO_BIN" serve --no-browser --gui-address="" > "$LOG_DIR/go.log" 2>&1 &
echo $! > "$LOG_DIR/go.pid"

sleep 10

# 5. 生成测试文件
echo "[5/6] Generating test files..."
dd if=/dev/urandom of="$SYNC_RUST/test_1mb.bin" bs=1M count=1 2>/dev/null
dd if=/dev/urandom of="$SYNC_RUST/test_10mb.bin" bs=1M count=10 2>/dev/null
mkdir -p "$SYNC_RUST/nested"
dd if=/dev/urandom of="$SYNC_RUST/nested/deep.bin" bs=1K count=100 2>/dev/null

echo "Test files created in $SYNC_RUST"

# 6. 等待同步
echo "[6/6] Waiting for sync (max 120s)..."
for i in $(seq 1 120); do
    rust_hash=$(find "$SYNC_RUST" -type f -exec md5sum {} + | sort | md5sum | awk '{print $1}')
    go_hash=$(find "$SYNC_GO" -type f -exec md5sum {} + | sort | md5sum | awk '{print $1}')
    if [[ "$rust_hash" == "$go_hash" && -n "$rust_hash" ]]; then
        echo ""
        echo "=== SYNC SUCCESS ✅ ==="
        echo "Files synced in ${i}s"
        echo "Rust files: $(find $SYNC_RUST -type f | wc -l)"
        echo "Go files: $(find $SYNC_GO -type f | wc -l)"
        break
    fi
    if [[ $i -eq 120 ]]; then
        echo ""
        echo "=== SYNC TIMEOUT ❌ ==="
        echo "Rust files: $(find $SYNC_RUST -type f | wc -l)"
        echo "Go files: $(find $SYNC_GO -type f | wc -l)"
        echo ""
        echo "Rust log tail:"
        tail -20 "$LOG_DIR/rust.log"
        echo ""
        echo "Go log tail:"
        tail -20 "$LOG_DIR/go.log"
        exit 1
    fi
    sleep 1
done

# 清理
echo ""
echo "Cleaning up..."
kill $(cat "$LOG_DIR/rust.pid") 2>/dev/null || true
kill $(cat "$LOG_DIR/go.pid") 2>/dev/null || true

echo ""
echo "=== Cross-Version Interop Test Complete ==="
