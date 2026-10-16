---
type: Policy
title: Testing Strategy and Pre-Submit Checklist
description: syncthing-rust 的测试基线、测试组织、E2E / TestNode 要求与提交前检查清单。
resource: ./testing.md
tags: [agent, testing, e2e, test-node, checklist, okf]
status: active
project: syncthing-rust
timestamp: 2026-06-25T00:00:00Z
---

# 测试策略与提交前检查

---

## 1. 测试基线

当前实测基线（2026-10-16，`cargo test --workspace`）：

- `cargo test --workspace`：**452 passed / 7 ignored / 0 failed**（38 个测试二进制，含 doc-test）
- `cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock`：0 warnings
- `cargo fmt --all -- --check`：通过
- `cargo doc --no-deps --workspace`：通过
- `cargo deny check all`：通过（advisories / bans / licenses / sources 全 ok）
- `cargo audit`：3 个 unmaintained 上游传递依赖已记录在 `.cargo/audit.toml` 中接受为债务

> 数值口径：本文件是测试基线的**唯一事实源**，其他文档不要手抄（`cargo fmt` 与 CI 的 `file-size` 等
> 易漂移数值同理，可用 `pwsh scripts/agent-facts.ps1` 只读导出）。

---

## 2. 测试组织

| 类型 | 位置 | 说明 |
|:---|:---|:---|
| 单元测试 | `src/*.rs` / `src/*/tests.rs` | 各 crate 内部模块测试 |
| 集成测试 | `crates/*/tests/*.rs` | 如 `bep-protocol/tests/wire_compat.rs` |
| E2E 测试 | `cmd/syncthing/tests/e2e_*.rs` | 双节点真实同步链路 |
| Benchmark | `crates/*/benches/*.rs` | criterion：`device_id`、`encode_decode`、`scanner`、`hash_parallel`、`puller` |
| 压力测试 | `cmd/syncthing/src/bin/stress_test.rs` + `cmd/syncthing/src/bin/monitor.rs` | 72h 耐久测试基础设施 |

### 2.1 E2E / 网络测试要求

- 网络层改动必须通过 `TestNode` 双实例验证；单实例测试视为无效。
- 新增端到端行为必须配套集成测试，禁止仅用 `#[cfg(test)]` 单元测试覆盖。
- **握手就绪必须显式等待**，不要用固定 sleep：连接建立 ≠ BEP 会话就绪。用
  `TestNode::wait_for_cluster_config(peer_id, timeout)`（对端 ClusterConfig 已收到）与
  `TestNode::wait_for_folder_ready(peer_id, folder_id, timeout)`（对端已通告共享该 folder）。
  这两个原语由 `crates/syncthing-test-utils/src/bep_bridge.rs` 的 `SessionReadiness` 事件泵驱动
  （生产侧同源实现见 `cmd/syncthing/src/tui/session_logger.rs`）。
- `test_two_node_single_file_sync` 因 §1 ClusterConfig 竞态在负载下偶发超时，被 `#[ignore]`；
  单独运行时可用 `-- --ignored` 手动验证。
- `e2e_crud.rs` 的断言统一走 `wait_synced()`（**不做断连重拨**：重拨会制造无连接窗口打断在途块请求，
  块请求自身已在 `bep_bridge` 中带 3×5s 重试）；超时会打印双方 `connected_devices()`。
  该测试集属**负载相关偶发**；`test_e2e_modify_file` 在套件条件下顽固失败（单独运行 6/6 通过），已按兜底方案加 `#[ignore]`，
  原因写在用例属性里。手动验证：`cargo test -p syncthing --test e2e_crud test_e2e_modify_file -- --ignored --nocapture`
  （见 [`KNOWN_ISSUES.md §23`](../KNOWN_ISSUES.md)、[§24](../KNOWN_ISSUES.md)）。
- 跨进程契约（如 MCP Bridge ↔ REST）用**真实二进制**做集成测试：`cmd/syncthing-mcp-bridge/tests/mcp_bridge.rs`
  启动进程内 stub HTTP 服务端，断言 stdio JSON-RPC 的请求落点与头部转发，不引入 HTTP 框架依赖。

---

## 3. 提交前检查清单

修改代码后必须运行：

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock
cargo fmt --all -- --check
cargo doc --no-deps --workspace
```

---

## 4. just 命令（推荐）

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

---

## 5. Profile

- `release`：`lto = true`、`codegen-units = 1`、`opt-level = 3`（正式发行）
- `release-thin`：`lto = "thin"`、`codegen-units = 16`（开发期快速验证）
- `bench`：`debug = true`（给 criterion 生成 flamegraph 用）
