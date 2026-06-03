#!/bin/bash
# cloud-deploy.sh — syncthing-rust 云端部署 Skill
# 用途: 从 ROG-X (Windows) 编译最新源码并部署到 Gray-Cloud (Ubuntu)
# 使用: bash scripts/cloud-deploy.sh [--compile-only|--deploy-only|--full]
#
# 环境要求:
#   ROG-X: bash, tar, scp (via Tailscale 100.73.228.59)
#   Cloud: ssh root@100.127.13.26 (Tailscale), cargo
#
# 最后更新: 2026-06-03

set -euo pipefail

CLOUD_HOST="${CLOUD_HOST:-root@100.127.13.26}"
CLOUD_CONFIG_DIR="${CLOUD_CONFIG_DIR:-/root/syncthing-persistent}"
CLOUD_DEV_DIR="${CLOUD_DEV_DIR:-/root/dev}"
CLOUD_BINARY_NAME="${CLOUD_BINARY_NAME:-syncthing-v0.2.10-rc2}"
ROGX_PROJECT_DIR="${ROGX_PROJECT_DIR:-/c/Users/22414/dev/syncthing-rust}"
TARBALL="/tmp/syncthing-rust-src.tar.gz"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log()  { echo -e "${GREEN}[$(date +%H:%M:%S)]${NC} $*"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
err()  { echo -e "${RED}[ERROR]${NC} $*"; }

# ═══════════════════════════════════════════════════════════════
# Phase 1: ROG-X 编译
# ═══════════════════════════════════════════════════════════════
compile_rogx() {
    log "Phase 1: 编译 ROG-X 二进制"
    cd "$ROGX_PROJECT_DIR"
    cargo build --release 2>&1 | tail -3
    log "ROG-X 编译完成: $(ls -lh target/release/syncthing.exe | awk '{print $5}')"
}

# ═══════════════════════════════════════════════════════════════
# Phase 2: 打包源码传输到云端
# ═══════════════════════════════════════════════════════════════
package_and_transfer() {
    log "Phase 2: 打包源码 (排除 target/)"
    tar -czf "$TARBALL" -C "$ROGX_PROJECT_DIR" --exclude=target --exclude=.git .
    log "压缩包: $(ls -lh $TARBALL | awk '{print $5}')"

    log "Phase 2b: SCP 传输到云端"
    scp "$TARBALL" "${CLOUD_HOST}:/tmp/"
    log "传输完成"
}

# ═══════════════════════════════════════════════════════════════
# Phase 3: 云端编译
# ═══════════════════════════════════════════════════════════════
compile_cloud() {
    log "Phase 3: 云端编译"
    ssh "$CLOUD_HOST" "bash -s" << 'ENDSSH'
set -e
source ~/.cargo/env
cd /root/dev

# 清理旧源码
rm -rf crates cmd Cargo.* rust-toolchain.toml justfile 2>/dev/null || true

# 解压新源码
tar -xzf /tmp/syncthing-rust-src.tar.gz

# 编译
cargo build --release 2>&1 | tail -5
ls -lh target/release/syncthing
ENDSSH
    log "云端编译完成"
}

# ═══════════════════════════════════════════════════════════════
# Phase 4: 停止旧进程
# ═══════════════════════════════════════════════════════════════
stop_cloud() {
    log "Phase 4: 停止云端旧进程"
    ssh "$CLOUD_HOST" "bash -s" << 'ENDSSH'
# 尝试 systemctl
systemctl stop syncthing 2>/dev/null || true
# 直接杀进程
pkill -f "syncthing-v0" 2>/dev/null || true
pkill -f "watchdog.sh" 2>/dev/null || true
sleep 2
# 验证
if ps aux | grep -E 'syncthing-v0|watchdog.sh' | grep -v grep > /dev/null; then
    echo "WARNING: 仍有进程残留，尝试 kill -9"
    pkill -9 -f "syncthing-v0" 2>/dev/null || true
    sleep 1
fi
echo "旧进程已停止"
ENDSSH
}

# ═══════════════════════════════════════════════════════════════
# Phase 5: 重置数据库 (可选)
# ═══════════════════════════════════════════════════════════════
reset_cloud_db() {
    log "Phase 5: 备份并重置云端数据库"
    ssh "$CLOUD_HOST" "bash -s" << 'ENDSSH'
TS=$(date +%Y%m%d-%H%M)
# 备份旧 DB (rename, 不删除)
[ -d /root/syncthing-persistent/db ] && mv /root/syncthing-persistent/db "/root/syncthing-persistent/db.old.$TS" && echo "DB backed to db.old.$TS" || echo "No db dir"
[ -f /root/syncthing-persistent/folder_gray-workspace.json ] && mv /root/syncthing-persistent/folder_gray-workspace.json "/root/syncthing-persistent/folder_gray-workspace.json.old.$TS" && echo "folder file backed" || echo "No folder file"
ENDSSH
}

# ═══════════════════════════════════════════════════════════════
# Phase 6: 部署二进制并启动
# ═══════════════════════════════════════════════════════════════
deploy_and_start() {
    log "Phase 6: 部署二进制并启动"

    # 如果 watchdog 使用固定路径，则覆盖旧路径
    ssh "$CLOUD_HOST" "bash -s" << ENDSSH
set -e
# 先确保完全停止
systemctl stop syncthing 2>/dev/null || true
pkill -9 -f "syncthing-v0" 2>/dev/null || true
pkill -9 -f "watchdog.sh" 2>/dev/null || true
sleep 2

# 复制新版 (wheel 锁定旧名称避免 watchdog 冲突)
cp /root/dev/target/release/syncthing ${CLOUD_CONFIG_DIR}/${CLOUD_BINARY_NAME}
chmod +x ${CLOUD_CONFIG_DIR}/${CLOUD_BINARY_NAME}

# 启动新版 (不同路径避免 watchdog 干扰)
nohup ${CLOUD_CONFIG_DIR}/${CLOUD_BINARY_NAME} run --config-dir ${CLOUD_CONFIG_DIR} > /dev/null 2>&1 &
sleep 3

# 验证
if ps aux | grep "${CLOUD_BINARY_NAME}" | grep -v grep > /dev/null; then
    PID=\$(ps aux | grep "${CLOUD_BINARY_NAME}" | grep -v grep | awk '{print \$2}')
    echo "云端启动成功 PID=\$PID"
else
    echo "ERROR: 云端启动失败"
    exit 1
fi
ENDSSH
}

# ═══════════════════════════════════════════════════════════════
# Phase 7: 验证连接 (ROG-X 侧)
# ═══════════════════════════════════════════════════════════════
verify_connection() {
    log "Phase 7: ROG-X 侧验证"
    cd "$ROGX_PROJECT_DIR"

    # 清理残留 pid
    rm -f "${HOME}/.kimi_openclaw/workspace/.syncthing/syncthing.pid" 2>/dev/null || true

    # 启动 ROG-X
    ./target/release/syncthing.exe run --config-dir "${HOME}/.kimi_openclaw/workspace/.syncthing" > /dev/null 2>&1 &
    ROGX_PID=$!
    log "ROG-X 启动 PID=$ROGX_PID"

    # 等待连接
    sleep 15
    LOG_FILE=$(ls -t "${HOME}/.kimi_openclaw/workspace/.syncthing/logs/daemon."*.log | head -1)

    if grep -q "Device connected.*YGM22XN" "$LOG_FILE" 2>/dev/null; then
        log "连接成功!"
        grep "shared folders" "$LOG_FILE" | tail -1
    else
        warn "未检测到连接，检查日志: $LOG_FILE"
    fi
}

# ═══════════════════════════════════════════════════════════════
# 快捷命令
# ═══════════════════════════════════════════════════════════════
cloud_status() {
    log "云端状态检查"
    ssh "$CLOUD_HOST" "bash -s" << 'ENDSSH'
echo "=== 进程 ==="
ps aux | grep syncthing | grep -v grep || echo "无 syncthing 进程"
echo "=== watchdog ==="
ps aux | grep watchdog.sh | grep -v grep || echo "无 watchdog"
echo "=== 日志 (最后5行) ==="
LOGFILE=$(ls -t /root/syncthing-persistent/logs/daemon.*.log 2>/dev/null | head -1)
[ -n "$LOGFILE" ] && tail -5 "$LOGFILE" || echo "无日志"
echo "=== 磁盘 ==="
df -h / | tail -1
echo "=== .tmp 数量 ==="
find /root/.openclaw/workspace -name "*.tmp" 2>/dev/null | wc -l
ENDSSH
}

rogx_status() {
    log "ROG-X 状态检查"
    ps aux | grep syncthing | grep -v grep || echo "无 syncthing 进程"
    LOGFILE=$(ls -t "${HOME}/.kimi_openclaw/workspace/.syncthing/logs/daemon."*.log 2>/dev/null | head -1)
    if [ -n "$LOGFILE" ]; then
        echo "最新日志: $LOGFILE ($(wc -l < "$LOGFILE") 行)"
        echo "=== Pull 触发 ==="
        grep -c "Debounced watcher" "$LOGFILE" 2>/dev/null || echo 0
    fi
    ls "${HOME}/.kimi_openclaw/workspace/_trigger-rename.md" 2>/dev/null && echo "_trigger-rename.md: 存在" || echo "_trigger-rename.md: 不存在"
}

# ═══════════════════════════════════════════════════════════════
# 主流程
# ═══════════════════════════════════════════════════════════════
case "${1:-full}" in
    --full|full)
        compile_rogx
        package_and_transfer
        compile_cloud
        stop_cloud
        reset_cloud_db
        deploy_and_start
        verify_connection
        ;;
    --compile-only)
        compile_rogx
        package_and_transfer
        compile_cloud
        ;;
    --deploy-only)
        stop_cloud
        reset_cloud_db
        deploy_and_start
        ;;
    --status)
        cloud_status
        echo "---"
        rogx_status
        ;;
    --stop)
        log "停止双方"
        taskkill //F //IM syncthing.exe 2>/dev/null || true
        ssh "$CLOUD_HOST" "pkill -f syncthing-v0 2>/dev/null; echo done"
        ;;
    --help|-h)
        echo "用法: bash scripts/cloud-deploy.sh [--full|--compile-only|--deploy-only|--status|--stop]"
        echo ""
        echo "  --full          完整部署流程 (默认)"
        echo "  --compile-only  仅编译 (不部署不重置DB)"
        echo "  --deploy-only   仅部署 (不编译, 重置DB+启动)"
        echo "  --status        双方状态检查"
        echo "  --stop          停止双方"
        ;;
    *)
        echo "未知参数: $1"
        echo "用法: bash scripts/cloud-deploy.sh --help"
        exit 1
        ;;
esac
