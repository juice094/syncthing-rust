# Agent 环境指引 · syncthing-rust

> 本文档面向 AI 编程 Agent。读者应被假设对项目一无所知。所有信息基于仓库实际内容（`Cargo.toml`、`README.md`、源码、CI、脚本、文档等）实测整理，不做推测。
>
> **详细约束、测试要求、安全注意事项与运维指引已拆分为 [`docs/agent/`](docs/agent/index.md) OKF bundle。**

---

## 1. 项目概述

`syncthing-rust` 是 [Syncthing](https://syncthing.net/) BEP（Block Exchange Protocol）协议的 Rust 实现，目标是成为 Go 版 Syncthing 的替代实现。

- **当前版本**：`CHANGELOG.md` 声明为 `3.0.3`（2026-06-13），最新 Git 标签为 `v3.0.3`，工作区内所有 crate 的 `Cargo.toml` 版本统一为 `3.0.0`。
- **当前阶段**：Production（谨慎生产）。核心 P2P 文件同步、Windows 托盘、TUI、REST API 已稳定运行；72h 耐久测试为 `v3.1.0` 准入线（尚未完成）。
- **主要特性**：BEP over TLS、块级 Pull/Push 双向同步、三路文本合并冲突解决、Simple/Staggered 版本控制、多路径发现、实时 TUI、Windows 托盘、REST API、与 Go Syncthing 线路兼容。

---

## 2. 快速入口

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

## 3. 核心红线（必读）

### 3.1 Crate 边界

| Crate | 职责 | 禁止做的事 |
|:---|:---|:---|
| `syncthing-core` | trait + 类型 + 常量 | 禁止依赖任何内部 crate；禁止放置具体实现 |
| `bep-protocol` | 协议编解码 | 禁止直接 I/O |
| `syncthing-net` | 网络传输、连接管理、发现 | 禁止同步逻辑 |
| `syncthing-sync` | 扫描、拉取、索引处理、状态机 | 禁止直接处理 wire format |
| `syncthing-fs` | 文件系统抽象、扫描、监控 | 禁止同步状态机逻辑 |
| `syncthing-db` | 存储后端 | 禁止暴露 sled 特有 API |
| `syncthing-api` | REST + 事件总线 + 配置 | 禁止直接持有 `ConnectionManagerHandle` / `LocalDatabase` 具体类型；必须走 trait |
| `syncthing-versioner` | 文件版本归档策略 | 禁止 FS I/O |
| `syncthing-test-utils` | 测试辅助 | 仅用于测试 / 开发工具 |

### 3.2 绝对禁止

- 生产代码使用 `unwrap()` / `expect()`（测试代码除外）。
- 为消除 cargo audit 警告而引入 breaking change 的依赖升级。
- 在 `syncthing-db` 暴露 sled 特有 API。
- 实现当前冻结项：共识算法、信誉系统、自定义加密、QUIC / MagicSocket、Web GUI。

### 3.3 修改前必查

```bash
cargo test --workspace                        # 392 passed / 6 ignored / 0 failed
cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock
cargo fmt --all -- --check
cargo doc --no-deps --workspace
```

### 3.4 测试硬要求

- 网络层改动必须通过 `TestNode` 双实例验证；单实例测试视为无效。
- 新增端到端行为必须配套集成测试或 E2E 测试，禁止仅用 `#[cfg(test)]` 单元测试覆盖。

### 3.5 文档同步义务

修改 crate 边界、构建/测试命令、CI 配置、安全上限/债务、架构约束时，必须同步更新 [`docs/agent/`](docs/agent/index.md) 或本文档。

---

## 4. 常用文件索引

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

*最后更新：2026-06-25（精简为快速参考卡，详细约束迁移至 docs/agent/ OKF bundle）*
