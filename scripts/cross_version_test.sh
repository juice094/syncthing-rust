#!/usr/bin/env bash
# 跨版本互通测试：syncthing-rust vs syncthing-go
# 验证 BEP 协议兼容性、文件同步完整性
#
# 关键经验（2026-05-14）：
# 1. Go syncthing 只接受 Base32+Luhn-32 格式 Device ID（56 字符）
# 2. Rust config.json 必须包含 Folder/Device 完整必填字段，否则 serde 反序列化失败回退到空配置
# 3. Go syncthing v2.x CLI 使用子命令：generate / serve / device-id
# 4. Windows 版 Go syncthing 是 GUI 子系统，需用 --log-file= 捕获日志

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TEST_DIR="${1:-/tmp/syncthing-cross-version}"
GO_SYNCTHING_VERSION="${2:-v2.1.0}"

RUST_DIR="$TEST_DIR/rust"
GO_DIR="$TEST_DIR/go"
SYNC_RUST="$TEST_DIR/sync_rust"
SYNC_GO="$TEST_DIR/sync_go"
LOG_DIR="$TEST_DIR/logs"

mkdir -p "$RUST_DIR" "$GO_DIR" "$SYNC_RUST" "$SYNC_GO" "$LOG_DIR"

PLATFORM="linux"
GO_BIN_NAME="syncthing"
RUST_BIN_NAME="syncthing"
PYTHON_CMD="python3"
if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "cygwin" || "$OSTYPE" == "win32" ]]; then
    PLATFORM="windows"
    GO_BIN_NAME="syncthing.exe"
    RUST_BIN_NAME="syncthing.exe"
    PYTHON_CMD="python"
fi

echo "=== Cross-Version Interop Test ==="
echo "Platform: $PLATFORM"
echo "Test dir: $TEST_DIR"
echo "Go syncthing version: $GO_SYNCTHING_VERSION"
echo ""

# 1. 获取 Go syncthing
GO_BIN="$GO_DIR/$GO_BIN_NAME"
if [[ ! -f "$GO_BIN" ]]; then
    echo "[1/7] Downloading Go syncthing $GO_SYNCTHING_VERSION..."
    if [[ "$PLATFORM" == "windows" ]]; then
        GO_URL="https://github.com/syncthing/syncthing/releases/download/${GO_SYNCTHING_VERSION}/syncthing-windows-amd64-${GO_SYNCTHING_VERSION}.zip"
        curl -sL "$GO_URL" -o "$GO_DIR/syncthing.zip"
        unzip -q "$GO_DIR/syncthing.zip" -d "$GO_DIR"
        mv "$GO_DIR/syncthing-windows-amd64-${GO_SYNCTHING_VERSION}/syncthing.exe" "$GO_BIN" 2>/dev/null || true
    else
        GO_URL="https://github.com/syncthing/syncthing/releases/download/${GO_SYNCTHING_VERSION}/syncthing-linux-amd64-${GO_SYNCTHING_VERSION}.tar.gz"
        curl -sL "$GO_URL" | tar -xzf - -C "$GO_DIR" --strip-components=1
    fi
fi
if [[ ! -f "$GO_BIN" ]]; then
    echo "ERROR: Go syncthing binary not found at $GO_BIN"
    exit 1
fi
"$GO_BIN" --version 2>/dev/null || true

# 2. 编译 Rust syncthing
echo "[2/7] Building Rust syncthing (release)..."
cd "$REPO_ROOT"
cargo build --release --bin syncthing 2>&1 | tail -3
RUST_BIN="$REPO_ROOT/target/release/$RUST_BIN_NAME"
if [[ ! -f "$RUST_BIN" ]]; then
    echo "ERROR: Rust syncthing binary not found at $RUST_BIN"
    exit 1
fi

# 3. 生成证书并提取 Device ID
echo "[3/7] Generating certificates and extracting device IDs..."

"$GO_BIN" generate --home="$GO_DIR" >/dev/null 2>&1 || "$GO_BIN" generate --config="$GO_DIR" >/dev/null 2>&1 || true

GO_DEVICE_ID=""
if [[ -f "$GO_DIR/config.xml" ]]; then
    GO_DEVICE_ID=$(grep -oP "(?<=id=')[A-Z2-7-]{56}(?=')" "$GO_DIR/config.xml" | head -1 || true)
fi
if [[ -z "$GO_DEVICE_ID" ]]; then
    GO_DEVICE_ID=$("$GO_BIN" device-id --home="$GO_DIR" 2>/dev/null | grep -oP '[A-Z2-7-]{56}' | head -1 || true)
fi
if [[ -z "$GO_DEVICE_ID" ]]; then
    echo "ERROR: Failed to extract Go device ID"
    exit 1
