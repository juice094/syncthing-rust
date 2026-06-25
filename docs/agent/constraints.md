---
type: Policy
title: Agent Constraints and Coding Standards
description: syncthing-rust 的 crate 边界红线、禁止事项、代码风格与开发约定。Agent 修改代码前必须阅读。
resource: ./constraints.md
tags: [agent, constraints, crate-boundary, coding-standards, okf]
status: active
project: syncthing-rust
timestamp: 2026-06-25T00:00:00Z
---

# Agent 约束与代码规范

---

## 1. Crate 边界红线

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

### 1.1 分层耦合红线

- `syncthing-core` 对下游 crate 是**只读**的：不要在这里加内部 crate 依赖，不要改 public API 而不写 ADR。
- `syncthing-api` 禁止直接持有 `syncthing-net` / `syncthing-sync` 具体类型，必须通过 `syncthing-core::traits`。
- `cmd/syncthing` 可以依赖所有 crate，但注意 `cmd/syncthing/Cargo.toml` **不直接依赖** `syncthing-db` 和 `syncthing-fs`；它们通过 `syncthing-sync` 间接消费。

### 1.2 Trait 唯一性

- `syncthing-core::traits::SyncModel` 是 canonical trait。
- `syncthing-sync` 内部禁止再定义同名 `SyncModel` trait。

---

## 2. 禁止事项

- 生产代码使用 `unwrap()` / `expect()`（测试代码除外）。
- 为消除 cargo audit 警告而引入 breaking change 的依赖升级。
- 在 `syncthing-db` 暴露 sled 特有 API。
- 实现当前冻结项：共识算法、信誉系统、自定义加密、QUIC / MagicSocket、Web GUI。

---

## 3. 代码风格

### 3.1 格式化

- 必须运行 `cargo fmt` 后提交。
- 单文件软上限 **600 行**；CI 会检查并警告。
- 当前超过 600 行的生产文件：
  - `cmd/syncthing/src/main.rs`（1843 行）
  - `crates/syncthing-sync/src/scanner.rs`（939 行）
  - `crates/syncthing-sync/src/folder_model/mod.rs`（914 行）
  - `crates/syncthing-sync/src/puller/mod.rs`（863 行）
  - `cmd/syncthing/src/tui/daemon_runner.rs`（658 行）
  - `cmd/syncthing/src/tray.rs`（635 行）

### 3.2 日志级别

| 级别 | 用途 |
|:---|:---|
| `trace` | 块级细节 |
| `debug` | 状态转换 |
| `info` | 生命周期 |
| `warn` | 可恢复错误 |
| `error` | 失败 |

### 3.3 错误处理

- 生产路径优先使用 `thiserror` / `anyhow`。
- 新增错误路径必须记录日志，不能静默吞掉。

### 3.4 异步

- 仅使用 `tokio`，禁止使用 `async-std`。
- 注意 `clippy::await_holding_lock` 警告。

### 3.5 提交规范（Conventional Commits）

```
feat:     新功能
fix:      Bug 修复
docs:     文档
refactor: 重构（无行为变化）
test:     测试
chore:    构建/工具
perf:     性能
```

### 3.6 注释与文档

- 新增 public API 必须写 doc comment。
- 关键常量集中在 `syncthing-core::constants`。
- 中文注释在项目中大量使用；新增代码可继续使用中文注释以保持风格一致。

---

## 4. 文件规模与模块拆分

- `daemon_runner.rs` 禁止继续膨胀；新增网络组件必须拆分为独立模块。
- 新增功能应拆分为独立文件，避免单文件超过 600 行。

---

## 5. 文档同步义务

修改以下任何内容时，必须同步更新本文档或 [`AGENTS.md`](../../AGENTS.md)：

- crate 边界 / 依赖关系
- 构建 / 测试命令
- CI 配置
- 安全相关上限或债务
- 架构约束
