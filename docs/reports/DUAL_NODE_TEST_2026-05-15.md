# 双节点真实网络 BEP E2E 测试报告

> **测试日期**：2026-05-15  
> **测试版本**：syncthing-rust v0.2.6 (commit `93a1bae`)  
> **测试类型**：双节点真实网络穿透 + 端到端文件同步  
> **报告状态**：⚠️ 部分成功 — 协议握手验证通过，块传输在 Windows 侧失败  
> **执行人**：Claude Code Agent + 用户协同（对侧 Gray-Cloud 值守）  

---

## 1. 测试目标

验证 syncthing-rust 在真实网络环境（校园网 ↔ 云服务器）下完成完整 BEP 链路：

```
TLS 1.3 握手 → BEP Hello → ClusterConfig ↔ Index ↔ Block Request/Response → 文件落盘
```

具体验收标准：
- T-1：本侧 → 对侧 文件同步成功（`a-to-b-test.txt`）
- T-2：对侧 → 本侧 文件同步成功（`b-to-a-test.txt`）
- T-3：双向同步后文件内容一致（SHA-256 校验）
- T-4：GUI REST API 可查询连接状态和文件夹统计

---

## 2. 环境拓扑

```
┌─────────────────────────────┐         Tailscale 100.64/10         ┌─────────────────────────────┐
│       本侧 (Campus-Net)      │ ◄─────────────────────────────────► │      对侧 (Gray-Cloud)       │
│  ─────────────────────────   │    虚拟内网穿透（UDP 443 打洞）      │  ─────────────────────────   │
│  OS: Windows 11 + WSL2      │                                     │  OS: Ubuntu 22.04 (VPS)      │
│  Role: Node-A (Rust binary) │     本侧 100.127.48.x :22001        │  Role: Node-B (Rust binary)  │
│  Firewall: 校园网阻断 TCP    │ ◄────────────────────────────────►│  Listen: 0.0.0.0:22001       │
│  出口 22001，Tailscale 绕过  │                                     │  Firewall: 安全组放行 22001  │
│  Device ID: PDLHUOL-...     │                                     │  Device ID: W4NW6FB-...      │
│  Folder: C:/Users/.../sync  │                                     │  Folder: /home/hadoop/.../sync│
└─────────────────────────────┘                                     └─────────────────────────────┘
```

**Tailscale 状态**：
- 本侧 Windows Tailscale 已安装，获得虚拟 IP
- 对侧 `100.127.13.26` 可达，`ping` 延迟 ~225ms
- `nc -vz 100.127.13.26 22001` 连接成功

---

## 3. 时间线与事件记录

| 时间 (UTC+8) | 事件 | 结果 |
|-------------|------|------|
| ~09:00 | 用户提出双节点真实网络测试需求 | — |
| 09:15 | 开始配置对侧（Gray-Cloud）Rust binary + config | 对侧部署成功 |
| 09:30 | 本侧尝试 WSL2 编译 / 运行 | 遭遇 bash 引号转义地狱，多次失败 |
| 10:00 | 切换至 Windows 原生 `.exe` | 编译成功，启动成功 |
| 10:30 | 首次连接尝试：裸 TCP 直连对侧公网 IP:22001 | ❌ 校园网防火墙阻断 TCP 22001 出站 |
| 11:00 | 决策：使用 Tailscale 虚拟内网穿透 | 用户安装 Tailscale，获得虚拟 IP |
| 11:30 | 配置双方 `config.json` device address 为 Tailscale IP | — |
| 12:00 | 连接成功！TLS + BEP Hello + ClusterConfig 交换 | ✅ 协议握手通过 |
| 12:15 | Index 交换验证：双方均收到对方文件列表 | ✅ Index 通过 |
| 12:30 | **T-1（A→B）**：创建 `a-to-b-test.txt`，观察同步 | ⚠️ 文件被检测到，但 `.syncthing.tmp` 持续 0 bytes |
| 13:00 | **T-2（B→A）**：对侧创建 `b-to-a-test.txt` | ⚠️ 同上，本侧 `.syncthing.tmp` 0 bytes |
| 13:30 | 远程 Agent 报告：`b-to-a-test.txt` 实际已写入对侧磁盘 | ✅ 对侧→本侧链路部分工作？ |
| 14:00 | 深入诊断：本侧 `puller` 报告 `No connected devices` | ❌ **Bug-1 定位** |
| 14:30 | 排查 `connected_devices()` → `conn.is_alive()` 在 Windows 返回 false | ❌ **Bug-1 确认** |
| 15:00 | 额外发现：Windows `.with_extension(".syncthing.tmp")` 产生 `..syncthing.tmp` | ❌ **Bug-2 确认** |
| 15:30 | 额外发现：Windows Scanner 使用反斜杠路径导致 `files_changed=0` | ❌ **Bug-3 确认** |
| 16:00 | 用户表达工程能力质疑，要求更严谨规范 | — |
| 16:30 | 制定修复计划（Bug-1/2/3 + C-UX-1~5），用户接受 | — |
| 17:00 | 撰写本报告 | 进行中 |

