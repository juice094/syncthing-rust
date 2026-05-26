---
type: report
status: completed
project: syncthing-rust
date: 2026-05-12
tags: [report, audit, concurrency, safety]
---

# Cross-Await Lock Holding Audit — 2026-05-12

## 背景

修复 T-F1 死锁后（commit `0d3e8bb`），全面审查工程中是否存在
类似的同步锁跨 `.await` 持有模式，避免未来出现相同问题。

## 审查范围

排除 `tests.rs` 和 `#[cfg(test)]` 块（只关注生产代码）：
- DashMap `get/get_mut/entry/iter` 操作
- parking_lot `read()/write()` 操作  
- std::sync::Mutex 操作

## 审查结果

### DashMap 持锁跨 await：✅ 已全部修复

| 文件 | 行 | 状态 |
|------|----|----|
| `manager/registry.rs:28-67` | 修复 | T-F1 修复，引入 RegisterAction enum |
| `manager/registry.rs:114-133` | 假阳性 | match arm 中 get_mut 在 await 前 drop |
| `manager/registry.rs:223-234` | 假阳性 | if-let 中 get_mut 作用域结束于 await 前 |
| `database.rs:114-121` | 假阳性 | delete_file 内 get_mut 不跨函数 |
| `database.rs:318-331` | 假阳性 | 同上 |
| `database.rs:97-108` | 假阳性 | update_files 调用 await 但 ref 在 callee 内 |

### parking_lot 持锁跨 await：✅ 无问题

| 文件 | 行 | 状态 |
|------|----|----|
| `dialer.rs:352` | 安全模式 | `Arc::clone(&*self.connector.read())` 立即 drop guard |

### std::sync::Mutex 跨 await：✅ 无问题

仅 `supervisor.rs:255` 在测试代码中使用，生产代码 0 处。

## 安全模式总结

### ✅ 正确模式 1: 立即克隆出锁
```rust
let connector = Arc::clone(&*self.field.read());  // guard 立即 drop
connector.do_async().await;                       // 持有 Arc，不持锁
```

### ✅ 正确模式 2: 显式作用域释放锁
```rust
let value = {
    let guard = self.field.read();
    guard.compute_something()
};  // guard 在此 drop
do_async(value).await;
```

### ✅ 正确模式 3: T-F1 修复模式（决策 + 行动分离）
```rust
let (action, data) = {
    let guard = self.dashmap.get(&key);
    // ... 读取并决策
    (decide_action(), Arc::clone(&guard.value().shared))
};  // guard 在此 drop
match action {
    Action::Do => data.async_method().await,  // 已在锁外
}
```

### ❌ 危险模式：跨 await 持有同步锁
```rust
if let Some(mut guard) = self.dashmap.get_mut(&key) {
    guard.modify_something().await;  // ← 持锁 await，死锁！
}
```

## 后续防护

### 已实施
- `T-F1` 修复（commit `0d3e8bb`）
- 86 + 40 个单测验证
- 72h 压测验证（当前 T+7h+ 稳定运行）

### 建议
- 在 CI 添加 `clippy::await_holding_lock`（但注意：dashmap RefMut 当前未被检出）
- 在 PR 模板中提醒检查这一点
- 考虑迁移高争用场景到 `tokio::sync::RwLock`（异步原生，可跨 await 持有）

## 结论

✅ 工程内**唯一**的跨 await 持锁问题已在 T-F1 中修复。
其余审查项均为合规的安全模式。
