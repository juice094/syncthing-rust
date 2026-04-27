# 离线交接文档 · syncthing-rust 互通验证

> **制定日期**: 2026-04-27
> **场景**: juice094 离开校园网环境，无法操作本机，仅可通过微信/QQ/邮件与格雷侧联系
> **本侧环境**: 校园网内网 (10.3.155.142)，无公网 IP，Tailscale 不可用
> **格雷侧环境**: kimiclaw 服务器虚拟机

---

## 一、本侧（juice094-PC）关键身份信息

| 信息项 | 值 | 说明 |
|--------|-----|------|
| **Device ID** | `XQVFE6J-4JCJRXW-4PSMU25-ZKZ3AKB-52XN6KZ-W6TRN5Y-4PH45KZ-XK4V3A6` | 本机唯一设备标识 |
| **局域网 IP** | `10.3.155.142` | 校园网内网地址，NAT 后 |
| **监听端口** | `22001` | ⚠️ 注意：不是默认的 22000 |
| **API 地址** | `http://10.3.155.142:8385` | REST API 端点 |
| **API Key** | `3c40m4cube54qrtvk934yxjy9gz3l5y6` | 用于 API 调用认证 |
| **设备名** | `juice094-PC` | config.json 中配置 |
| **证书路径** | `%LOCALAPPDATA%\syncthing-rust\cert.pem` / `key.pem` | Ed25519 TLS 证书 |

---

## 二、网络拓扑与限制（格雷侧必须理解）

```
┌─────────────────────────────────────────────────────────────┐
│  格雷侧 (kimiclaw 服务器虚拟机)                                │
│  ┌──────────────────────────────────────────────┐           │
│  │  可能有公网 IP / 弹性 IP / 端口映射            │           │
│  │  或仅在宿主机 NAT 后                          │           │
│  └──────────────────┬───────────────────────────┘           │
│                     │                                       │
│           互联网 / 校园网                                   │
│                     │                                       │
│  ┌──────────────────▼───────────────────────────┐           │
│  │  校园网 NAT (阻止所有入站)                    │           │
│  │  10.3.155.142 不可从外部直接访问             │           │
│  └──────────────────┬───────────────────────────┘           │
│                     │                                       │
│  ┌──────────────────▼───────────────────────────┐           │
│  │  juice094-PC (校园网内网)                     │           │
│  │  IP: 10.3.155.142, Port: 22001               │           │
│  │  Tailscale: ❌ 不可用 (NoState)               │           │
│  │  Relay: ❌ 当前关闭                           │           │
│  │  Global Discovery: ❌ 当前关闭                │           │
│  └──────────────────────────────────────────────┘           │
└─────────────────────────────────────────────────────────────┘
```

### 关键限制

1. **本侧无公网 IP**：校园网 NAT 阻止所有入站 TCP 连接。格雷侧无法主动连接本侧。
2. **Tailscale 不可用**：服务运行但状态 `NoState`，校园网可能封锁 UDP/41641。
3. **本侧当前配置关闭了 Relay 和 Global Discovery**：
   - `relays_enabled: false`
   - `global_announce_enabled: false`
   - `local_announce_enabled: true`（仅局域网有效）

### 结论

**在当前配置下，跨网络互通是不可能的。** 本侧只能主动出站连接格雷侧（如果格雷侧有可访问地址），或必须修改配置启用 Relay/Global Discovery。

---

## 三、格雷侧可立即执行的准备工作（无需本机配合）

### 步骤 1：确认格雷侧网络可达性

```bash
# 在格雷侧虚拟机执行
# 1. 确认本机监听端口
ss -tlnp | grep 22000    # 或 22001，看实际配置

# 2. 确认公网 IP
curl -s https://api.ipify.org

# 3. 确认端口可从外部访问（如果格雷侧有公网IP）
# 从另一台机器测试:
# telnet <格雷侧公网IP> 22000

# 4. 如果是云服务器，检查安全组/防火墙
# 阿里云: 安全组规则需放行 TCP 22000/22001, 8384/8385
# 腾讯云: 防火墙规则同上
```

### 步骤 2：准备 syncthing-rust 最新版

```bash
# 1. 克隆/拉取仓库
git clone https://github.com/juice094/syncthing-rust.git
cd syncthing-rust
git checkout main
# 确认版本: git rev-parse --short HEAD → 应为 88509b9 或更新

# 2. 编译 release 版本
cargo build --release -p syncthing

# 3. 创建测试目录
mkdir -p ~/syncthing-test/sync-folder
```

### 步骤 3：准备配置文件

创建 `~/syncthing-test/config.json`：

```json
{
  "version": 1,
  "listen_addr": "0.0.0.0:22000",
  "device_name": "gray-kimiclaw",
  "folders": [
    {
      "id": "test-folder",
      "path": "/home/<user>/syncthing-test/sync-folder",
      "label": "互通测试文件夹",
      "folder_type": "SendReceive",
      "paused": false,
      "rescan_interval_secs": 10,
      "devices": ["XQVFE6J-4JCJRXW-4PSMU25-ZKZ3AKB-52XN6KZ-W6TRN5Y-4PH45KZ-XK4V3A6"],
      "ignore_patterns": [],
      "versioning": null
    }
  ],
  "devices": [
    {
      "id": "XQVFE6J-4JCJRXW-4PSMU25-ZKZ3AKB-52XN6KZ-W6TRN5Y-4PH45KZ-XK4V3A6",
      "name": "juice094-PC",
      "addresses": ["Dynamic"],
      "paused": false,
      "introducer": false
    }
  ],
  "options": {
    "listen_addresses": ["0.0.0.0:22000"],
    "global_announce_enabled": true,
    "local_announce_enabled": true,
    "relays_enabled": true
  }
}
```