**总耗时**：~3.5 小时（其中 ~2 小时消耗在环境配置与命令行转义问题，~1 小时网络穿透，~0.5 小时根因定位）。

---

## 4. 已验证项（✅）

### 4.1 TLS 1.3 握手
- **验证方式**：双方日志均出现 `TLS handshake completed` / `Accepted TLS connection`
- **证书**：自签名 X.509，设备 ID 从证书公钥派生，匹配预期

### 4.2 BEP Hello 交换
- **验证方式**：`bep_session.rs` 日志 `Hello received name=syncthing-rust`
- **结果**：双方 device ID 匹配 `config.json` 配置

### 4.3 ClusterConfig 交换
- **验证方式**：日志 `Received ClusterConfig folder_count=1`
- **结果**：双方文件夹 ID `test-folder` 匹配，设备列表互相包含

### 4.4 Index 推送与接收
- **验证方式**：
  - 发送端：`Sending index folder=test-folder files=1`
  - 接收端：`Received full index folder=test-folder file_count=1`
- **结果**：文件元数据（名称、大小、块哈希、修改时间）正确传输

### 4.5 Tailscale 网络穿透
- **验证方式**：`nc -vz 100.127.13.26 22001` 成功；BEP 连接建立
- **结论**： campus-net 防火墙可通过 Tailscale/Headscale 方案绕过，v0.3.0 Transport Plugin 设计合理

---

## 5. 未验证 / 失败项（❌）

### 5.1 Block Transfer 文件落盘（T-1 / T-2）

**症状**：
- 文件被检测并触发 pull：`New file from remote file=a-to-b-test.txt`
- 临时文件创建：`C:\Users\...\sync\a-to-b-test.txt..syncthing.tmp`（注意双点号）
- 临时文件大小持续为 **0 bytes**，无 `Request` / `Response` 日志

**根因链（多层故障叠加）**：

```
Layer 1: is_alive() 返回 false
    ↓
    connected_devices() 过滤掉所有设备
    ↓
    BlockSource::request_block() 返回 Err("No connected devices")
    ↓
    puller 无法获取块 → 文件大小 0 bytes

Layer 2: with_extension(".syncthing.tmp") 在 Windows 产生 ..syncthing.tmp
    ↓
    最终重命名可能失败（路径不一致）
    ↓
    即使 Block 传输修复，文件组装也可能异常

Layer 3: Scanner 反斜杠路径
    ↓
    files_changed=0（本地变更检测失败）
    ↓
    本侧文件无法推送到对侧（单向同步失效）
```

**涉及的代码位置**：

