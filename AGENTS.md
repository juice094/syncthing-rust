# AGENTS.md · syncthing-rust

> **本文档是 AI 编程 Agent 的入口页**，只保留"改动前必须知道"的红线与导航。
> 详细约束、测试要求、安全事项、运维与 Claude 专属参考已拆分到 [`docs/agent/`](docs/agent/index.md)。
>
> 面向人类贡献者的文档是 [`CONTRIBUTING.md`](CONTRIBUTING.md) / [`SECURITY.md`](SECURITY.md) / [`README.md`](README.md)。

---

## 1. 项目定位

`syncthing-rust` 是 [Syncthing](https://syncthing.net/) BEP（Block Exchange Protocol）协议的 Rust 实现，目标为 Go 版 Syncthing 的替代实现：零运行时依赖、单静态二进制、与官方 daemon 线格式互通。

| 维度 | 事实 |
|:---|:---|
| 版本真源 | 根 `Cargo.toml` 的 `[workspace.package] version`（当前 `3.0.4`），各 crate 用 `version.workspace = true` 继承 |
| 阶段 | Production（安全加固）；72h 耐久测试是 `v3.1.0` 准入线，尚未完成 |
| 已交付 | BEP over TLS、块级 Pull/Push、三路文本合并、Simple/Staggered 版本控制、多路径发现、自建 Relay Server v1、WSS、RBAC API keys、JSON 结构化日志、Prometheus 指标、TUI、Windows 托盘、MCP Bridge |
| 冻结项 | 共识算法、信誉系统、QUIC / MagicSocket、Web GUI（详见 [`docs/agent/constraints.md`](docs/agent/constraints.md)） |
| 默认端口 | BEP `22001`、REST API `8385`（loopback）、Relay Protocol `22067`、Relay Session `22068` |
| 技术栈 | Tokio + rustls 0.23 + ed25519-dalek + prost + sled + Axum + ratatui；Rust 1.85+ |

架构与运行时拓扑见 [`docs/design/topology.md`](docs/design/topology.md)。

---

## 2. 硬红线

完整版见 [`docs/agent/constraints.md`](docs/agent/constraints.md)；以下是不可协商的最小集：

1. **生产路径禁止 `unwrap()` / `expect()`**（测试代码除外）；fallible 路径必须显式处理并留日志。
2. **crate 边界**：`syncthing-core` 只放 trait / 类型 / 常量，禁止依赖任何内部 crate；`bep-protocol` 禁止直接 I/O；`syncthing-db` 禁止暴露 sled 特有 API。
3. **禁止为实现审计债务而引入 breaking change 的依赖升级**（大版本升级必须先过 CI，见 [`docs/agent/operations.md`](docs/agent/operations.md)）。
4. **安全校验器不可绕过**：远端文件名必须过 `validate_remote_name()`（路径穿越防御）；出站地址必须过 `validate_outbound_addr()`（SSRF 防御）；`Config.max_connections` 在 accept 处强制。
5. **网络层改动必须用 `TestNode` 双实例验证**；单实例测试视为无效。新增端到端行为必须有集成 / E2E 测试，不能只用 `#[cfg(test)]` 单测覆盖。
6. **冻结项不实现**（见上表），除非先落地 ADR 并达成讨论。
7. **新依赖能不加就不加**；新功能优先拆分独立文件，单文件软上限 600 行。

---

## 3. 构建与测试

```bash
cargo build --release            # 正式发行构建（lto=true, codegen-units=1）
cargo build --release --bin syncthing --features tray   # Windows 桌面（含托盘）
cargo build --release --bin syncthing --no-default-features   # 无头/服务器

cargo run --release -- init                                   # 初始化配置与证书
cargo run --release -- run --config-dir ~/.syncthing          # 守护进程
cargo run --release -- tui --config-dir ~/.syncthing          # TUI
cargo run --release -- relay-server --listen :22067           # 自建 Relay
cargo run -p syncthing-cli -- status                          # CLI 状态查询
```

**提交前必须全绿**（等价于 CI 的 fmt / clippy / test / doc / deny 五道门）：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock
cargo test --workspace
cargo doc --no-deps --workspace
cargo deny check all
```

测试基线、E2E 要求与已知偶发用例见 [`docs/agent/testing.md`](docs/agent/testing.md)。常用任务也可用 `just`（`just --list`）。

---

## 4. 常见陷阱

- **连接建立 ≠ BEP 会话就绪**：ClusterConfig 未交换前索引/块传输无人收发。测试中用 `TestNode::wait_for_cluster_config()` / `wait_for_folder_ready()`，不要靠"睡够时间"。
- **拉取失败不做就地重试**：`Puller::pull_folder` 只统计失败数；失败文件由 `FolderModel` 的拉取循环按退避重试，正确性依赖 `check_needed_files()` 重新推导。
- **`.stignore` 目录规则**：增量（watcher）路径必须走 `is_ignored_with_ancestors()`，只匹配文件路径会漏掉目录规则。
- **索引/删除推断与在途拉取有竞态**：不要在没有确认拉取状态时按"DB 有记录但磁盘无文件"推断删除。

---

## 5. 文档地图

| 主题 | 文件 |
|:---|:---|
| 项目拓扑、crate DAG、运行时组件、关键入口 | [`docs/design/topology.md`](docs/design/topology.md) |
| Agent 约束入口（crate 边界、禁止事项、代码风格） | [`docs/agent/index.md`](docs/agent/index.md) → [`constraints.md`](docs/agent/constraints.md) |
| 测试基线与提交前清单 | [`docs/agent/testing.md`](docs/agent/testing.md) |
| 威胁模型、关键上限、审计债务 | [`docs/agent/security.md`](docs/agent/security.md) |
| 构建产物、部署脚本、灾备恢复、CI/CD | [`docs/agent/operations.md`](docs/agent/operations.md) |
| Claude Code 专属参考（生命周期、CLI/TUI/REST 全表） | [`docs/agent/claude_reference.md`](docs/agent/claude_reference.md) |
| 已知问题（权威缺陷登记） | [`docs/KNOWN_ISSUES.md`](docs/KNOWN_ISSUES.md) |
| 架构决策记录 | [`docs/design/ARCHITECTURE_DECISIONS.md`](docs/design/ARCHITECTURE_DECISIONS.md) |
| 计划与路线图索引 | [`docs/plans/INDEX.md`](docs/plans/INDEX.md) |
| BEP 协议 Agent 技能 | [`.agents/skills/bep-protocol/SKILL.md`](.agents/skills/bep-protocol/SKILL.md) |

`docs/KNOWN_ISSUES.md` 是项目范围的**权威缺陷台账**：验证任何"某某已修复"的说法时以它为准；新发现的缺陷必须登记在那里。

---

## 6. 文档同步义务

修改以下任何内容时，必须同步更新 [`docs/agent/`](docs/agent/index.md)（必要时一并更新本文档）：

- crate 边界或 crate 间依赖
- 构建 / 测试 / 校验命令
- CI 配置与 job 组成
- 安全上限、安全校验器、审计债务
- 架构约束与冻结项
