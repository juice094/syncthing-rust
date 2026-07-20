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

当前实测基线：

- `cargo test --workspace`：**404 passed / 6 ignored / 0 failed**
- `cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock`：0 warnings
- `cargo doc --no-deps --workspace`：通过
- `cargo audit`：3 个 unmaintained 上游传递依赖已记录在 `.cargo/audit.toml` 中接受为债务
- `cargo deny check all`：通过

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
- `test_two_node_single_file_sync` 当前因 ClusterConfig race 在并行测试负载下偶发超时，被 `#[ignore]`；生产代码已通过 reconnect 逻辑规避。

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
