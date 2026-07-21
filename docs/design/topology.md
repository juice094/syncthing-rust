---
type: Topology
title: Project Topology and Architecture
description: syncthing-rust 的目录拓扑、Crate 依赖 DAG、运行时组件与关键入口的权威视图。
resource: ./topology.md
tags: [topology, architecture, crate-dag, runtime, okf]
status: active
project: syncthing-rust
timestamp: 2026-06-25T00:00:00Z
---

# 项目拓扑与架构

> 本文档是 Agent 快速建立项目空间感的入口。详细设计决策见 [ARCHITECTURE_DECISIONS.md](ARCHITECTURE_DECISIONS.md)，网络发现层细节见 [NETWORK_DISCOVERY_DESIGN.md](NETWORK_DISCOVERY_DESIGN.md)，功能对标见 [FEATURE_COMPARISON.md](FEATURE_COMPARISON.md)。

---

## 1. 目录拓扑（Worktree）

```
syncthing-rust/
├── cmd/
│   ├── syncthing/          # CLI 入口 + TUI 主循环 + 守护进程 + 辅助 bin
│   ├── syncthing-cli/      # generate-cert / show-id / metrics-flush
│   ├── syncthing-bench/    # 同步基准测试
│   ├── syncthing-mcp-bridge/# MCP stdio → REST API Bridge
│   └── syncthing-tray/     # Windows 托盘薄 wrapper
├── crates/                 # 核心库 Crate
│   ├── syncthing-core/     # 核心类型与 trait（无内部依赖）
│   ├── bep-protocol/       # BEP 编解码 + 握手（无 I/O）
│   ├── syncthing-net/      # TCP+TLS、ConnectionManager、发现、Relay
│   ├── syncthing-sync/     # Scanner、Puller、IndexHandler、FolderModel
│   ├── syncthing-fs/       # 文件系统抽象、.stignore、watcher
│   ├── syncthing-db/       # 元数据与块缓存（sled-backed）
│   ├── syncthing-api/      # REST API（Axum）+ 事件总线
│   ├── syncthing-versioner/# Simple / Staggered 版本控制
│   └── syncthing-test-utils/# MemoryPipe、TestNode 等测试辅助
├── docs/                   # 设计文档、计划、报告（OKF bundle）
├── scripts/                # 健康检查、cloud-deploy、压力测试
├── Cargo.toml              # Workspace 定义
├── AGENTS.md               # Agent 操作约束与红线
└── CLAUDE.md               # Claude Code 专属指引
```

---

## 2. Crate 依赖 DAG

```
cmd/syncthing
  ├─ syncthing-api
  ├─ syncthing-net
  ├─ syncthing-sync ─┬─ syncthing-db  (通过 sync::database 消费)
  │                  └─ syncthing-fs  (通过 sync 内部消费)
  ├─ syncthing-core
  ├─ syncthing-versioner
  ├─ bep-protocol
  └─ syncthing-test-utils   # 仅测试/开发工具

syncthing-core ← {protocol, net, sync, fs, db, api, versioner}
bep-protocol   ← {net, sync, test-utils}
versioner      ← {sync}
net            ← {sync, test-utils, cli}
```

### 2.1 耦合红线

| Crate | 职责 | 禁止做的事 |
|:---|:---|:---|
| `syncthing-core` | trait + 类型 + 常量 | 禁止依赖任何内部 crate；禁止放置具体实现 |
| `bep-protocol` | 协议编解码 | 禁止直接 I/O |
| `syncthing-net` | 网络传输、连接管理、发现 | 禁止同步逻辑 |
| `syncthing-sync` | 扫描、拉取、索引处理、状态机 | 禁止直接处理 wire format |
| `syncthing-fs` | 文件系统抽象、扫描、监控 | 禁止同步状态机逻辑 |
| `syncthing-db` | 存储后端 | 禁止暴露 sled 特有 API |
| `syncthing-api` | REST + 事件总线 + 配置 | 禁止直接持有 `ConnectionManagerHandle` / `LocalDatabase` 具体类型 |
| `syncthing-versioner` | 文件版本归档策略 | 禁止 FS I/O |
| `syncthing-test-utils` | 测试辅助 | 仅用于测试 / 开发工具 |

---

## 3. 运行时架构

### 3.1 启动流程

