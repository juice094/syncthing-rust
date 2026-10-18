# CLAUDE.md

> Claude Code 专用入口。**完整约束以 [`AGENTS.md`](AGENTS.md) 与 [`docs/agent/`](docs/agent/index.md) 为准**，
> 本文件只做 Claude 侧的使用提示与导航。

---

## 项目一句话

`syncthing-rust` 是 Syncthing BEP 协议的 Rust 实现（v3.0.5，Production 阶段，数据丢失防护加固）：零运行时依赖、单静态二进制、与 Go 版 Syncthing 线格式互通。日常部署在 ROG-X（Windows 11）↔ Gray-Cloud（Ubuntu 24.04），以自建 Relay 替代 Tailscale。

---

## 动手前

1. 读 [`AGENTS.md`](AGENTS.md) 的「硬红线」与「常见陷阱」两节。
2. 改网络层 / 协议层前读 [`docs/agent/constraints.md`](docs/agent/constraints.md)；改安全相关路径前读 [`docs/agent/security.md`](docs/agent/security.md)。
3. 任何"已修复 / 已知问题"的判断以 [`docs/KNOWN_ISSUES.md`](docs/KNOWN_ISSUES.md) 为准（它是权威台账；新缺陷必须登记）。

## 提交前

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock
cargo test --workspace
cargo doc --no-deps --workspace
cargo deny check all
```

基线数值、E2E 双实例要求与偶发用例说明见 [`docs/agent/testing.md`](docs/agent/testing.md)。

## 导航

| 需求 | 入口 |
|:---|:---|
| 构建矩阵、feature flag、跨平台编译 | [`docs/agent/claude_reference.md`](docs/agent/claude_reference.md) |
| daemon 生命周期、进程内组件接线 | [`docs/design/topology.md`](docs/design/topology.md) |
| CLI 命令 / TUI 按键 / REST 路由全表 | [`docs/agent/claude_reference.md`](docs/agent/claude_reference.md) |
| 构建产物、部署与灾备恢复、CI/CD | [`docs/agent/operations.md`](docs/agent/operations.md) |
| BEP 协议 Agent 技能（命令速查、消息表、MCP Bridge） | [`.agents/skills/bep-protocol/SKILL.md`](.agents/skills/bep-protocol/SKILL.md) |