⚠️ **重要**：格雷侧配置中 `relays_enabled: true` 和 `global_announce_enabled: true`，以便本侧发现格雷侧地址。

### 步骤 4：启动并收集日志

```bash
cd ~/syncthing-test
RUST_LOG=info,syncthing_net=debug,syncthing_sync=debug \
  ~/syncthing-rust/target/release/syncthing run \
  --config ~/syncthing-test/config.json \
  --gui-address 0.0.0.0:8385 > gray-log.txt 2>&1
```

---

## 四、需要 juice094 回来后才能执行的配置修改

由于本侧当前关闭了 Relay 和 Global Discovery，跨网络连接需要以下修改之一：

### 方案 1：启用 Relay + Global Discovery（推荐）

juice094 回到校园网后，修改 `%LOCALAPPDATA%\syncthing-rust\config.json`：

```json
"options": {
  "listen_addresses": ["0.0.0.0:22001"],
  "global_announce_enabled": true,    // ← 改为 true
  "local_announce_enabled": true,
  "relays_enabled": true              // ← 改为 true
}
```

修改后重启 syncthing-rust。这样即使本侧无公网 IP，也能通过 Relay 服务器和 Global Discovery 与格雷侧建立连接。

### 方案 2：手动配置格雷侧地址（如果格雷侧有公网IP）

如果格雷侧确认有公网 IP 且端口已放行：

```json
"devices": [
  {
    "id": "<格雷侧 DeviceID>",
    "name": "gray-kimiclaw",
    "addresses": [
      {"Tcp": "<格雷侧公网IP>:22000"},
      "Dynamic"
    ],
    "paused": false,
    "introducer": false
  }
]
```

### 方案 3：修复 Tailscale（如果校园网允许）

如果校园网只是临时封锁 Tailscale UDP：

```powershell
# 尝试重启 Tailscale 服务
Restart-Service tailscale
# 等待 30 秒后检查
tailscale status
tailscale ip
```

若成功获取 `100.x.x.x` IP，双方可通过 Tailscale 虚拟网络互通。

---

## 五、信息交换清单（格雷侧 ↔ juice094）

| 信息 | 已知方 | 待确认 |
|------|--------|--------|
| 本侧 Device ID | ✅ juice094 | — |
| 本侧局域网 IP | ✅ juice094 | — |
| 本侧监听端口 | ✅ juice094 (22001) | — |
| 格雷侧 Device ID | — | 格雷侧启动后提供 |
| 格雷侧公网 IP | — | 格雷侧提供 |
| 格雷侧监听端口 | — | 格雷侧提供 (默认 22000) |
| 格雷侧是否在同一校园网 | — | 格雷侧确认 |
| 双方是否都启用 Relay | — | 需协调 |

---

## 六、快速决策树

```
格雷侧准备好最新版 Rust syncthing-rust
        │
        ▼
格雷侧是否有公网 IP 且端口放行?
    ┌───┴───┐
    ▼       ▼
   是      否
    │       │
    ▼       ▼
 将公网IP  必须启用 Relay
 告知      + Global Discovery
 juice094  (双方都需要)
    │       │
    ▼       ▼
 juice094  juice094
 回来后    回来后
 手动配置  修改配置
 格雷侧    启用 Relay
 地址      和 Discovery
    │       │
    └───────┘
        │
        ▼
   双方启动
   观察日志
   确认连接
```

---

## 七、格雷侧日志中需要关注的关键行

启动格雷侧后，观察日志中是否出现以下内容：

| 日志内容 | 含义 | 期望 |
|----------|------|------|
| `Connection manager started on ...` | 监听成功 | ✅ 必须出现 |
| `Global discovery announce succeeded` | 全局发现注册成功 | ✅ 若启用 global_announce |
| `Parallel dialing ... with ... relay candidates` | 拨号行为 | ✅ 若启用 relays |
| `Client TLS handshake completed` | TLS 握手成功 | ✅ 连接建立的关键标志 |
| `BEP connection established` | BEP 连接建立 | ✅ 最终目标 |
| `Connection registered for device` | 连接注册成功 | ✅ 双方互信完成 |
| `os error 10061` / `Connection refused` | 连接被拒绝 | ❌ 说明网络不可达 |
| `Heartbeat timeout` | 心跳超时 | ⚠️ 连接不稳定，需关注频率 |

---

## 八、stress test 状态（背景任务）

本机当前正在执行 30 分钟短周期 stress test 预验证（`bin/stress_test.rs`）。

截至 02:04（运行约 16 分钟），状态健康：
- ✅ 双节点启动、证书生成、SyncService 启动正常
- ✅ TLS + BEP Hello 双向交换持续成功
- ✅ 故障注入（断开→重连）循环工作正常
- ✅ 重连调度器在 ~2s 内触发
- ✅ 连接竞争解决机制工作正常
- ⚠️ 偶发 `Heartbeat timeout`（预期行为，连接重建期间）
- ❌ 无 panic，无致命错误

完整日志路径（本机）：`C:\Users\22414\.kimi\sessions\...\tasks\bash-ohdy3g9d\output.log`

---

## 九、联系人信息

| 项目 | 内容 |
|------|------|
| 本侧负责人 | juice094 |
| 联系窗口 | 微信/QQ/邮件（用户自行填写） |
| 本侧不可用时段 | 离开校园网期间 |
| 本侧可恢复操作时间 | 返回校园网后 |

---

*本文档已提交至 GitHub: `docs/plans/OFFLINE_HANDOVER_2026-04-27.md`*
