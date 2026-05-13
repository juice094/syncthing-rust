# syncthing-rust

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![Tests](https://img.shields.io/badge/tests-296%20passed-brightgreen)]()
[![Clippy](https://img.shields.io/badge/clippy-0%20warnings-brightgreen)]()
[![Version](https://img.shields.io/badge/version-v0.2.5-blue)]()
[![License](https://img.shields.io/badge/license-MIT-blue)](./LICENSE)

[Syncthing](https://syncthing.net/) 协议栈的 Rust 实现，设计目标为**零运行时依赖**部署，与官方 Go Syncthing 守护进程线级兼容互操作。

> ⚠️ **当前阶段（2026-05-13 post-T2.6）**：**alpha — 核心 sync 链路已打通，但未到生产**。
> 
> - ✅ **连接层稳定**：9h+ 压测 758 次连接周期，0 死锁、0 panic（T-F1 已修复）。
> - ✅ **协议层正确**：TLS、BEP Hello、ClusterConfig、Index 编解码全部正常。
> - ✅ **端到端同步已打通**（T2.6 修复，2026-05-13）：`e2e_sync` 测试通过，单文件双节点同步 ~12s 完成（TLS → Hello → ClusterConfig → Index → Block → 文件落地）。详细 RCA 见 [`docs/KNOWN_ISSUES.md`](./docs/KNOWN_ISSUES.md) §2。
> - ⏳ **72h 长跑、跨版本互通自动化** 仍未完成。
>
> **尚未达到 Go Syncthing 的替代水准**，但已适用于：研究 BEP 协议、参考 Rust 实现、做受控实验、参与开发。

---

## 概览

| 维度 | 状态 |
|-----------|-------|
| BEP 协议（TLS + Hello + ClusterConfig + Index + Request/Response） | ✅ 编解码 + 握手验证通过 |
| **端到端文件同步**（A→B 实际传输） | ✅ **工作正常**（`e2e_sync` 测试通过，T2.6 修复） |
| 文件同步内部模块（puller / scanner / folder_model）单测 | ✅ 295 unit tests 全通过 |
| 网络发现（Local + Global + STUN + UPnP + Relay v1） | ✅ 实现完成；ParallelDialer 带 RTT 评分 |
| REST API（读写，兼容 Go 布局） | ✅ 读路径完整；写路径完成 |
| 测试 | **295 unit + 1 e2e 通过，0 ignored** |
| 代码检查 | **0 clippy warnings**（含 `await_holding_lock` + `manual_let_else`） |
| 安全审计 | **3 unmaintained** 上游传递依赖（已接受债务，见 `.cargo/audit.toml`） |
| 二进制体积 | ~12 MB（release，Windows x64） |

> **当前限制（务必阅读）**：
> - **§1 ClusterConfig 首次握手 10s 超时**：首次连接约 12s 才完成 ClusterConfig 交换（自动重连第二轮兜底）。
> - 72h 压测在 Windows 桌面不可行（休眠杀进程），需 Linux 平台。
> - Go Syncthing 完整文件同步互操作仅 2026-04-11 手工验证一次，无自动化。

---

## 快速开始（Windows）

```powershell
# 1. 编译 release 二进制（现代硬件上 < 1 分钟）
cargo build --release -p syncthing

# 2. 带交互式 TUI 运行
cargo run --release -p syncthing -- tui

# 3. 或 headless 运行
cargo run --release -p syncthing -- run
```

首次运行会生成 Ed25519 TLS 证书并存放在 `%LOCALAPPDATA%\syncthing-rust`。

默认端口：BEP `22001`，REST API `8385`。环回地址在本地调试时绕过 API Key 认证。

### 验证运行

```powershell
# 检查 REST 健康状态
curl http://127.0.0.1:8385/rest/system/status | ConvertFrom-Json

# 预期：uptime > 0，folders/devices 数量与配置匹配
```

---

## 功能清单

**已实现**
- 与官方 Go Syncthing 节点建立 TLS 加密 BEP 会话。
- 通过 `Request`/`Response` 逐块拉取文件并在本地重组。
- 被动响应块请求（上传）给已连接节点。
- 扫描本地文件夹，计算 SHA-256 块哈希，广播 `IndexUpdate`。
- 文件系统变更监听（`notify` + 1s debounce → 扫描 → 约 2s 内广播）。
- 多路径节点发现：LAN UDP 广播、Global Discovery（HTTPS mTLS）、STUN、UPnP、Syncthing Relay v1。
- ParallelDialer 竞速直连 TCP 与 Relay 候选路径，带 RTT 评分。
- REST API（兼容 Go 布局），含读写端点（配置、暂停/恢复、扫描、重启/关机）。
- TUI 实时同步状态（文件夹状态、设备连接、同步进度）通过事件桥。
- `config.json` 热重载（notify-based watcher，无需重启）。

**尚未实现**
- 主动 Push 调度（扫描触发本地索引更新，但不主动请求节点拉取）。
- Web GUI（仅 TUI）。
- QUIC 传输。
- 生产级打包（systemd service / MSI 安装器）。

---

## 路线图

| 阶段 | 目标 | 状态 |
|-------|------|--------|
| **Phase 1** | 核心 BEP 协议（TLS, Hello, ClusterConfig, Index） | ✅ 完成 |
| **Phase 2** | 网络抽象、watcher、REST API、双节点共存 | ✅ 完成 |
| **Phase 3** | BepSession 可观测性、Push/Pull E2E 远程节点 | ✅ 完成（已验证格雷侧 pre-fix Rust 构建；Go 节点待验证） |
| **Phase 3.5** | 连接稳定性、配置持久化 | ✅ 完成 |
| **Phase 4** | TUI 硬化（事件桥、实时同步状态、配置热重载） | ✅ 完成 |
| **Phase 5** | Zero-Tailscale 互联（发现结果 → ConnectionManager 地址池） | 🔵 核心已集成；现场验证待完成 |
| **Phase A** | 安全债务接受（cargo audit） | ✅ 完成（`.cargo/audit.toml` 已创建） |
| **Phase B** | 72h 压测 | ⏳ 基础设施就绪（`bin/stress_test.rs` 存在）；执行中 |
| **Phase C** | REST API 写路径闭环 | ✅ 完成（override/revert 已实现，scan `sub` 已支持，device pause/resume body 已生效） |

Phase 5 设计：[`docs/design/NETWORK_DISCOVERY_DESIGN.md`](docs/design/NETWORK_DISCOVERY_DESIGN.md)。

当前路线图：[`docs/plans/POST_V0_2_0_ROADMAP.md`](docs/plans/POST_V0_2_0_ROADMAP.md)。
计划索引：[`docs/plans/INDEX.md`](docs/plans/INDEX.md)。
计划审计：[`docs/plans/PLAN_AUDIT_2026-04-27.md`](docs/plans/PLAN_AUDIT_2026-04-27.md)。

---

## 架构

```
cmd/syncthing/          # CLI 入口 + TUI
crates/
├── syncthing-core/     # DeviceId, FileInfo, VersionVector — 稳定只读边界
├── bep-protocol/       # BEP Hello, Request/Response, Index, ClusterConfig
├── syncthing-net/      # TCP+TLS, ConnectionManager, dialer, discovery, relay
├── syncthing-sync/     # SyncService, Scanner, Puller, IndexHandler, watcher
├── syncthing-api/      # REST API 服务器（Axum）
└── syncthing-db/       # 元数据与块缓存抽象
docs/
├── design/             # 活跃 ADR 与网络设计
├── plans/              # 路线图与改进计划
├── reports/            # 验证报告、实现总结
└── archive/            # 历史决策
```

> **信任边界**：`syncthing-core` 对下游 crate 只读。详见 [`docs/design/ARCHITECTURE_DECISIONS.md`](docs/design/ARCHITECTURE_DECISIONS.md)。

---

## 文档

| 文档 | 用途 |
|----------|--------|
| [`docs/README.md`](docs/README.md) | 文档导航 |
| [`docs/design/ARCHITECTURE_DECISIONS.md`](docs/design/ARCHITECTURE_DECISIONS.md) | 架构决策记录（ADR） |
| [`docs/design/NETWORK_DISCOVERY_DESIGN.md`](docs/design/NETWORK_DISCOVERY_DESIGN.md) | 网络发现层设计 |
| [`docs/reports/IMPLEMENTATION_SUMMARY.md`](docs/reports/IMPLEMENTATION_SUMMARY.md) | Crate 级实现状态 |
| [`docs/reports/VERIFICATION_REPORT_BEP_2026-04-11.md`](docs/reports/VERIFICATION_REPORT_BEP_2026-04-11.md) | BEP 互操作测试报告 |
| [`docs/design/FEATURE_COMPARISON.md`](docs/design/FEATURE_COMPARISON.md) | 与 Go Syncthing 的功能对标 |
| [`docs/plans/INDEX.md`](docs/plans/INDEX.md) | 计划文档导航与交叉引用 |
| [`docs/plans/PLAN_AUDIT_2026-04-27.md`](docs/plans/PLAN_AUDIT_2026-04-27.md) | 计划有效性审计与项目阶段重新校准 |
| [`docs/ai-protocol.md`](docs/ai-protocol.md) | AI Agent 跨会话状态锚点 |

---

## 贡献

见 [`CONTRIBUTING.md`](./CONTRIBUTING.md)。精简版：

```powershell
cargo test --workspace          # 必须通过
cargo clippy --all-targets      # 必须为 0 warnings

# 或使用本地健康检查脚本（Windows）
.\scripts\check-health.ps1
```

---

## 许可证

[MIT License](./LICENSE)。
