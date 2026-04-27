# 格雷侧操作指南 · syncthing-rust 互通验证

> **制定日期**: 2026-04-27
> **对侧环境**: 40GB 磁盘虚拟机，无 Rust 工具链
> **工作流**: 本侧编译 → GitHub Release → 格雷侧下载运行
> **本机版本**: `main` @ `785bd5c`
> **通信方式**: 校园网内网 / 公网直连 / Relay 中继（Tailscale 当前不可用）

---

## 零、⚠️ 磁盘安全警告（执行前必读）

**格雷侧总磁盘仅 40GB**，syncthing-rust 代码库中有 **430 个日志调用**分布在 50 个文件中。长期运行（72h stress test）时：

| 日志级别 | 72h 估算 | 风险 |
|----------|----------|------|
| `info`（默认） | ~7 MB | ✅ 安全 |
| `debug` | 72-360 MB | ⚠️ 谨慎 |
| `trace` | >720 MB | ❌ 禁止用于长期测试 |

**sled 数据库膨胀风险**：若运行完整 daemon 并同步大量文件，数据库可能膨胀至数 GB（LSM-tree 写放大）。

**格雷侧启动时必须显式指定 `--log-level info`**。

**磁盘预算（40GB 总空间）**：
- 系统预留：~15 GB
- 可用测试空间：**~20 GB**
- syncthing-rust 数据目录上限：建议 **<5 GB**

---

## 一、工作流：预编译二进制（格雷侧无需 Rust 工具链）

```
┌──────────────┐     compile      ┌──────────────┐     download      ┌──────────────┐
│ juice094-PC  │ ───────────────► │ GitHub main  │ ───────────────► │ 格雷侧虚拟机  │
│ (Windows)    │   cargo build    │ (Release)    │   wget/curl      │ (Linux)       │
└──────────────┘                  └──────────────┘                  └──────────────┘
```

### 1.1 本侧编译并上传

```powershell
# 1. 确认版本
cd C:\path\to\syncthing-rust
git rev-parse --short HEAD  # 应输出当前 main hash

# 2. 编译 Linux x86_64 版本（格雷侧是 Linux 虚拟机）
# 需要安装 cross 或直接在 WSL 中编译
# 方式 A: WSL
cd /mnt/c/path/to/syncthing-rust
cargo build --release -p syncthing

# 方式 B: cross (若已安装)
cross build --release -p syncthing --target x86_64-unknown-linux-gnu

# 3. 上传 Release
# 手动上传到 GitHub Releases 或推送到服务器
```

### 1.2 格雷侧下载运行

```bash
# 创建独立测试目录（必须限制大小）
mkdir -p ~/syncthing-test
cd ~/syncthing-test

# 下载预编译二进制（替换为实际 URL）
wget https://github.com/juice094/syncthing-rust/releases/download/v0.2.0/syncthing-x86_64-linux
chmod +x syncthing-x86_64-linux

# 创建数据目录（限制在此目录内，便于整体清理）
mkdir -p ~/syncthing-test/data
```

---

## 二、方案 A: 同版本 Rust ↔ Rust 互通验证（P0）

### A1. 格雷侧启动配置

```bash
# ⚠️ 必须指定 --log-level info 防止磁盘耗尽
# ⚠️ 必须指定 --data-dir 限制数据目录位置

cd ~/syncthing-test
./syncthing-x86_64-linux run \
    --config ~/syncthing-test/config.json \
    --data-dir ~/syncthing-test/data \
    --gui-address 127.0.0.1:8385 \
    --log-level info
```

**配置要求**（`config.json` 示例）—— 根据网络路径二选一：

**路径一：公网直连（格雷侧有公网 IP）**
```json
{
  "options": {
    "global_announce_enabled": false,
    "relays_enabled": false,
    "listen_addr": "0.0.0.0:22000"
  },
  "devices": [
    {
      "deviceID": "XQVFE6J-4JCJRXW-4PSMU25-ZKZ3AKB-52XN6KZ-W6TRN5Y-4PH45KZ-XK4V3A6",
      "name": "juice094-local",
      "addresses": ["tcp://10.3.155.142:22001"]
    }
  ],
  "folders": [
    {
      "id": "test-folder",
      "path": "/home/gray/syncthing-test/sync-folder",
      "devices": [{"deviceID": "XQVFE6J-4JCJRXW-4PSMU25-ZKZ3AKB-52XN6KZ-W6TRN5Y-4PH45KZ-XK4V3A6"}]
    }
  ]
}
```