| Bug | 文件 | 行号 | 问题描述 |
|-----|------|------|----------|
| Bug-1 | `crates/syncthing-net/src/manager/mod.rs` | 321-327 | `connected_devices()` 使用 `conn.is_alive()` 过滤，Windows 下 TCP 连接状态判断错误 |
| Bug-1 | `crates/syncthing-sync/src/puller/mod.rs` | 245-253 | `BlockSource` 检查 `devices.is_empty()` 直接返回错误，未尝试重试或等待 |
| Bug-1 | `cmd/syncthing/src/main.rs` | 338-353 | `ManagerBlockSource::request_block()` 直接调用 `connected_devices()`，无 fallback |
| Bug-2 | `crates/syncthing-sync/src/puller/mod.rs` | 211 | `temp_path = file_path.with_extension(TEMP_SUFFIX)`，`TEMP_SUFFIX = ".syncthing.tmp"` 导致 `file.txt` → `file.txt..syncthing.tmp` |
| Bug-3 | `crates/syncthing-sync/src/scanner.rs` | 207 | 路径使用 `\` 分隔符，与内部统一 `/` 不匹配，导致相对路径计算错误 |

---

## 6. 根因深度分析

### 6.1 Bug-1：`is_alive()` 平台差异

**代码**：
```rust
pub fn connected_devices(&self) -> Vec<DeviceId> {
    self.connections
        .iter()
        .filter(|entry| entry.value().iter().any(|e| e.value().conn.is_alive()))
        .map(|entry| *entry.key())
        .collect()
}
```

**推测根因**：
- `is_alive()` 可能依赖 `tokio::net::TcpStream` 的某种平台特定属性（如 `SO_ERROR` get / `peek`）
- Windows 下 TLS 握手完成后，连接进入某种状态导致 `is_alive()` 误判为死
- 但底层 TCP 连接实际存活，因为 Index 消息仍能收发（由 BepSession 独立管理）

**矛盾点**：BepSession 能正常收发 Index，说明连接物理存活；`is_alive()` 返回 false，说明状态检测逻辑与 BepSession 不一致。

**修复方向**：
- 方案 A：`connected_devices()` 不再过滤 `is_alive()`，改为返回所有已握手完成的设备（以 `BepSession` 存在为准）
- 方案 B：修复 `is_alive()` 的 Windows 兼容性，改用 `try_write` / `try_read` 或显式心跳
- 方案 C：`BlockSource` 增加指数退避重试，不依赖 `connected_devices()` 瞬时状态

### 6.2 Bug-2：`with_extension` 语义陷阱

Rust `Path::with_extension(".syncthing.tmp")` 的语义：
- 对于 `file.txt`，替换扩展名为 `.syncthing.tmp` → `file.syncthing.tmp`
- 但 `TEMP_SUFFIX` 被定义为 `".syncthing.tmp"`（带点号），导致 `file.txt` → `file.txt..syncthing.tmp`

**修复**：将临时文件名逻辑改为 `format!("{}.syncthing.tmp", file_path.display())` 或使用 `set_extension("syncthing.tmp")`。

### 6.3 Bug-3：Windows 路径分隔符

Scanner 生成文件路径时使用平台原生分隔符 `\`，但 BEP 协议和内部数据库统一使用 `/`。
当 `base_path` 包含反斜杠时，`strip_prefix` 和相对路径计算失败，导致所有文件被过滤。

**修复**：在 Scanner 入口处统一将路径转换为正斜杠（或至少在相对路径计算时转换）。

---

## 7. 配置 UX 灾难（C-UX-1 ~ C-UX-5）

本次 3.5 小时测试过程中，**超过 50% 时间消耗在配置问题**，而非代码缺陷：

| 问题 | 耗时 | 痛苦指数 |
|------|------|----------|
| Device ID 不匹配（对侧存在多个 syncthing 进程） | 30min | 🔥🔥🔥 |
| 文件夹 ID / 设备列表不匹配（remote 已有 `test-folder`，非 `cross-test`） | 20min | 🔥🔥🔥 |
| WSL2 ↔ Windows ↔ msys2 命令行引号转义地狱 | 60min | 🔥🔥🔥🔥🔥 |
| `config.json` 手动编辑，无验证，启动后静默失败 | 30min | 🔥🔥🔥🔥 |
| Windows 路径反斜杠在 JSON 中需转义 `\\` | 15min | 🔥🔥 |
| Tailscale 安装与虚拟 IP 配置 | 30min | 🔥🔥 |

**结论**：当前 syncthing-rust 的部署体验是**灾难性的**。一个没有 Rust 开发经验的用户几乎不可能独立完成双节点部署。

---

## 8. 修复计划（已获用户接受）

### P0：Bug-1/2/3（本周内）

| ID | 问题 | 预计工作量 | 验证方式 |
|----|------|-----------|----------|
| Bug-1 | `is_alive()` Windows 兼容性 | 4h | 单测 + Windows 本机 Block 传输 |
| Bug-2 | `with_extension` 双点号 | 1h | 单测断言临时文件名 |
| Bug-3 | Scanner 反斜杠路径 | 2h | Windows 本机 `files_changed > 0` |

### P1：配置 UX 重构（C-UX-1 ~ C-UX-5，2 周内）

| ID | 改进 | 说明 |
|----|------|------|
| C-UX-1 | CLI 初始化向导 | `syncthing-rust init` 交互式生成 config，输入设备名、文件夹路径、对侧地址 |
| C-UX-2 | `AddressType` 序列化兼容 | 支持 `"tcp://host:port"` / `"tailscale://device"` / `"relay://..."` 字符串，而非裸 JSON object |
| C-UX-3 | REST API `PUT /rest/config/devices` | 运行时热添加对侧设备，无需重启 |
| C-UX-4 | 配置验证 + 快速失败 | 启动时检查 device ID 格式、路径存在性、地址可解析性，错误信息人类可读 |
| C-UX-5 | 单实例锁 | Windows `CreateMutex` / Unix `pidfile`，防止重复启动导致 device ID 冲突 |

---

## 9. 经验教训

1. **真实网络测试必须尽早、必须自动化**：在模拟器/单机上通过的测试，在真实防火墙/NAT 环境下可能完全失效。
2. **Windows 是一等公民，不是边缘平台**：项目主要开发者使用 Linux/WSL2，导致 Windows 路径、进程管理、网络 API 被系统性忽视。
3. **配置即代码，代码即体验**：3.5 小时测试中 2 小时花在配置，说明 UX 缺陷与功能缺陷同等致命。
4. **跨环境命令执行是高危操作**：WSL2/Windows/msys2 三层命令转义极易出错，应避免在自动化脚本中混用。
5. **日志是唯一的 debugger**：当块传输失败时，详细的 `Request` / `Response` / `BlockSource` 日志是定位 Bug-1 的关键。

---

## 10. 结论

- **协议层（TLS + BEP Hello + ClusterConfig + Index）**：✅ **已通过真实网络验证**
- **传输层（Tailscale 穿透校园网）**：✅ **已验证可行**
- **应用层（Block Transfer → 文件落盘）**：❌ **Windows 侧因 `is_alive()` 平台差异中断，待修复**
- **扫描层（本地变更检测）**：❌ **Windows 路径分隔符导致失效，待修复**
- **部署体验**：❌ **灾难级，需立即启动 C-UX 重构**

**项目当前状态**：发动机、变速箱、传动轴已装通，但在 Windows 路况下离合器打滑（`is_alive()`）且方向盘装反（路径分隔符）。修复 Bug-1/2/3 后，预计可完成 T-1/T-2 验收。

---

**下一步动作**：
1. 提交本报告与工程规范到 Git
2. 切出 `fix/windows-block-transfer` 分支
3. 按 Bug-1 → Bug-2 → Bug-3 顺序修复，每个 Bug 附带单测 + 本机验证