fi
echo "  Go device ID:    $GO_DEVICE_ID"

RUST_LOG_FILE="$LOG_DIR/rust_init.log"
if [[ "$PLATFORM" == "windows" ]]; then
    powershell.exe -Command "Start-Process -FilePath '$RUST_BIN' -ArgumentList 'run','--config-dir=$RUST_DIR','--listen=127.0.0.1:22001','-d=rust-node' -RedirectStandardOutput '$RUST_LOG_FILE' -WindowStyle Hidden"
    sleep 5
    powershell.exe -Command "Stop-Process -Name syncthing -Force -ErrorAction SilentlyContinue"
else
    nohup "$RUST_BIN" run --config-dir "$RUST_DIR" --listen "127.0.0.1:22001" -d "rust-node" > "$RUST_LOG_FILE" 2>&1 &
    RUST_INIT_PID=$!
    sleep 5
    kill $RUST_INIT_PID 2>/dev/null || true
fi

RUST_DEVICE_ID=""
if [[ -f "$RUST_DIR/config.json" ]]; then
    RUST_DEVICE_ID=$($PYTHON_CMD -c "import json; print(json.load(open('$RUST_DIR/config.json')).get('local_device_id',''))" 2>/dev/null || true)
fi
if [[ -z "$RUST_DEVICE_ID" ]]; then
    echo "ERROR: Failed to extract Rust device ID"
    exit 1
fi
echo "  Rust device ID:  $RUST_DEVICE_ID"

# 4. 生成完整字段配置
echo "[4/7] Generating complete configs..."

$PYTHON_CMD << PYEOF
import json, os
cfg = {
    "version": 1,
    "listen_addr": "127.0.0.1:22001",
    "device_name": "rust-node",
    "folders": [
        {
            "id": "cross-test",
            "path": os.path.abspath("$SYNC_RUST").replace("/", os.sep),
            "label": None,
            "folder_type": "SendReceive",
            "paused": False,
            "rescan_interval_secs": 3600,
            "devices": ["$RUST_DEVICE_ID", "$GO_DEVICE_ID"],
            "ignore_patterns": [],
            "versioning": None
        }
    ],
    "devices": [
        {
            "id": "$GO_DEVICE_ID",
            "name": "go-node",
            "addresses": [{"Tcp": "tcp://127.0.0.1:22000"}],
            "paused": False,
            "introducer": False
        }
    ],
    "local_device_id": "$RUST_DEVICE_ID",
    "gui": {"enabled": True, "address": "127.0.0.1:8385", "api_key": ""},
    "options": {
        "listen_addresses": [],
        "global_announce_enabled": False,
        "local_announce_enabled": False,
        "relays_enabled": False
    }
}
with open("$RUST_DIR/config.json", "w") as f:
    json.dump(cfg, f, indent=2)
print("Rust config written")
PYEOF

$PYTHON_CMD << PYEOF
import xml.etree.ElementTree as ET
import os
cfg = ET.Element("configuration", version="52")
dev_go = ET.SubElement(cfg, "device", id="$GO_DEVICE_ID", name="go-node")
ET.SubElement(dev_go, "address").text = "dynamic"
dev_rust = ET.SubElement(cfg, "device", id="$RUST_DEVICE_ID", name="rust-node")
ET.SubElement(dev_rust, "address").text = "tcp://127.0.0.1:22001"
folder = ET.SubElement(cfg, "folder", id="cross-test", path=os.path.abspath("$SYNC_GO").replace("/", os.sep), type="sendreceive")
ET.SubElement(folder, "device", id="$GO_DEVICE_ID")
ET.SubElement(folder, "device", id="$RUST_DEVICE_ID")
gui = ET.SubElement(cfg, "gui", enabled="true", tls="false")
ET.SubElement(gui, "address").text = "127.0.0.1:8384"
ET.SubElement(gui, "apikey").text = "cross-version-test-key"
opts = ET.SubElement(cfg, "options")
ET.SubElement(opts, "listenAddress").text = "tcp://127.0.0.1:22000"
ET.SubElement(opts, "globalAnnounceEnabled").text = "false"
ET.SubElement(opts, "localAnnounceEnabled").text = "false"
ET.SubElement(opts, "relaysEnabled").text = "false"
next_child = None
def indent(elem, level=0):
    i = "\n" + level*"    "
    if len(elem):
        if not elem.text or not elem.text.strip():
            elem.text = i + "    "
        if not elem.tail or not elem.tail.strip():
            elem.tail = i
        for child in elem:
            indent(child, level+1)
        next_child = child
        if not child.tail or not child.tail.strip():
            child.tail = i
    else:
        if level and (not elem.tail or not elem.tail.strip()):
            elem.tail = i
