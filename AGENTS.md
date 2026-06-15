# Agent 环境指引 · syncthing-rust

> 本文档面向 AI 编程 Agent。读者应被假设对项目一无所知。所有信息基于仓库实际内容（`Cargo.toml`、`README.md`、源码、CI、脚本、文档等）实测整理，不做推测。

---

## 1. 项目概述

`syncthing-rust` 是 [Syncthing](https://syncthing.net/) BEP（Block Exchange Protocol）协议的 Rust 实现，目标是成为 Go 版 Syncthing 的替代实现。

- **当前版本**：`CHANGELOG.md` 声明为 `3.0.3`（2026-06-13），最新 Git 标签为 `v3.0.2`，工作区内所有 crate 的 `Cargo.toml` 版本统一为 `3.0.0`。
- **当前阶段**：Production（谨慎生产）。核心 P2P 文件同步、Windows 托盘、TUI、REST API 已稳定运行；72h 耐久测试为 `v3.1.0` 准入线（尚未完成）。
- **主要特性**：
  - BEP over TLS（prost 编解码 + LZ4 压缩）
  - 块级 Pull/Push 双向同步
  - 三路文本合并冲突解决（git 风格冲突标记）
  - Simple / Staggered 版本控制（`.stversions/`）
  - 多路径发现：LAN UDP 广播、Global Discovery（HTTPS mTLS）、STUN、UPnP、Relay v1
  - 实时 TUI、Windows 系统托盘、REST API（Axum，兼容 Go 版布局）
  - 与 Go Syncthing 线路兼容（已跨版本验证）

---

## 2. 技术栈

| 层级 | 技术 |
|------|------|
| 语言 | Rust（`edition = "2021"`，工具链 `stable`，要求 Rust 1.85+） |
| 异步运行时 | Tokio（`workspace` 统一 `tokio = { version = "1.35", features = ["full"] }`） |
| TLS | rustls 0.23 + tokio-rustls，ed25519-dalek 设备身份证书 |
| 协议 | BEP（prost 0.12）+ LZ4 压缩 |
| 网络 | TCP + TLS，`ParallelDialer`，Relay v1，自研 DERP，可选 WebSocket（`websocket` feature） |
| 发现 | UDP 广播、HTTPS mTLS Global Discovery、STUN、UPnP、PCP、NAT-PMP |
| 存储 | sled 0.34（元数据 + 块缓存），LRU 缓存 |
| REST API | Axum 0.7 + tower-http |
| TUI | ratatui 0.30 + crossterm 0.28 |
| 托盘 | Windows Win32 API（`windows = "0.58"`） |
| 日志 | tracing + tracing-subscriber + tracing-appender（按小时轮转） |
| CLI | clap 4.5 |
| 构建工具 | Cargo + just（`justfile`） |

---

## 3. 仓库结构与代码组织

项目采用 **Cargo Workspace**，分为 `crates/`（库）和 `cmd/`（可执行文件）两大类。

```
syncthing-rust/
├── Cargo.toml              # workspace 定义 + 统一依赖
├── justfile                # 常用开发命令
├── rust-toolchain.toml     # stable + rustfmt + clippy
├── .cargo/config.toml      # release-thin / bench profile
├── deny.toml               # cargo-deny 配置（CI 使用）
├── cargo-deny.toml         # CI 兼容版 cargo-deny 配置
├── .cargo/audit.toml       # cargo audit 忽略项
│
├── crates/                 # 库 crate
│   ├── syncthing-core/     # 核心类型、trait、错误、常量（禁止依赖内部 crate）
│   ├── bep-protocol/       # BEP 消息 prost 编解码、握手、连接帧（无 I/O）
│   ├── syncthing-net/      # TCP+TLS、ConnectionManager、拨号、发现、Relay、STUN/UPnP
│   ├── syncthing-sync/     # Scanner、Puller、IndexHandler、FolderModel、Watcher、冲突解决
│   ├── syncthing-fs/       # 文件系统抽象、扫描、.stignore 解析、监控
│   ├── syncthing-db/       # sled 元数据/块存储、缓存
│   ├── syncthing-api/      # REST API（Axum）、配置存储、事件总线
│   ├── syncthing-versioner/# Simple / Staggered 版本控制
│   └── syncthing-test-utils/# 测试辅助：MemoryPipe、TestNode
│
├── cmd/                    # 可执行文件
│   ├── syncthing/          # 主守护进程 + TUI + 托盘（Windows）+ 4 个 bin target
│   ├── syncthing-cli/      # 命令行工具：generate-cert / show-id / metrics-flush
│   ├── syncthing-bench/    # 同步基准测试工具
│   ├── syncthing-mcp-bridge/# MCP stdio → REST API Bridge
│   └── syncthing-tray/     # 托盘薄 wrapper（当前无外部依赖）
│
├── docs/                   # 设计文档、计划、报告、操作手册
├── scripts/                # 部署、压测、健康检查脚本（bash + PowerShell）
└── acceptance-tests/       # 验收测试（独立 Cargo 包）
```

### 3.1 核心 crate 边界（红线）

| Crate | 职责 | 禁止做的事 |
|-------|------|-----------|
| `syncthing-core` | trait + 类型 + 常量 | 禁止依赖任何内部 crate；禁止放置具体实现 |
| `bep-protocol` | 协议编解码 | 禁止直接 I/O |
| `syncthing-net` | 网络传输、连接管理、发现 | 禁止同步逻辑 |
| `syncthing-sync` | 扫描、拉取、索引处理、状态机 | 禁止直接处理 wire format |
| `syncthing-fs` | 文件系统抽象、扫描、监控 | 禁止同步状态机逻辑 |
| `syncthing-db` | 存储后端 | 禁止暴露 sled 特有 API；同步逻辑应通过 `BlockStore` trait |
| `syncthing-api` | REST + 事件总线 + 配置 | 禁止直接持有 `ConnectionManagerHandle`/`LocalDatabase` 等具体类型；必须走 trait |
| `syncthing-versioner` | 文件版本归档策略 | 禁止 FS I/O |
| `syncthing-test-utils` | 测试辅助（`MemoryPipe`、`TestNode`） | 仅用于测试 / 开发工具 |

### 3.2 关键入口与模块

- **主入口**：`cmd/syncthing/src/main.rs`
  - 子命令：`run`、`tui`、`init`、`status`、`devices list`、`folders list`、`logs`、`install-autostart`、`uninstall-autostart`
  - 无参数启动（Windows + `tray` feature）：`daemon + 系统托盘`
- **Daemon 启动器**：`cmd/syncthing/src/tui/daemon_runner.rs`
  - 负责加载 TLS 证书、创建 `ConnectionManager`、`SyncService`、注册传输层、启动发现任务、API 服务器
- **BEP 会话**：`crates/syncthing-net/src/session/`
- **同步状态机**：`crates/syncthing-sync/src/service/`
- **REST 路由**：`crates/syncthing-api/src/rest/`
- **核心 trait**：`crates/syncthing-core/src/traits/mod.rs`

---

## 4. 构建与运行

### 4.1 环境要求

- Rust 1.85.0+
- `rustfmt`、`clippy`（`rust-toolchain.toml` 已指定）
- Windows 是主要开发/运行平台；Linux/macOS 可编译运行

### 4.2 常用命令

```bash
# 编译 release
cargo build --release

# 运行守护进程
cargo run --release -- run --config-dir ~/.syncthing

# 交互式初始化向导
cargo run --release -- init

# 启动 TUI
cargo run --release -- tui --config-dir ~/.syncthing

# CLI 查询状态
cargo run -p syncthing-cli -- status
```

### 4.3 just 命令（推荐）

```bash
just --list              # 查看所有命令
just check               # fmt + clippy + test + doc + audit
just test                # cargo test --workspace
just clippy              # cargo clippy --workspace --all-targets
just deny                # cargo deny check all
just e2e                 # release 模式 E2E 同步测试
just release-check       # cargo check --release --workspace
just fmt                 # cargo fmt --all
just doc                 # cargo doc --no-deps --workspace
just bench-smoke         # benchmark 冒烟测试
just build-release       # 编译 release 二进制（syncthing / cli / monitor）
```

### 4.4 Profile

- `release`：`lto = true`、`codegen-units = 1`、`opt-level = 3`（正式发行）
- `release-thin`：`lto = "thin"`、`codegen-units = 16`（开发期快速验证）
- `bench`：`debug = true`（给 criterion 生成 flamegraph 用）

---

## 5. 测试策略

### 5.1 测试基线（实测）

- `cargo test --workspace`：**364 passed / 4 ignored / 0 failed**
- `cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock`：0 warnings
- `cargo doc --no-deps --workspace`：通过
- `cargo audit`：3 个 unmaintained 上游传递依赖已记录在 `.cargo/audit.toml` 中接受为债务
- `cargo deny check all`：通过（仅保留 `zune-jpeg` 等重复版本警告，已接受）

### 5.2 测试组织

| 类型 | 位置 | 说明 |
|------|------|------|
| 单元测试 | `src/*.rs` / `src/*/tests.rs` | 各 crate 内部模块测试 |
| 集成测试 | `crates/*/tests/*.rs` | 如 `bep-protocol/tests/wire_compat.rs` |
| E2E 测试 | `cmd/syncthing/tests/e2e_sync.rs` | 双节点真实同步链路 |
| 验收测试 | `acceptance-tests/` | 独立包 |
| Benchmark | `crates/*/benches/*.rs` | criterion：`device_id`、`encode_decode`、`scanner`、`hash_parallel`、`puller` |
| 压力测试 | `cmd/syncthing/src/bin/stress_test.rs` + `cmd/syncthing/src/bin/monitor.rs` | 72h 耐久测试基础设施 |

### 5.3 E2E / 网络测试要求

- 网络层改动必须通过 `TestNode` 双实例验证；单实例测试视为无效
- 新增端到端行为必须配套集成测试，禁止仅用 `#[cfg(test)]` 单元测试覆盖
- `test_two_node_single_file_sync` 当前因 ClusterConfig race 被 `#[ignore]`，生产代码已通过 reconnect 逻辑规避

### 5.4 提交前检查清单

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock
cargo fmt --all -- --check
cargo doc --no-deps --workspace
```

---

## 6. 代码风格与开发约定

### 6.1 格式化

- 必须运行 `cargo fmt` 后提交
- 单文件软上限 **600 行**；CI 会检查并警告
- **当前超过 600 行的生产文件**（需拆分或保持关注）：
  - `cmd/syncthing/src/main.rs`（1699 行）
  - `crates/syncthing-sync/src/puller/mod.rs`（837 行）
  - `crates/syncthing-sync/src/scanner.rs`（812 行）
  - `cmd/syncthing/src/tui/mod.rs`（703 行）
  - `cmd/syncthing/src/tray.rs`（635 行）

### 6.2 日志级别约定

| 级别 | 用途 |
|------|------|
| `trace` | 块级细节 |
| `debug` | 状态转换 |
| `info` | 生命周期 |
| `warn` | 可恢复错误 |
| `error` | 失败 |

### 6.3 错误处理

- 生产路径优先使用 `thiserror`/`anyhow`
- **禁止生产代码使用 `unwrap()` / `expect()`**（测试代码除外）
- 新增错误路径必须记录日志，不能静默吞掉

### 6.4 异步

- 仅使用 `tokio`，禁止使用 `async-std`
- 注意 `clippy::await_holding_lock` 警告

### 6.5 提交规范（Conventional Commits）

```
feat:     新功能
fix:      Bug 修复
docs:     文档
refactor: 重构（无行为变化）
test:     测试
chore:    构建/工具
perf:     性能
```

示例：
```
fix(sync): Windows rename fallback with exponential backoff
```

### 6.6 注释与文档

- 新增 public API 必须写 doc comment
- 关键常量集中在 `syncthing-core::constants`
- 中文注释在项目中大量使用；新增代码可继续使用中文注释以保持风格一致

---

## 7. 运行时架构

### 7.1 进程启动流程（`syncthing run` / auto 模式）

1. 安装 rustls ring crypto provider
2. 解析 CLI / 配置文件（`config.json`）
3. 加载或生成 Ed25519 TLS 证书（`cert.pem` / `key.pem`）→ 派生 `DeviceId`
4. 创建 `FileSystemDatabase`（`db/`）
5. 创建 `SyncService`（持有配置和 DB）
6. 创建 `ConnectionManager` + `TlsIdentity`
7. 注册传输层：`RawTcpTransport`（默认）、可选 `WebSocketTransport`（需 `websocket` feature）、自研 `DerpTransport`、代理感知 `ProxiedTransport`
8. 启动 Local Discovery / Global Discovery / STUN / UPnP / Relay listener
9. 启动 REST API 服务器（Axum）
10. 当对端连接建立时，启动 `BepSession` + `DaemonBepHandler`
11. 连接断开后自动重连

### 7.2 主要运行时组件

| 组件 | crate / 文件 | 职责 |
|------|--------------|------|
| `ConnectionManager` | `syncthing-net` | 维护连接池、拨号、重试、地址评分 |
| `BepSession` | `syncthing-net` | 单条 BEP 连接上的消息收发、心跳 |
| `SyncService` / `SyncManager` | `syncthing-sync` | 文件夹生命周期、扫描/拉取/watcher 调度 |
| `FolderModel` | `syncthing-sync` | 单个文件夹的本地/远程索引与状态 |
| `Puller` | `syncthing-sync` | 块请求、下载、组装文件 |
| `Scanner` | `syncthing-sync` / `syncthing-fs` | 本地文件扫描、SHA-256 哈希 |
| `IndexHandler` | `syncthing-sync` | 处理收到的 Index/IndexUpdate |
| `Versioner` | `syncthing-versioner` | 文件修改/删除前归档到 `.stversions/` |
| `RestApi` | `syncthing-api` | REST 端点、配置热重载、事件流 |
| `TUI` | `cmd/syncthing/src/tui/` | 实时状态、文件夹/设备表单、日志视图 |
| `Tray` | `cmd/syncthing/src/tray.rs` | Windows 托盘图标、右键菜单、daemon 启停 |
| `MCP Bridge` | `cmd/syncthing-mcp-bridge` | MCP stdio JSON-RPC → REST API 桥接 |

### 7.3 默认端口

- BEP：`22001`（避免与 Go Syncthing 默认 `22000` 冲突）
- REST API：`8385`（避免与 Go Syncthing 默认 `8384` 冲突）

### 7.4 关键默认值

- 扫描间隔：`3600` 秒
- 块大小：`128 KiB`
- TCP keepalive：`60s` 间隔 / `10s` 探测间隔
- BEP 心跳：`30s`
- 连接超时：`120s`
- 最大连接数：`1000`
- 日志轮转：按小时，保留 `168` 个文件（7 天）

---

## 8. 安全注意事项

### 8.1 威胁模型内

- BEP 协议解析（长度字段溢出、超大消息）
- TLS 配置（rustls 0.23 + ed25519-dalek）
- 路径遍历：`..`、绝对路径、符号链接逃逸必须被拒绝
- 资源耗尽：连接数、扫描内存、消息大小上限
- REST API 认证：默认 loopback 绑定，API key 鉴权

### 8.2 关键上限

| 参数 | 当前值 | 位置 |
|------|--------|------|
| `MAX_BEP_MESSAGE_SIZE` | 128 MiB | `bep-protocol` |
| `MAX_BEP_HEADER_SIZE` | 64 KiB | `bep-protocol` |
| `max_connections` | 1000 | `daemon_runner.rs` |
| `connection_timeout` | 120s | `daemon_runner.rs` |

### 8.3 接受的审计债务

以下警告已通过 `.cargo/audit.toml` 和 `deny.toml` 显式接受：

| ID | Crate | 路径 | 原因 |
|----|-------|------|------|
| RUSTSEC-2024-0384 | `instant` | `sled → parking_lot → instant` | native 上是 `std::time::Instant` 薄包装 |
| RUSTSEC-2025-0057 | `fxhash` | `sled` 内部 hash table | 无外部输入直接到达 |
| RUSTSEC-2024-0436 | `paste` | `netdev → netlink-packet-core → paste` | 编译期过程宏，运行时暴露为零 |

**禁止**为消除这些警告而引入 breaking change 的依赖升级。

### 8.4 cargo-deny 状态

- `cargo deny check all`（使用 `deny.toml` 或 `cargo-deny.toml`）**已通过**。
- 历史问题：`cmd/syncthing-tray/Cargo.toml` 缺少 `license` 字段且未声明 `publish = false`，导致 cargo-deny 将其视为需许可证的发布 crate。
- 修复方式：已为 `syncthing-tray` 添加 `license = "MIT"` 与 `publish = false`。

### 8.5 部署安全建议

- REST API 默认绑定 `127.0.0.1:8385`，不要暴露到公网
- 将 `config.json` 视为机密（含 API key）
- 同步目录限制在专用目录，不要选系统根目录
- 监控 RSS 增长；`FileSystemDatabase` 当前使用无界内存缓存

---

## 9. 部署与运维

### 9.1 产物

- `target/release/syncthing.exe` / `syncthing`：主守护进程
- `target/release/syncthing-cli`：CLI 工具
- `target/release/syncthing-monitor`：监控工具
- `target/release/syncthing-bench`：基准测试
- `target/release/syncthing-mcp-bridge`：MCP Bridge
- `target/release/syncthing-tray.exe`：托盘薄 wrapper（启动同目录 `syncthing.exe`）

### 9.2 部署脚本

- `scripts/cloud-deploy.sh`：从 Windows 编译并部署到云端 Ubuntu（SCP + systemd）
- `scripts/72h_stress_test.sh` / `72h_monitor.sh` / `72h_report.sh`：长跑测试
- `scripts/check-health.ps1` / `check-sync-consistency.ps1`：健康检查
- `scripts/recover-remote.sh`：对侧格式化/重装后的灾备恢复
- `scripts/two-node-real-network-test.ps1` / `scripts/stop-two-node-test.ps1`：真实网络双节点测试

### 9.3 灾备恢复协议（对侧格式化/重装后）

1. 停止双端 syncthing
2. 删除双端 `db/` 和 `syncthing.pid`
3. 本侧 `git bundle create workspace.bundle --all`
4. SCP → 对侧 `git clone workspace.bundle workspace`
5. 对侧 `systemctl start syncthing`
6. 本侧启动 syncthing
7. 验证：0 error code 3 + scans stable（files_changed=0）

### 9.4 CI / CD

- 平台：GitHub Actions（`.github/workflows/ci.yml`）
- 矩阵：ubuntu-latest / windows-latest / macos-latest
- Job：fmt、clippy、test、audit、cargo-deny、all-features、bench-smoke、release-check、doc-check、e2e-test、file-size（600 行软限制检查）
- **注意**：`README.md` / `CHANGELOG.md` 声称 19/19 jobs passing；本地 `cargo deny check all` 也已通过，CI 状态需以最新 GitHub Actions 运行结果为准。

---

## 10. Agent 修改代码时的关键约束

### 10.1 分层耦合红线

- `syncthing-core` 对下游 crate 是**只读**的：不要在这里加内部 crate 依赖，不要改 public API 而不写 ADR
- `syncthing-api` 禁止直接持有 `syncthing-net` / `syncthing-sync` 具体类型，必须通过 `syncthing-core::traits`
- `cmd/syncthing` 可以依赖所有 crate

### 10.2 文件规模

- `daemon_runner.rs` 禁止继续膨胀；新增网络组件必须拆分为独立模块
- 单文件软上限 600 行；超过需拆分或保持关注

### 10.3 Trait 唯一性

- `syncthing-core::traits::SyncModel` 是 canonical trait
- `syncthing-sync` 内部禁止再定义同名 `SyncModel` trait

### 10.4 测试要求

- 新增功能必须配套集成测试或 E2E 测试
- 网络层改动必须通过 `TestNode` 双实例验证
- 不要仅用 `#[cfg(test)]` 单元测试覆盖端到端行为

### 10.5 禁止事项

- 生产代码 `unwrap()` / `expect()`
- 为消除 cargo audit 警告而引入 breaking change 依赖升级
- 在 `syncthing-db` 暴露 sled 特有 API
- 实现当前冻结项：共识算法、信誉系统、自定义加密、QUIC/MagicSocket、Web GUI

### 10.6 文档同步

修改以下任何内容时，必须同步更新本文档或相关 `.md`：

- crate 边界 / 依赖关系
- 构建 / 测试命令
- CI 配置
- 安全相关上限或债务
- 架构约束

---

## 11. 常用文件索引

| 文件 | 用途 |
|------|------|
| `Cargo.toml` | workspace 定义 |
| `justfile` | 开发命令 |
| `deny.toml` / `cargo-deny.toml` | 依赖审计 |
| `.cargo/audit.toml` | cargo audit 忽略 |
| `docs/plans/INDEX.md` | 计划索引 |
| `docs/plans/POST_V0_2_0_ROADMAP.md` | 权威路线图 |
| `docs/KNOWN_ISSUES.md` | 已知问题与根因 |
| `docs/design/ARCHITECTURE_DECISIONS.md` | 架构决策记录 |
| `SECURITY.md` | 安全策略 |
| `CONTRIBUTING.md` | 贡献指南 |
| `CHANGELOG.md` | 变更日志 |
| `scripts/cloud-deploy.sh` | 云端部署 |
| `cmd/syncthing/src/tui/daemon_runner.rs` | daemon 启动器 |
| `crates/syncthing-core/src/traits/mod.rs` | 核心 trait |
| `crates/syncthing-core/src/constants.rs` | 全局常量 |

---

*最后更新：2026-06-14（基于仓库实际内容实测整理）*