1. 安装 rustls ring crypto provider
2. 解析 CLI / 配置文件（`config.json`）
3. 加载或生成 Ed25519 TLS 证书 → 派生 `DeviceId`
4. 创建 `FileSystemDatabase`（`db/`）
5. 创建 `SyncService`，注入 `BlockSource`、并发策略、`FolderOrchestrator`、`HealthPredictor`
6. 创建 `ConnectionManager` + `TlsIdentity`
7. 注册传输层：`RawTcpTransport`、可选 `WebSocketTransport`、自研 `DerpTransport`、代理感知 `ProxiedTransport`
8. 启动 Local Discovery / Global Discovery / STUN / UPnP / Relay listener
9. 启动 REST API 服务器（绑定 `config.gui.address`，默认 `127.0.0.1:8385`）
10. 对端连接建立 → 启动 `BepSession` + `DaemonBepHandler`
11. 连接断开后自动重连

### 3.2 主要运行时组件

| 组件 | 位置 | 职责 |
|:---|:---|:---|
| `ConnectionManager` | `syncthing-net` | 维护连接池、拨号、重试、地址评分 |
| `BepSession` | `syncthing-net` | 单条 BEP 连接上的消息收发、心跳 |
| `SyncService` / `SyncManager` | `syncthing-sync` | 文件夹生命周期、扫描/拉取/watcher 调度 |
| `FolderModel` | `syncthing-sync` | 单个文件夹的本地/远程索引与状态 |
| `Puller` | `syncthing-sync` | 块请求、下载、组装文件 |
| `Scanner` | `syncthing-sync` / `syncthing-fs` | 本地文件扫描、SHA-256 哈希 |
| `IndexHandler` | `syncthing-sync` | 处理收到的 Index / IndexUpdate |
| `Versioner` | `syncthing-versioner` | 文件修改/删除前归档到 `.stversions/` |
| `FolderOrchestrator` | `syncthing-sync` | 多文件夹扫描/拉取统一调度、并发限制、抖动与优先级 |
| `HealthPredictor` | `cmd/syncthing` | 订阅同步事件，评估失败率/丢事件/状态翻转趋势并自动节流 |
| `RestApi` | `syncthing-api` | REST 端点、配置热重载、事件流 |
| `TUI` | `cmd/syncthing/src/tui/` | 实时状态、文件夹/设备表单、日志视图 |
| `Tray` | `cmd/syncthing/src/tray.rs` | Windows 托盘图标、右键菜单、daemon 启停 |

---

## 4. 关键入口

| 入口 | 文件 | 说明 |
|:---|:---|:---|
| 主二进制 | `cmd/syncthing/src/main.rs` | CLI 子命令解析、daemon / TUI / tray 生命周期 |
| Daemon 启动器 | `cmd/syncthing/src/tui/daemon_runner.rs` | TLS、ConnectionManager、SyncService、发现、API 服务器装配 |
| BEP 会话 | `crates/syncthing-net/src/session/` | 单连接消息收发与心跳 |
| 同步状态机 | `crates/syncthing-sync/src/service/` | 文件夹生命周期与扫描/拉取/watcher 循环 |
| REST 路由 | `crates/syncthing-api/src/rest/` | Axum handler 与状态机 |
| 核心 trait | `crates/syncthing-core/src/traits/mod.rs` | 跨 crate 契约 |
| 全局常量 | `crates/syncthing-core/src/constants.rs` | 端口、超时、默认值 |

---

## 5. 核心声明验证

| 声明 | 状态 | 证据 |
|:---|:---|:---|
| 完整 BEP 协议 | ✅ | `bep-protocol/` prost 编解码 + TLS 握手 |
| 端到端文件同步 | ✅ | `syncthing-sync/` Scanner / Puller / IndexHandler |
| 多路径发现 | ✅ | `syncthing-net/` UDP 广播 / HTTPS mTLS / STUN / UPnP / Relay v1 |
| 实时 TUI | ✅ | `cmd/syncthing/src/tui/` |
| REST API | ✅ | `syncthing-api/` Axum，兼容 Go 版布局 |
| 版本控制 | ✅ | `syncthing-versioner/` Simple + Staggered |
| 单静态二进制 ~13MB | ✅ | `cargo build --release` |
| 与 Go Syncthing v2.1.0 线路兼容 | ✅ | 跨版本互操作验证 |
| 测试基线 | ✅ | `cargo test --workspace` → 433 passed / 6 ignored / 0 failed |
| 双许可证 MIT + 商业 | ✅ | `LICENSE` + `LICENSE-COMMERCIAL.md` |

---

## 6. 相关概念

- [架构决策记录](ARCHITECTURE_DECISIONS.md)
- [网络发现层设计](NETWORK_DISCOVERY_DESIGN.md)
- [功能对比矩阵](FEATURE_COMPARISON.md)
- [计划索引](../plans/INDEX.md)
- [已知问题](../KNOWN_ISSUES.md)
- [工程纪律](../ENGINEERING_DISCIPLINE.md)
