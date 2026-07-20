# Agent 环境指引 · syncthing-rust

> 本文档面向 AI 编程 Agent。读者应被假设对项目一无所知。所有信息基于仓库实际内容（`Cargo.toml`、`README.md`、源码、CI、脚本、文档等）实测整理，不做推测。
>
> **详细约束、测试要求、安全注意事项与运维指引已拆分为 [`docs/agent/`](docs/agent/index.md) OKF bundle。**

---

## 1. 项目概述

`syncthing-rust` 是 [Syncthing](https://syncthing.net/) BEP（Block Exchange Protocol）协议的 Rust 实现，目标是成为 Go 版 Syncthing 的替代实现。

- **当前版本**：`3.0.4`（2026-06-27）。版本号唯一真源为根 `Cargo.toml` 的 `[workspace.package] version`，工作区内所有 crate 通过 `version.workspace = true` 继承。
- **当前阶段**：Production（安全加固）。核心 P2P 文件同步、自建 Relay Server、Windows 托盘、TUI、REST API 已稳定；72h 耐久测试为 `v3.1.0` 准入线（尚未完成）。
- **主要特性**：BEP over TLS、块级 Pull/Push 双向同步、三路文本合并、Simple/Staggered 版本控制、多路径发现、Relay Server v1、WSS、RBAC API keys、JSON 结构化日志、Prometheus 指标、实时 TUI、Windows 托盘、Docker/K8s 部署。

---

## 2. 技术栈

| 层面 | 技术 |
|:---|:---|
| 协议 | BEP over TLS（`prost` 编解码 + `lz4` 压缩） |
| 异步运行时 | Tokio 1.35+ |
| TLS / 身份 | rustls 0.23 + ed25519-dalek 2.1 |
| 网络传输 | TCP+TLS、Relay v1、WebSocket/WSS、代理感知传输 |
| 发现 | UDP 广播、HTTPS mTLS 全局发现、STUN、UPnP |
| 存储 | sled 0.34（元数据 + 块缓存抽象） |
| REST API | Axum 0.7 |
| TUI | ratatui 0.30 + crossterm 0.28 |
| CLI | clap 4.5 |
| 日志 | tracing + tracing-subscriber + tracing-appender |
| 构建工具 | Cargo workspace、just |
| Rust 版本 | 1.85.0+（`rust-toolchain.toml` 使用 stable） |

---

## 3. 项目结构与代码组织

### 3.1 顶层目录

```
syncthing-rust/
├── cmd/                          # 可执行入口
│   ├── syncthing/               # 主守护进程 + TUI + 托盘 + stress/monitor 辅助 bin
│   ├── syncthing-cli/           # CLI 工具（generate-cert / show-id / status）
│   ├── syncthing-bench/         # 同步基准测试
│   ├── syncthing-mcp-bridge/    # MCP stdio → REST API Bridge
│   └── syncthing-tray/          # Windows 托盘薄 wrapper
├── crates/                       # 核心库 crate
│   ├── syncthing-core/          # 核心类型与 trait（无内部依赖）
│   ├── bep-protocol/            # BEP 编解码 + 握手（无直接 I/O）
│   ├── syncthing-net/           # TCP+TLS、ConnectionManager、发现、Relay
│   ├── syncthing-sync/          # Scanner、Puller、IndexHandler、FolderModel
│   ├── syncthing-fs/            # 文件系统抽象、.stignore、watcher
│   ├── syncthing-db/            # 元数据与块缓存（sled-backed）
│   ├── syncthing-api/           # REST API（Axum）+ 事件总线 + 配置
│   ├── syncthing-versioner/     # Simple / Staggered 版本控制
│   └── syncthing-test-utils/    # MemoryPipe、TestNode 等测试辅助
├── docs/                         # 设计文档、Agent 指引、计划、报告
├── scripts/                      # 健康检查、cloud-deploy、压力测试
├── Cargo.toml                    # Workspace 定义
├── justfile                      # 常用开发命令
├── deny.toml / cargo-deny.toml   # 依赖审计
├── .cargo/audit.toml             # cargo audit 忽略
├── AGENTS.md                     # 本文档
└── CLAUDE.md                     # Claude Code 专属指引
```

### 3.2 Crate 依赖 DAG

```
cmd/syncthing
  ├─ syncthing-api
  ├─ syncthing-net
  ├─ syncthing-sync ─┬─ syncthing-db  （通过 sync 内部消费）
  │                  └─ syncthing-fs  （通过 sync 内部消费）
  ├─ syncthing-core
  ├─ syncthing-versioner
  ├─ bep-protocol
  └─ syncthing-test-utils   # 仅测试 / 开发工具

syncthing-core ← {protocol, net, sync, fs, db, api, versioner}
bep-protocol   ← {net, sync, test-utils}
versioner      ← {sync}
net            ← {sync, test-utils, cli}
```

---

## 4. 核心红线（必读）

### 4.1 Crate 边界

| Crate | 职责 | 禁止做的事 |
|:---|:---|:---|
| `syncthing-core` | trait + 类型 + 常量 | 禁止依赖任何内部 crate；禁止放置具体实现 |
| `bep-protocol` | 协议编解码 | 禁止直接 I/O |
| `syncthing-net` | 网络传输、连接管理、发现 | 禁止同步逻辑 |
| `syncthing-sync` | 扫描、拉取、索引处理、状态机 | 禁止直接处理 wire format |
| `syncthing-fs` | 文件系统抽象、扫描、监控 | 禁止同步状态机逻辑 |
| `syncthing-db` | 存储后端 | 禁止暴露 sled 特有 API；同步逻辑应通过 `BlockStore` trait |
| `syncthing-api` | REST + 事件总线 + 配置 | 禁止直接持有 `ConnectionManagerHandle` / `LocalDatabase` 具体类型；必须走 trait |
| `syncthing-versioner` | 文件版本归档策略 | 禁止 FS I/O |
| `syncthing-test-utils` | 测试辅助（`MemoryPipe`、`TestNode`） | 仅用于测试 / 开发工具 |

### 4.2 绝对禁止

- 生产代码使用 `unwrap()` / `expect()`（测试代码除外）。
- 为消除 cargo audit 警告而引入 breaking change 的依赖升级。
- 在 `syncthing-db` 暴露 sled 特有 API。
- 实现当前冻结项：共识算法、信誉系统、自定义加密、QUIC / MagicSocket、Web GUI。

### 4.3 文档同步义务

修改 crate 边界、构建/测试命令、CI 配置、安全上限/债务、架构约束时，必须同步更新 [`docs/agent/`](docs/agent/index.md) 或本文档。

---

## 5. 构建与运行

### 5.1 常用命令

```bash
# 编译 release 二进制
cargo build --release

# 交互式初始化向导（生成配置、证书）
cargo run --release -- init

# 启动守护进程
cargo run --release -- run --config-dir ~/.syncthing

# 启动 TUI
cargo run --release -- tui --config-dir ~/.syncthing

# 启动托盘（Windows）
cargo run --release --bin syncthing-tray

# CLI 查询状态
cargo run -p syncthing-cli -- status
```

### 5.2 just 命令

项目提供 `justfile`，推荐通过 `just` 执行常规任务：

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
just build-release       # 编译 release 二进制
```

### 5.3 构建产物

| 产物 | 说明 |
|:---|:---|
| `target/release/syncthing.exe` / `syncthing` | 主守护进程 |
| `target/release/syncthing-cli` | CLI 工具 |
| `target/release/syncthing-monitor` | 监控工具 |
| `target/release/syncthing-bench` | 基准测试 |
| `target/release/syncthing-mcp-bridge` | MCP Bridge |
| `target/release/syncthing-tray.exe` | Windows 托盘薄 wrapper |

### 5.4 Profile

- `release`：`lto = true`、`codegen-units = 1`、`opt-level = 3`（正式发行）
- `release-thin`：`lto = "thin"`、`codegen-units = 16`（开发期快速验证）
- `bench`：`debug = true`（给 criterion 生成 flamegraph 用）

### 5.5 默认端口

- BEP：`22001`（避免与 Go Syncthing 默认 `22000` 冲突）
- REST API：`8385`（避免与 Go Syncthing 默认 `8384` 冲突）

---

## 6. 代码风格与开发约定

### 6.1 格式化

- 必须运行 `cargo fmt` 后提交。
- 单文件软上限 **600 行**；CI 会检查并警告。
- 当前超过 600 行的生产文件：
  - `cmd/syncthing/src/main.rs`（1843 行）
  - `crates/syncthing-sync/src/scanner.rs`（939 行）
  - `crates/syncthing-sync/src/folder_model/mod.rs`（914 行）
  - `crates/syncthing-sync/src/puller/mod.rs`（863 行）
  - `cmd/syncthing/src/tui/daemon_runner.rs`（658 行）
  - `cmd/syncthing/src/tray.rs`（635 行）

### 6.2 日志级别

| 级别 | 用途 |
|:---|:---|
| `trace` | 块级细节 |
| `debug` | 状态转换 |
| `info` | 生命周期 |
| `warn` | 可恢复错误 |
| `error` | 失败 |

### 6.3 错误处理

- 生产路径优先使用 `thiserror` / `anyhow`。
- 新增错误路径必须记录日志，不能静默吞掉。

### 6.4 异步

- 仅使用 `tokio`，禁止使用 `async-std`。
- 注意 `clippy::await_holding_lock` 警告。

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

### 6.6 注释与文档

- 新增 public API 必须写 doc comment。
- 关键常量集中在 `syncthing-core::constants`。
- 中文注释在项目中大量使用；新增代码可继续使用中文注释以保持风格一致。

---

## 7. 测试策略

### 7.1 测试基线

当前实测基线：

- `cargo test --workspace`：**414 passed / 6 ignored / 0 failed**（2026-07-20 实测）
- `cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock`：0 warnings
- `cargo doc --no-deps --workspace`：通过
- `cargo audit`：3 个 unmaintained 上游传递依赖已记录在 `.cargo/audit.toml` 中接受为债务
- `cargo deny check all`：通过

### 7.2 测试组织

| 类型 | 位置 | 说明 |
|:---|:---|:---|
| 单元测试 | `src/*.rs` / `src/*/tests.rs` | 各 crate 内部模块测试 |
| 集成测试 | `crates/*/tests/*.rs` | 如 `bep-protocol/tests/wire_compat.rs` |
| E2E 测试 | `cmd/syncthing/tests/e2e_*.rs` | 双节点真实同步链路 |
| Benchmark | `crates/*/benches/*.rs` | criterion：`device_id`、`encode_decode`、`scanner`、`hash_parallel`、`puller` |
| 压力测试 | `cmd/syncthing/src/bin/stress_test.rs` + `cmd/syncthing/src/bin/monitor.rs` | 72h 耐久测试基础设施 |

### 7.3 E2E / 网络测试要求

- 网络层改动必须通过 `TestNode` 双实例验证；单实例测试视为无效。
- 新增端到端行为必须配套集成测试或 E2E 测试，禁止仅用 `#[cfg(test)]` 单元测试覆盖。
- `test_two_node_single_file_sync` 当前因 ClusterConfig race 在并行测试负载下偶发超时，被 `#[ignore]`；生产代码已通过 reconnect 逻辑规避。

### 7.4 提交前检查清单

修改代码后必须运行：

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock
cargo fmt --all -- --check
cargo doc --no-deps --workspace
```

---

## 8. 安全注意事项

### 8.1 威胁模型（范围内）

- BEP 协议解析（长度字段溢出、超大消息）
- TLS 配置（rustls 0.23 + ed25519-dalek）
- 路径遍历：`..`、绝对路径、符号链接逃逸必须被拒绝
- 资源耗尽：连接数、扫描内存、消息大小上限
- REST API 认证：默认 loopback 绑定，API key 鉴权
- SSRF：出站地址过滤（multicast / unspecified / link-local）

### 8.2 关键上限

| 参数 | 当前值 | 位置 |
|:---|:---|:---|
| `MAX_BEP_MESSAGE_SIZE` | 128 MiB | `bep-protocol` |
| `MAX_BEP_HEADER_SIZE` | 64 KiB | `bep-protocol` |
| `max_connections` | 1000 | `daemon_runner.rs` |
| `connection_timeout` | 120s | `daemon_runner.rs` |

### 8.3 接受的审计债务

以下警告已通过 `.cargo/audit.toml` 和 `deny.toml` 显式接受：

| ID | Crate | 路径 | 原因 |
|:---|:---|:---|:---|
| RUSTSEC-2024-0384 | `instant` | `sled → parking_lot → instant` | native 上是 `std::time::Instant` 薄包装 |
| RUSTSEC-2025-0057 | `fxhash` | `sled` 内部 hash table | 无外部输入直接到达 |
| RUSTSEC-2024-0436 | `paste` | `netdev → netlink-packet-core → paste` | 编译期过程宏，运行时暴露为零 |

**禁止**为消除这些警告而引入 breaking change 的依赖升级。

### 8.4 部署安全建议

- REST API 默认绑定 `127.0.0.1:8385`，不要暴露到公网。
- 将 `config.json` 视为机密（含 API key）。
- 同步目录限制在专用目录，不要选系统根目录。
- 监控 RSS 增长；`FileSystemDatabase` 当前使用无界内存缓存。
- 私钥文件（`cert.pem`、`key.pem`、`config.json`）在 Unix 上已加固为 `0o600`。

---

## 9. 部署与运维

### 9.1 部署脚本

- `scripts/cloud-deploy.sh`：从 Windows 编译并部署到云端 Ubuntu（SCP + systemd）
- `scripts/72h_stress_test.sh` / `72h_monitor.sh` / `72h_report.sh`：长跑测试
- `scripts/check-health.ps1` / `check-sync-consistency.ps1`：健康检查
- `scripts/recover-remote.sh`：对侧格式化/重装后的灾备恢复
- `scripts/two-node-real-network-test.ps1` / `scripts/stop-two-node-test.ps1`：真实网络双节点测试

### 9.2 灾备恢复协议（对侧格式化/重装后）

1. 停止双端 syncthing
2. 删除双端 `db/` 和 `syncthing.pid`
3. 本侧 `git bundle create workspace.bundle --all`
4. SCP → 对侧 `git clone workspace.bundle workspace`
5. 对侧 `systemctl start syncthing`
6. 本侧启动 syncthing
7. 验证：0 error code 3 + scans stable（files_changed=0）

### 9.3 CI / CD

- 平台：GitHub Actions（`.github/workflows/ci.yml` + `.github/workflows/release.yml`）
- 矩阵：ubuntu-latest / windows-latest / macos-latest
- CI job（`.github/workflows/ci.yml`）：fmt、clippy（×3 OS）、test（×3 OS）、audit、cargo-deny、all-features、bench-smoke、release-check（×3 OS）、doc-check、e2e-test、file-size（600 行软限制检查）—— 11 个 job 定义，矩阵展开后 17 个运行实例
- Release job（`.github/workflows/release.yml`）：build-linux、build-macos、build-windows —— 3 个运行实例
- **注意**：`README.md` / `CHANGELOG.md` 声称 19/19 jobs passing（17 CI + 2 release）；本地 `cargo deny check all` 也已通过，CI 状态需以最新 GitHub Actions 运行结果为准。

### 9.4 关键默认值

- 扫描间隔：`3600` 秒
- 块大小：`128 KiB`
- TCP keepalive：`60s` 间隔 / `10s` 探测间隔
- BEP 心跳：`30s`
- 连接超时：`120s`
- 最大连接数：`1000`
- 日志轮转：按小时，保留 `168` 个文件（7 天）

---

## 10. 快速入口

| 主题 | 文件 |
|:---|:---|
| 项目拓扑、Crate DAG、运行时架构、关键入口 | [`docs/design/topology.md`](docs/design/topology.md) |
| Agent 约束（crate 边界、禁止事项、代码风格） | [`docs/agent/constraints.md`](docs/agent/constraints.md) |
| 测试策略与提交前检查清单 | [`docs/agent/testing.md`](docs/agent/testing.md) |
| 安全注意事项与审计债务 | [`docs/agent/security.md`](docs/agent/security.md) |
| 构建产物、部署脚本、灾备恢复、CI/CD | [`docs/agent/operations.md`](docs/agent/operations.md) |
| 计划与路线图索引 | [`docs/plans/INDEX.md`](docs/plans/INDEX.md) |
| 已知问题与项目阶段定位 | [`docs/KNOWN_ISSUES.md`](docs/KNOWN_ISSUES.md) |
| 架构决策记录 | [`docs/design/ARCHITECTURE_DECISIONS.md`](docs/design/ARCHITECTURE_DECISIONS.md) |

---

## 11. 常用文件索引

| 文件 | 用途 |
|:---|:---|
| `Cargo.toml` | workspace 定义 |
| `justfile` | 开发命令 |
| `deny.toml` / `cargo-deny.toml` | 依赖审计 |
| `.cargo/audit.toml` | cargo audit 忽略 |
| `docs/design/topology.md` | 项目拓扑与架构 |
| `docs/agent/index.md` | Agent 指引 bundle 入口 |
| `docs/plans/INDEX.md` | 计划索引 |
| `docs/plans/POST_V3_0_3_ROADMAP.md` | 当前后续计划 |
| `docs/KNOWN_ISSUES.md` | 已知问题与根因 |
| `docs/design/ARCHITECTURE_DECISIONS.md` | 架构决策记录 |
| `SECURITY.md` | 安全策略 |
| `CONTRIBUTING.md` | 贡献指南 |
| `CHANGELOG.md` | 变更日志 |
| `cmd/syncthing/src/tui/daemon_runner.rs` | daemon 启动器 |
| `crates/syncthing-core/src/traits/mod.rs` | 核心 trait |
| `crates/syncthing-core/src/constants.rs` | 全局常量 |

---

*最后更新：2026-07-20（版本号统一至 `[workspace.package]`，修正 release workflow job 数量描述）*