indent(cfg)
ET.ElementTree(cfg).write("$GO_DIR/config.xml", encoding="utf-8", xml_declaration=False)
print("Go config written")
PYEOF

# 5. 启动节点
echo "[5/7] Starting nodes..."
if [[ "$PLATFORM" == "windows" ]]; then
    powershell.exe -Command "Start-Process -FilePath '$RUST_BIN' -ArgumentList 'run','--config-dir=$RUST_DIR','--listen=127.0.0.1:22001','-d=rust-node' -RedirectStandardOutput '$LOG_DIR/rust.log' -RedirectStandardError '$LOG_DIR/rust.err' -WindowStyle Hidden"
    powershell.exe -Command "Start-Process -FilePath '$GO_BIN' -ArgumentList 'serve','--home=$GO_DIR','--no-browser','--no-restart','--log-file=$LOG_DIR/go.log' -WindowStyle Hidden"
else
    nohup "$RUST_BIN" run --config-dir "$RUST_DIR" --listen "127.0.0.1:22001" -d "rust-node" > "$LOG_DIR/rust.log" 2>&1 &
    echo $! > "$LOG_DIR/rust.pid"
    nohup "$GO_BIN" serve --home="$GO_DIR" --no-browser --no-restart > "$LOG_DIR/go.log" 2>&1 &
    echo $! > "$LOG_DIR/go.pid"
fi

sleep 10

# 6. 生成测试文件
echo "[6/7] Generating test files..."
$PYTHON_CMD -c "import os; open('$SYNC_RUST/test_1mb.bin','wb').write(os.urandom(1024*1024))"
$PYTHON_CMD -c "import os; open('$SYNC_RUST/test_10mb.bin','wb').write(os.urandom(10*1024*1024))"
mkdir -p "$SYNC_RUST/nested"
$PYTHON_CMD -c "import os; open('$SYNC_RUST/nested/deep.bin','wb').write(os.urandom(100*1024))"
echo "Hello from cross-version interop test" > "$SYNC_RUST/hello.txt"
echo "Test files created in $SYNC_RUST"

# 7. 等待同步
echo "[7/7] Waiting for sync (max 180s)..."
SYNC_OK=0
for i in $(seq 1 180); do
    if [[ ! -d "$SYNC_GO" ]]; then
        sleep 1; continue
    fi
    ALL_MATCH=1
    while IFS= read -r -d '' f; do
        rel="${f#$SYNC_RUST/}"
        go_file="$SYNC_GO/$rel"
        if [[ ! -f "$go_file" ]]; then
            ALL_MATCH=0; break
        fi
        rust_size=$(stat -c%s "$f" 2>/dev/null || stat -f%z "$f" 2>/dev/null || wc -c < "$f")
        go_size=$(stat -c%s "$go_file" 2>/dev/null || stat -f%z "$go_file" 2>/dev/null || wc -c < "$go_file")
        if [[ "$rust_size" != "$go_size" ]]; then
            ALL_MATCH=0; break
        fi
    done < <(find "$SYNC_RUST" -type f -print0 2>/dev/null)

    if [[ "$ALL_MATCH" == "1" ]]; then
        SYNC_OK=1
        echo ""
        echo "=== SYNC SUCCESS ==="
        echo "Files synced in ${i}s"
        echo "Rust files: $(find $SYNC_RUST -type f 2>/dev/null | wc -l)"
        echo "Go files:   $(find $SYNC_GO -type f 2>/dev/null | wc -l)"
        echo ""
        echo "Content verification:"
        cat "$SYNC_GO/hello.txt"
        break
    fi
    if [[ $i -eq 180 ]]; then
        echo ""
        echo "=== SYNC TIMEOUT ==="
        echo "Rust files: $(find $SYNC_RUST -type f 2>/dev/null | wc -l)"
        echo "Go files:   $(find $SYNC_GO -type f 2>/dev/null | wc -l)"
        echo ""
        echo "Rust log tail:"
        tail -30 "$LOG_DIR/rust.log" 2>/dev/null || true
        echo ""
        echo "Go log tail:"
        tail -30 "$LOG_DIR/go.log" 2>/dev/null || true
        exit 1
    fi
    sleep 1
done

# 清理
echo ""
echo "Cleaning up..."
if [[ "$PLATFORM" == "windows" ]]; then
    powershell.exe -Command "Stop-Process -Name syncthing -Force -ErrorAction SilentlyContinue"
else
    kill $(cat "$LOG_DIR/rust.pid" 2>/dev/null) 2>/dev/null || true
    kill $(cat "$LOG_DIR/go.pid" 2>/dev/null) 2>/dev/null || true
fi

echo ""
echo "=== Cross-Version Interop Test Complete ==="
