---
name: bep-protocol
description: P2P file synchronization via BEP protocol (syncthing-rust). Use when working with syncthing-rust BEP messages, TCP+TLS transport, relay/discovery, the MCP Bridge, or when you need the repo's build/test commands and authoritative fact sources. Covers Rust↔Go interop, handshake/ClusterConfig flow, and cross-instance verification.
---

# BEP Protocol Skill (syncthing-rust)

> 事实口径以仓库内文档为准，本文件只做**操作入口**，不复制易漂移的数值：
> - 测试基线 / 提交前检查：[`docs/agent/testing.md`](../../../docs/agent/testing.md)
> - Agent 硬红线：[`AGENTS.md`](../../../AGENTS.md)、[`docs/agent/constraints.md`](../../../docs/agent/constraints.md)
> - 权威缺陷台账：[`docs/KNOWN_ISSUES.md`](../../../docs/KNOWN_ISSUES.md)
> - 架构与 crate DAG：[`docs/design/topology.md`](../../../docs/design/topology.md)

## Quick Commands

```bash
cargo test --workspace                     # 全量测试（基线数值见 docs/agent/testing.md）
cargo test -p syncthing --test e2e_crud    # 双节点 CRUD / .stignore / 重命名（单独跑才稳定）
cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock
cargo fmt --all -- --check
cargo deny check all
```

```powershell
pwsh scripts/agent-facts.ps1   # 只读导出易漂移事实：版本 / 端口 / MCP 工具数 / CI job / 超 600 行文件
```

版本号唯一真源是根 `Cargo.toml` 的 `[workspace.package] version`（不要在文档里手抄版本号）。
默认端口：BEP `22001`、REST API `8385`（loopback）、Relay Protocol `22067`、Relay Session `22068`。

## BEP Messages

| Message | Direction | Notes |
|---------|-----------|-------|
| `Hello` | bidirectional | Device ID + name + capabilities，TLS 之上的首个交换 |
| `ClusterConfig` | bidirectional | Folders + devices；`WireFolder.label` 是 `String`（非 `repeated`） |
| `Index` / `IndexUpdate` | bidirectional | 文件元数据；删除项必须清空 `blocks` |
| `Request` / `Response` | bidirectional | 块级传输（按 offset/size/hash 拉取） |

握手顺序：TLS → `Hello` → `ClusterConfig`（双向完成才算会话就绪）→ `Index` → steady-state。
**注意**：TCP 连接建立 ≠ BEP 会话就绪。测试里用 `TestNode::wait_for_cluster_config()` /
`wait_for_folder_ready()` 等待 ClusterConfig 完成，不要用固定 sleep。

## MCP Bridge

```
Agent (Trae / Claude / Kimi) ─ MCP stdio ─ syncthing-mcp-bridge ─ HTTP REST ─ syncthing daemon
```

- **进程**：`cmd/syncthing-mcp-bridge`（手写 JSON-RPC 2.0，零第三方 MCP SDK 依赖；工具名册在 `src/tools.rs`）
- **环境变量**：`SYNCTHING_API_URL`（默认 `http://127.0.0.1:8385`）、`SYNCTHING_API_KEY`（可选，转成 `X-API-Key`；loopback 免鉴权）
- **工具 19 个 + 资源 3 个**：完整清单见 [`docs/design/AGENT_INTEGRATION.md`](../../../docs/design/AGENT_INTEGRATION.md) §2
- **集成测试**：`cmd/syncthing-mcp-bridge/tests/mcp_bridge.rs`（进程内 stub HTTP 服务端）

## Verification Discipline

- 网络层改动**必须**用 `TestNode` 双实例验证；单实例测试无效。
- 新增端到端行为必须有集成 / E2E 测试；不能只用 `#[cfg(test)]` 单测覆盖。
- 已发现的坑（动手前先读）：
  - `test_two_node_single_file_sync` 因 §1 ClusterConfig 竞态被 `#[ignore]`；
  - `e2e_crud` 属负载相关偶发（单独跑稳定，和多测试二进制并发时可能超时）——§23 / §24；
  - CI 的 `e2e-test` job 目前只做 release 编译，不执行断言——§24。
- 禁止实现冻结项：共识算法、信誉系统、QUIC / MagicSocket、Web GUI、替代 BEP over TLS 的自定义加密。