**路径二：Relay 中继（格雷侧无公网 IP）**
```json
{
  "options": {
    "global_announce_enabled": true,
    "relays_enabled": true,
    "listen_addr": "0.0.0.0:22000"
  },
  "devices": [
    {
      "deviceID": "XQVFE6J-4JCJRXW-4PSMU25-ZKZ3AKB-52XN6KZ-W6TRN5Y-4PH45KZ-XK4V3A6",
      "name": "juice094-local",
      "addresses": ["dynamic"]
    }
  ],
  "folders": [
    {
      "id": "test-folder",
      "path": "/home/gray/syncthing-test/sync-folder",
      "devices": [{"deviceID": "XQVFE6J-4JCJRXW-4PSMU25-ZKZ3AKB-52XN6KZ-W6TRN5Y-4PH45KZ-XK4V3A6"}]
    }
  ]
}
```

### A2. 格雷侧基础网络自查（启动前执行）

```bash
# 1. 确认 22000 端口在监听
ss -tlnp | grep 22000
# 预期输出: 0.0.0.0:22000 处于 LISTEN 状态

# 2. 检查磁盘空间（启动前必须确认）
df -h ~
# 确保 ~ 分区可用空间 > 5GB

# 3. 防火墙自查
sudo iptables -L -n | grep 22000
# 若为空，临时放行:
# sudo iptables -I INPUT -p tcp --dport 22000 -j ACCEPT
```

### A3. 格雷侧日志收集（限制大小）

**方式一：systemd journal（推荐，自动轮转压缩）**

```bash
sudo tee /etc/systemd/system/syncthing-rust.service << 'EOF'
[Unit]
Description=Syncthing Rust
After=network.target

[Service]
Type=simple
User=gray
WorkingDirectory=/home/gray/syncthing-test
ExecStart=/home/gray/syncthing-test/syncthing-x86_64-linux run \
    --config /home/gray/syncthing-test/config.json \
    --data-dir /home/gray/syncthing-test/data \
    --log-level info
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal
SyslogIdentifier=syncthing-rust

# 磁盘保护：限制数据目录大小
ExecStartPre=/bin/bash -c 'du -sh /home/gray/syncthing-test/data | awk "{print \$1}"'

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable syncthing-rust
sudo systemctl start syncthing-rust
# 查看日志: sudo journalctl -u syncthing-rust -f
```

**方式二：裸文件（不推荐，必须限制大小）**
```bash
# 仅 info 级别，禁止 debug/trace
RUST_LOG=info ./syncthing-x86_64-linux run --log-level info > log.txt 2>&1 &
# 启动后每小时检查文件大小，超过 50MB 立即轮转
```

日志中需要关注的关键行：
- `Connection manager started on ...` — 监听成功
- `Parallel dialing ...` — 开始拨号
- `Client TLS handshake completed` / `Server TLS handshake completed` — TLS 成功
- `BEP connection established` — BEP 连接建立
- `Connection registered for device` — 连接注册成功
- `Heartbeat timeout` — 若频繁出现，说明连接不稳定
- `Scheduling reconnect` — 重连行为

---

## 三、方案 B: Go Syncthing ↔ 新版 Rust 互通验证

### B1. 格雷侧启动 Go Syncthing

```bash
# 下载官方 Go Syncthing（若未安装）
# https://github.com/syncthing/syncthing/releases

cd ~/syncthing-test
./syncthing -home=/home/gray/syncthing-test/go-config -no-browser
```

### B2. 格雷侧配置 Go Syncthing

通过 Web UI (`http://127.0.0.1:8384`) 或 API 配置：
1. 添加远程设备（本机 DeviceID）
2. 添加共享文件夹
3. **关键**: 在 Settings → Connections 中确认：
   - `Sync Protocol Listen Addresses`: `tcp://0.0.0.0:22000`
   - `Global Discovery`: ON（若走 Relay）/ OFF（若公网直连）
   - `Local Discovery`: ON

### B3. 格雷侧提供信息给本机

```bash
# 获取 Go Syncthing 的 Device ID
curl -H "X-API-Key: <api-key>" http://127.0.0.1:8384/rest/system/status | jq .myID

# 获取监听地址
curl -H "X-API-Key: <api-key>" http://127.0.0.1:8384/rest/system/status | jq .listeners
```

---

## 四、方案 C: 旧版 Rust ↔ 新版 Rust 兼容性验证（仅当需要时）

若方案 A 成功、方案 B 成功，但旧版 Rust 仍然失败，则问题锁定为**版本兼容性 bug**。

### C1. 格雷侧保留旧版运行

保持当前 pre-fix 构建不变，仅收集日志（同样限制级别）：

