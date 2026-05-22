#!/bin/bash
# recover-remote.sh — 一键恢复远程节点（在本地 Git Bash 执行）
# 前置条件：Tailscale SSH 已恢复（ssh root@100.127.13.26 可连接）

set -euo pipefail

REMOTE="root@100.127.13.26"
REMOTE_DIR="/root/syncthing-persistent"
BINARY_WIN="/c/Users/22414/dev/syncthing-rust/target/release/syncthing-linux"

echo "[$(date '+%H:%M:%S')] Checking SSH..."
ssh -o BatchMode=yes -o ConnectTimeout=5 "$REMOTE" "echo PONG" >/dev/null 2>&1 || {
    echo "ERROR: SSH to $REMOTE failed. Check Tailscale."
    exit 1
}

echo "[$(date '+%H:%M:%S')] Stopping remote daemon + watchdog..."
ssh "$REMOTE" "
    systemctl stop gray-syncthing 2>/dev/null || true
    pkill -f 'syncthing-v0.2.9-rc4 run' 2>/dev/null || true
    pkill -f 'watchdog.sh' 2>/dev/null || true
    sleep 2
    echo STOPPED
"

echo "[$(date '+%H:%M:%S')] Copying Linux binary..."
scp "$BINARY_WIN" "$REMOTE:/tmp/syncthing-new"

echo "[$(date '+%H:%M:%S')] Installing and restarting..."
ssh "$REMOTE" "
    chmod +x /tmp/syncthing-new
    cp /tmp/syncthing-new $REMOTE_DIR/syncthing-v0.2.9-rc4
    systemctl start gray-syncthing
    sleep 3
    echo '--- Daemon status ---'
    ps aux | grep syncthing-v0.2.9-rc4 | grep -v grep || echo 'NOT RUNNING'
    echo '--- Churn status ---'
    ps aux | grep remote-churn | grep -v grep || echo 'NOT RUNNING'
    echo '--- Sync dir count ---'
    ls $REMOTE_DIR/sync/ | wc -l
"

echo "[$(date '+%H:%M:%S')] Recover complete."
