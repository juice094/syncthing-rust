---
type: report
status: completed
project: syncthing-rust
date: 2026-05-12
tags: [report, audit, safety, rust]
---

# Unwrap/Expect Audit Update — 2026-05-12

## 总结

**T-F2 Batch 3-6 整体完成**（合并执行）

经过正确分类（区分生产代码 vs `tests.rs`/`#[cfg(test)]` 块），
工作区**生产代码** unwrap/expect 数量为：

| 状态 | 数量 |
|------|------|
| 修复前（2026-05-11 错误统计）| ~735 |
| 修复前（正确统计）| 22 |
| **修复后（2026-05-12）** | **15**（全部为 `.expect("...")` 文档化）|

## 修复明细

### 已修复的运行时风险点（7 个）

| 文件 | 修复方式 |
|------|---------|
| `crates/syncthing-test-utils/src/lib.rs:109-110` | `parse().unwrap()` → `SocketAddr::new(...)` 构造器 |
| `crates/syncthing-net/src/connection.rs:214,217` | 同上 |
| `crates/syncthing-net/src/derp/server.rs:40` | 同上 |
| `crates/syncthing-net/src/manager/mod.rs:238` | 同上 |
| `crates/syncthing-net/src/dialer.rs:316` | `relay_url.as_ref().unwrap()` → `let Some(...) else continue` |
| `crates/syncthing-net/src/tcp_transport.rs:432,435` | `Option::unwrap()` → `ok_or_else(SyncthingError)` |
| `crates/syncthing-db/src/store.rs:98` | `cache_capacity` → `cache_capacity.max(1)` 避免零值 |

### 转为文档化 `.expect("...")`（保留但语义明确）

- `manager/mod.rs:163` → "just set above"（同函数内刚 set）
- `manager/{events,dialer,mod}.rs:×6` → "self_weak set during ConnectionManager::new"
- `connection.rs:464,604` → "read/write_half already taken"
- `manager/events.rs:17` → "event receiver already taken"
- `block_cache/lru.rs:15` → "max(1) guarantees non-zero"
- `store.rs:80,98` → "1024 is non-zero" / "max(1) guarantees non-zero"
- `scanner/hash.rs:28` → "hash thread pool"

## 关键发现

### 原始审计误差
2026-05-11 的 `UNWRAP_AUDIT_2026-05-11.md` 报告了 ~735 unwrap，
但当时**未区分 `tests.rs` 文件与生产代码**。T-E1 文件拆分将
大量 `#[cfg(test)] mod tests {}` 内联测试移到独立 `tests.rs` 文件，
这些文件的 unwrap 仍是测试代码（运行时不会 panic 影响生产）。

正确分类后：
- 生产代码 (`crates/*/src/*.rs`，不含 `tests.rs`/`tests/`): 22
- 测试代码 (`tests.rs`、`#[cfg(test)] mod tests`): 700

### 设计原则

新代码遵循以下规则：

1. **优先用类型系统消除 Option/Result**（如用构造器替代 parse）
2. **不可避免的 unwrap 必须升级为 `.expect("invariant doc")`**
3. **可能因配置/输入触发的失败 → `?` + 类型化错误**
4. **不变量违反时使用 `.expect()`**（panic 时有清晰诊断）

## 关联 PR/Commit

- `0d3e8bb` fix(net): DashMap 跨 await 持锁死锁修复（间接发现）
- 本次提交：T-F2 unwrap 审计 + 修复

## 状态

- ✅ Batch 3-6 合并执行完毕
- ✅ 所有变更通过 86 + 40 个单测
- ✅ 生产代码 zero `.unwrap()`，仅保留文档化 `.expect()`
- ⏳ 72h 压测进行中（验证修复的稳定性）
