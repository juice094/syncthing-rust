---
type: Index
title: Agent Guidance
description: 面向 AI Agent 的操作约束、测试要求、安全注意事项与运维指引的 OKF bundle 入口。
resource: ./index.md
tags: [agent, guidance, constraints, okf]
status: active
project: syncthing-rust
timestamp: 2026-06-25T00:00:00Z
---

# Agent 操作指引

> 本文档面向 AI 编程 Agent。人类贡献者请同时参考根目录 [`CONTRIBUTING.md`](../../CONTRIBUTING.md)。

本 bundle 将 [`AGENTS.md`](../../AGENTS.md) 中的操作约束拆分为可独立引用的概念文件，方便 Agent 在修改特定子系统时快速定位相关规则。

---

## 核心约束

| 文件 | 内容 |
|------|------|
| [constraints.md](constraints.md) | Crate 边界红线、禁止事项、代码风格、文件规模限制 |
| [testing.md](testing.md) | 测试基线、E2E / TestNode 要求、提交前检查清单 |
| [security.md](security.md) | 威胁模型、关键上限、接受的审计债务、部署安全建议 |
| [operations.md](operations.md) | 构建产物、部署脚本、灾备恢复协议、CI/CD 说明 |

---

## 快速参考

- **绝不做的**：生产代码 `unwrap()` / `expect()`、为消除 cargo audit 警告而引入 breaking change、在 `syncthing-db` 暴露 sled API、实现冻结项（QUIC/MagicSocket/Web GUI/共识/信誉/自定义加密）。
- **修改前必查**：`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock`、`cargo fmt --all -- --check`。
- **网络层改动**：必须通过 `TestNode` 双实例验证；单实例测试视为无效。
- **文档同步义务**：修改 crate 边界、构建/测试命令、CI 配置、安全上限或架构约束时，必须同步更新本 bundle 或 [`AGENTS.md`](../../AGENTS.md)。

---

## 相关概念

- [项目拓扑与架构](../design/topology.md)
- [架构决策记录](../design/ARCHITECTURE_DECISIONS.md)
- [已知问题](../KNOWN_ISSUES.md)
- [工程纪律](../ENGINEERING_DISCIPLINE.md)
