# 格雷侧操作指南 · syncthing-rust 互通验证

> **制定日期**: 2026-04-27
> **对侧环境**: 可运行 Go Syncthing / Rust syncthing-rust 双版本
> **本机版本**: `main` @ `785bd5c`
> **通信方式**: 校园网内网 / 公网直连 / Relay 中继（Tailscale 当前不可用）

---

## 零、⚠️ 磁盘安全警告（执行前必读）

syncthing-rust 代码库中有 **430 个日志调用**分布在 50 个文件中。长期运行（72h stress test）时：

| 日志级别 | 72h 估算 | 风险 |
|----------|----------|------|
| `info`（默认） | ~7 MB | ✅ 安全 |
| `debug` | 72-360 MB | ⚠️ 谨慎 |
| `trace` | >720 MB | ❌ 禁止用于长期测试 |

**格雷侧启动时必须显式指定 `--log-level info`**。

如果格雷侧虚拟机磁盘已紧张（如遇 `ENOSPC`），优先排查：
1. Kimi Claw / 其他服务的日志累积
2. systemd journal 体积（`journalctl --disk-usage`）
3. Docker/container 日志

**推荐**：使用 systemd 启动（`StandardOutput=journal`），由 journald 自动压缩轮转，避免裸日志文件无限增长。

---

## 验证目标优先级

```
┌─────────────────────────────────────────────────────────────┐
│  方案 A: 同版本 Rust ↔ Rust（推荐首选）                        │
│  方案 B: Go Syncthing ↔ 新版 Rust（跨语言验证）                │
│  方案 C: 旧版 Rust ↔ 新版 Rust（仅当需要定位版本兼容性时）      │
└─────────────────────────────────────────────────────────────┘
```

**策略**: 先执行方案 A。若同版本互通成功 → 问题为旧版兼容性，需修 bug；若同版本也失败 → 问题为网络/防火墙/Discovery 配置，与代码无关。

---

## 方案 A: 同版本 Rust ↔ Rust 互通验证（P0）

### A1. 格雷侧编译最新版

```powershell
# 1. 拉取最新 main
cd C:\path\to\syncthing-rust
git fetch origin
git checkout main
git pull origin main
# 确认版本: git rev-parse --short HEAD 应输出 785bd5c 或更新

# 2. 编译 release 版本（推荐，更稳定）
cargo build --release -p syncthing

# 3. 创建测试目录
mkdir C:\syncthing-test
mkdir C:\syncthing-test\sync-folder
```

### A2. 格雷侧启动配置

```powershell
# 启动守护进程（headless，无 TUI）
# ⚠️ 必须指定 --log-level info 防止磁盘耗尽
cd C:\syncthing-test
C:\path\to\syncthing-rust\target\release\syncthing.exe run `
    --config C:\syncthing-test\config.json `
    --gui-address 127.0.0.1:8385 `
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
      "path": "C:\\syncthing-test\\sync-folder",
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
      "path": "C:\\syncthing-test\\sync-folder",
      "devices": [{"deviceID": "XQVFE6J-4JCJRXW-4PSMU25-ZKZ3AKB-52XN6KZ-W6TRN5Y-4PH45KZ-XK4V3A6"}]
    }
  ]
}
```

### A3. 格雷侧基础网络自查（启动前执行）

```powershell
# 1. 确认 22000 端口在监听
netstat -an | findstr 22000
# 预期输出: 0.0.0.0:22000 或 127.0.0.1:22000 处于 LISTENING 状态

# 2. 确认本机端口可达性（若已知本机公网 IP）
# Test-NetConnection <本机公网IP> -Port 22001

# 3. 检查 Relay 连通性（若使用路径二）
# 确认 global_announce_enabled 和 relays_enabled 均为 true

# 4. 防火墙自查
Get-NetFirewallRule | Where-Object { $_.DisplayName -like "*syncthing*" -or $_.DisplayName -like "*rust*" }
# 若为空，考虑临时放行:
# New-NetFirewallRule -DisplayName "syncthing-rust-test" -Direction Inbound -LocalPort 22000 -Protocol TCP -Action Allow
```

### A4. 格雷侧日志收集要求

**⚠️ 磁盘安全：日志重定向必须限制大小**

方式一：systemd journal（推荐，自动轮转压缩）
```powershell
# 创建 systemd service 文件（Linux 虚拟机）
sudo tee /etc/systemd/system/syncthing-rust.service << 'EOF'
[Unit]
Description=Syncthing Rust
After=network.target

[Service]
Type=simple
ExecStart=/home/gray/syncthing-rust/target/release/syncthing run --config /home/gray/syncthing-test/config.json --log-level info
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal
SyslogIdentifier=syncthing-rust

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable syncthing-rust
sudo systemctl start syncthing-rust
# 查看日志: sudo journalctl -u syncthing-rust -f
```

方式二：裸文件（不推荐，必须限制大小）
```powershell
# 仅 info 级别，禁止 debug/trace
$env:RUST_LOG = "info"
C:\path\to\syncthing-rust\target\release\syncthing.exe run > C:\syncthing-test\log.txt 2>&1
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

## 方案 B: Go Syncthing ↔ 新版 Rust 互通验证

### B1. 格雷侧启动 Go Syncthing

```powershell
# 下载官方 Go Syncthing（若未安装）
# https://github.com/syncthing/syncthing/releases

# 启动
cd C:\syncthing-test
.\syncthing.exe -home=C:\syncthing-test\go-config -no-browser
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

```powershell
# 获取 Go Syncthing 的 Device ID
curl http://127.0.0.1:8384/rest/system/status | ConvertFrom-Json | Select-Object myID

# 获取监听地址
curl http://127.0.0.1:8384/rest/system/status | ConvertFrom-Json | Select-Object -ExpandProperty listeners
```

---

## 方案 C: 旧版 Rust ↔ 新版 Rust 兼容性验证（仅当需要时）

若方案 A 成功、方案 B 成功，但旧版 Rust 仍然失败，则问题锁定为**版本兼容性 bug**。

### C1. 格雷侧保留旧版运行

保持当前 pre-fix 构建不变，仅收集日志（同样限制级别）：

```powershell
$env:RUST_LOG = "info"
<旧版二进制路径> run --log-level info > C:\syncthing-test\old-version-log.txt 2>&1
```

### C2. 需要格雷侧提供的旧版信息

```powershell
# 1. 旧版 commit hash（若已知）
cd <旧版源码目录>
git rev-parse --short HEAD

# 2. 旧版 Cargo.lock 关键依赖版本
cat Cargo.lock | findstr "^name = \"syncthing-net\"" -A 2
cat Cargo.lock | findstr "^name = \"bep-protocol\"" -A 2
cat Cargo.lock | findstr "^name = \"rustls\"" -A 2

# 3. 旧版启动时的完整日志（前 100 行 + 连接失败部分）
```

---

## 信息交换清单

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

## 快速决策树

```
格雷侧启动最新版 Rust
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

*本指南随 `main` 更新而更新。最新版本见仓库 `docs/plans/GRAY_SIDE_OPS.md`。*