```bash
RUST_LOG=info ./syncthing-old run --log-level info --data-dir ~/syncthing-test/old-data
```

### C2. 需要格雷侧提供的旧版信息

```bash
# 1. 旧版 commit hash（若已知）
cd <旧版源码目录>
git rev-parse --short HEAD

# 2. 旧版 Cargo.lock 关键依赖版本
grep -A2 '^name = "syncthing-net"' Cargo.lock
grep -A2 '^name = "bep-protocol"' Cargo.lock
grep -A2 '^name = "rustls"' Cargo.lock

# 3. 旧版启动时的完整日志（前 100 行 + 连接失败部分）
```

---

## 五、数据目录监控与清理脚本

**格雷侧必须部署磁盘监控**（40GB 极易耗尽）：

```bash
#!/bin/bash
# ~/syncthing-test/disk-watch.sh
# 每 10 分钟检查一次数据目录大小

DATA_DIR="/home/gray/syncthing-test/data"
LOG_DIR="/home/gray/syncthing-test"
LIMIT_MB=5120  # 5GB 上限

while true; do
    SIZE_MB=$(du -sm "$DATA_DIR" 2>/dev/null | awk '{print $1}')
    echo "$(date): data dir = ${SIZE_MB}MB"

    if [ "$SIZE_MB" -gt "$LIMIT_MB" ]; then
        echo "$(date): WARNING data dir ${SIZE_MB}MB > ${LIMIT_MB}MB limit!"
        echo "$(date): Consider stopping syncthing and cleaning data dir."
        # 可选：自动清理
        # systemctl stop syncthing-rust
        # rm -rf "$DATA_DIR"/*
    fi

    # 同时检查日志大小
    if [ -f "$LOG_DIR/log.txt" ]; then
        LOG_MB=$(du -sm "$LOG_DIR/log.txt" | awk '{print $1}')
        if [ "$LOG_MB" -gt 100 ]; then
            echo "$(date): Rotating log (${LOG_MB}MB)"
            mv "$LOG_DIR/log.txt" "$LOG_DIR/log.txt.$(date +%s)"
        fi
    fi

    sleep 600
done
```

**启动监控**：
```bash
nohup ~/syncthing-test/disk-watch.sh > ~/syncthing-test/disk-watch.log 2>&1 &
```

---

## 六、信息交换清单

本机 ↔ 格雷侧需要交换的信息：

| 信息 | 提供方 | 用途 |
|------|--------|------|
| Device ID | 双方互相提供 | 配置对方为可信设备 |
| 公网 IP / 可达地址 | 格雷侧确认 | 连接地址（若公网直连） |
| 监听端口 | 双方确认 | Rust 默认 22000，本机使用 22001 |
| API 端口 | 双方确认 | Rust 默认 8385，Go 默认 8384 |
| 网络路径 | 双方确认 | 公网直连 or Relay |
| 日志文件 | 格雷侧 | 排查握手/连接失败原因 |
| 防火墙状态 | 格雷侧 | 排除网络层阻塞 |

---

## 七、快速决策树

```
本侧编译 → GitHub 上传
        │
        ▼
格雷侧下载 → 配置 config.json
        │
        ▼
格雷侧有公网 IP 且端口放行？
        │
    ┌───┴───┐
    ▼       ▼
   是      否
    │       │
    ▼       ▼
 公网直连  启用 Relay
    │       │
    ▼       ▼
检查磁盘空间 (>5GB 可用?)
        │
    ┌───┴───┐
    ▼       ▼
   是      否
    │       │
    ▼       ▼
 启动测试  清理旧数据
    │
    ▼
Test-NetConnection 或 Relay 握手
        │
    ┌───┴───┐
    ▼       ▼
  成功    失败
    │       │
    ▼       ▼
 本机启动  检查防火墙/
 最新版    Discovery 配置
    │       │
    ▼       ▼
 双向连接?  修复网络
    │
┌───┴───┐
▼       ▼
成功   失败
 │      │
 ▼      ▼
方案A   收集日志
通过    分析原因
        │
        ▼
   执行方案B/C
```

---

## 八、紧急清理（磁盘再次耗尽时）

```bash
# 停止服务
sudo systemctl stop syncthing-rust

# 清理数据（保留配置）
rm -rf ~/syncthing-test/data/*
rm -f ~/syncthing-test/log.txt*

# 验证磁盘
 df -h ~

# 重启
sudo systemctl start syncthing-rust
```

---

*本指南随 `main` 更新而更新。最新版本见仓库 `docs/plans/GRAY_SIDE_OPS.md`。*
