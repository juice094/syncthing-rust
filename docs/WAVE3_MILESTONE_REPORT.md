# Wave 3 里程碑报告

**日期**: 2026-04-09  
**分支**: main (`syncthing-rust-rearch` workspace)

---

## 1. 本阶段目标回顾

Wave 3 的主题是**连接管理与运行时基础设施**（Connection Management & Runtime），包含三项子任务：
1. **NET-REBIND** — 网络变更监听与 TCP 连接重绑定
2. **SYNC-SUPERVISOR** — Rust 版 `suture.Supervisor`（自动重启 + 退避 + 最大重启限制）
3. **NET-DIALER** — 地址质量评分 + 并行拨号竞速

---

## 2. 完成项

### 2.1 NET-REBIND (`syncthing-net`)
- **新建文件**: `crates/syncthing-net/src/netmon.rs`
  - `NetMonitor` 通过 `netdev::get_interfaces()` 轮询（每 5s）检测网络接口变化
  - 输出 `tokio::sync::mpsc::Receiver<NetChangeEvent>`
- **修改文件**: `crates/syncthing-net/src/manager.rs`
  - `ConnectionManager` 集成 `NetMonitor`，在后台监听网络变更
  - 实现 `cleanup_stale_connections()`：移除已失效的连接条目
  - 收到 `NetChangeEvent` 后，对离线设备自动创建 `PendingConnection` 触发重拨
- **修改文件**: `crates/syncthing-net/src/lib.rs` — 导出 `netmon` 模块
- **新增测试**:
  - `test_netmon_detects_interface_change` ✅
  - `test_rebind_triggers_redial` ✅

### 2.2 SYNC-SUPERVISOR (`syncthing-sync`)
- **新建文件**: `crates/syncthing-sync/src/supervisor.rs`
  - `Supervisor` 管理多个 `SupervisedTask`
  - 支持策略：`Always`、`OnFailure`、`Never`
  - 指数退避：初始延迟 → 最大延迟，窗口到期后重置计数
  - `max_restarts`：单位时间内超过阈值则标记 `Failed` 并触发回调
  - `shutdown()` 优雅终止所有子任务
- **修改文件**: `crates/syncthing-sync/src/service.rs`
  - `SyncService::run()` 现在通过 `Supervisor` 启动 folder services
  - `SyncService::stop()` 调用 `supervisor.shutdown().await`
- **修改文件**: `crates/syncthing-sync/src/lib.rs` — 导出 `supervisor` 模块
- **新增测试**:
  - `test_supervisor_restarts_on_panic` ✅
  - `test_supervisor_backoff_increases` ✅
  - `test_supervisor_max_restarts_exceeded` ✅
  - `test_supervisor_graceful_shutdown` ✅

### 2.3 NET-DIALER (`syncthing-net`)
- **新建文件**: `crates/syncthing-net/src/dialer.rs`
  - `ParallelDialer` 支持最多 3 个地址并发竞速拨号
  - `AddressScore` 评分维度：LAN 优先、低 RTT 奖励、成功率加权、失败惩罚
  - 首个完成 TLS + BEP Hello 的连接获胜，其余任务通过 `AbortHandle` 取消
  - 内部 `DashMap` 维护每个地址的历史统计
- **修改文件**: `crates/syncthing-net/src/manager.rs`
  - `ConnectionManager` 持有 `Arc<ParallelDialer>`
  - `spawn_connect_task` 改为调用 `parallel_dialer.dial(...)` 并按评分排序地址
  - `ConnectionManager::new` 自动生成 TLS 证书并实例化 `ParallelDialer`
- **修改文件**: `crates/syncthing-net/src/lib.rs` — 导出 `dialer` 模块
- **新增测试**:
  - `test_parallel_dialer_race` ✅
  - `test_address_score_preference` ✅
  - `test_dialer_cancels_slow_connections` ✅

---

## 3. 测试验证结果

执行命令：
```bash
cargo test -p syncthing-core -p syncthing-sync -p syncthing-net -p bep-protocol
```

| Crate | Passed | Failed | Ignored |
|-------|--------|--------|---------|
| `bep-protocol` | 13 | 0 | 0 |
| `syncthing-core` | 12 | 0 | 0 |
| `syncthing-net` | **39** | 0 | 1 |
| `syncthing-sync` | **27** | 0 | 0 |

**Wave 3 新增测试：9 个（netmon 2 + supervisor 4 + dialer 3）**

**全量核心工作区：91 passed, 0 failed, 1 ignored**

> 说明：存在若干 `unused_imports` / `dead_code` warning，但均非新增错误，不影响编译与测试通过。

---

## 4. 当前整体架构状态

### 已完成功能（Wave 1 ~ Wave 3）

| 层级 | 功能 | 状态 |
|------|------|------|
| **网络层** | TCP+TLS BEP 握手 | ✅ |
| **网络层** | STUN 客户端 | ✅ |
| **网络层** | Portmapper (UPnP/NAT-PMP) | ✅ |
| **网络层** | 网络变更重绑定 (netmon) | ✅ |
| **网络层** | 地址评分 + 并行拨号 | ✅ |
| **网络层** | iroh 集成 | ⏸️ pending (降级为可选 feature) |
| **同步引擎** | Delta Index (IndexID + Sequence) | ✅ |
| **同步引擎** | 冲突解决流水线 | ✅ |
| **同步引擎** | Supervisor 监督树 | ✅ |
| **BEP 协议** | Hello / 消息编解码 | ✅ |

### 缺失/待规划
- `syncthing-fs` / `syncthing-db` / `syncthing-api` — 当前工作区目录中不存在这些 crate，相关任务（完整 ignore 规则、数据库 metadata、API TOML 配置）在另一代码库完成，尚未合并到本工作区。
- 运行时 CLI (`cmd/syncthing`) 目前为占位实现，需要后续串联各 crate 启动流程。

---

## 5. Git 状态

Wave 3 已作为新 commit 提交：
```
wave2 milestone: delta index, conflict resolution, workspace compile fixes
add wave3 execution plan: net rebind, parallel dialer, supervisor
```
后续 Wave 3 代码变更已落盘，等待最终归档 commit。
