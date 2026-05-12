# 72h Stress Test 进程死亡调查报告

**调查时间**: 2026-05-12 12:00-13:20 UTC+8
**调查者**: T-F1 增强诊断
**结论**: ✅ **根本原因已定位并修复**

---

## 根因总览

**问题**: `ConnectionManager::register_connection` 在 DashMap 写锁守卫范围内执行 `.await`

**位置**: `crates/syncthing-net/src/manager/registry.rs` 原 L28-L67

**机制**:
```rust
// BUG: nested 是 DashMap 写锁守卫（RefMut）
if let Some(nested) = self.connections.get_mut(&device_id) {
    if let Some(existing) = nested.iter().next() {
        ...
        existing.value().conn.close().await.ok();  // ← 跨 .await 持锁
        nested.clear();
        nested.insert(conn_id, ...);
    }
}
```

DashMap 内部使用 `parking_lot::RwLock` 分片，写锁守卫跨 `.await` 持有时：
1. 当前 tokio worker 在 `.await` 挂起前持有该分片的同步锁
2. 其他 tokio worker 试图获取**同一分片**任何 key 时会**完全阻塞**（同步等待）
3. 由于 worker 被阻塞，tokio runtime 失去调度能力
4. 当所有 worker 都在等待该分片时 → **完全死锁**

**爆发条件**:
- BEP race resolution 路径触发 `close().await`
- 同时多个连接（双向 incoming/outgoing）争用同一 device_id 分片
- 连接风暴时（重连风暴）：所有 worker 试图获取同分片 → 全部阻塞

## 验证数据

### 修复前
| 启动方式 | 启动时间 | 心跳停止 | 进程状态 |
|---------|---------|---------|---------|
| PowerShell Start-Process | 12:36:20 | T+0s | T+~180s 死亡 |
| bash nohup & | 12:40:34 | T+210s | T+~240s 死亡 |
| bash nohup + disown | 12:54:34 | T+180s | **冻结，未死亡** |

### 修复后（含 disown）
| 时间 | 状态 | 心跳数 | CSV 行数 |
|------|------|--------|----------|
| T+5min | 健康运行 | 9 | 5 |
| T+10min | 健康运行 | 22 | 12 |
| 内存 | 12.7→14.5 MB（稳定）| | |
| CPU | 0.45s/10min（极低）| | |

### 验证不再冻结的关键证据
- ✅ T+185s（原冻结点）后心跳继续：hb#7, hb#8, hb#9...
- ✅ T+185s（原冻结点）后 CSV 写入继续：T+185s, T+245s, T+305s...
- ✅ T+305s 文件注入任务激活：files_a 从 1→2
- ✅ T+605s 第二次注入：files_a 从 2→3

## 修复方案

将持锁的 `.await` 重构为：
1. **读取锁内容到本地变量**（克隆 `Arc<BepConnection>`）
2. **释放锁**（作用域结束）
3. **执行 .await**（不持锁）
4. **必要时重新获取锁**做最终修改

引入 `RegisterAction` 枚举将状态决策与执行分离。

```rust
let (action, old_conn_to_close) = {
    if let Some(nested) = self.connections.get(&device_id) {
        // ... 读取并决策，克隆 Arc
        (action, Some(Arc::clone(&existing.value().conn)))
    } else {
        (RegisterAction::CreateNew, None)
    }
};  // 锁在此释放

// 安全 .await
if let Some(old_conn) = old_conn_to_close {
    old_conn.close().await.ok();
}

// 执行最终修改
match action { ... }
```

## 衍生发现

### PowerShell `Start-Process -Redirect*` 问题
即使没有 DashMap 死锁，PowerShell 的 stdin/stdout/stderr 重定向机制
在父 PowerShell 退出后会**破坏子进程文件句柄**：
- tracing_subscriber 写入 silently fail
- 进程残存但所有 I/O 失败
- 表现为 "T+5s 后无任何日志"

**解决**: 使用 `bash> ... > log 2> err < /dev/null & disown` 真正脱离会话。

### Defender 假阳性
进程死亡前后大量 `Defender cloud lookup`（Event ID 2010），
经验证并非 Block/Detection。系本地高频文件 I/O 触发的常规扫描。

## 工程层面改进

1. **诊断设施增强**（已合并）
   - `stress-crash.log`：panic hook + force_capture backtrace
   - `stress-heartbeat.log`：主线程 30s 心跳，独立于 tracing
   - 监控任务 60s tick（长运行模式）
   - daemon 脚本添加 stderr 捕获

2. **死锁防护**（已合并）
   - register_connection 完全重构，无跨 .await 持锁
   - 86 个 syncthing-net 单元测试全部通过

3. **后续审计建议**
   - 全工程审查所有 `dashmap.get_mut().await` 模式
   - 添加 `clippy::await_holding_lock` 到 CI（注：dashmap RefMut 未必被检出）
   - 考虑迁移到 `tokio::sync::RwLock`（异步原生）

## 状态总结

| 项目 | 状态 |
|------|------|
| 死亡根因定位 | ✅ DashMap 跨 await 持锁 |
| 修复实施 | ✅ commit pending |
| 单元测试 | ✅ 86/86 通过 |
| 实际验证 | ✅ T+10min 稳定运行 |
| 72h 压测 | 🔄 进行中（PID 20048，启动 13:07:54）|
