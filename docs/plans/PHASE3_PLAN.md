# Phase 3 Plan — BepSession Hardening & Production Validation

> ⚠️ **勘误（2026-04-27）**：3.3 节"Cloud Go 节点双向验证"实际为**格雷侧远程节点**，当时误识别为 Go Syncthing，实际运行的是 pre-fix Rust 构建。验证结论"Push/Pull E2E 完成"对 Rust→Go 场景不具参考价值。
>
> **基线**: Phase 2 已完成（`90776e7`），`ReliablePipe` + `BepSession` + `ConnectionManager` 多路径架构稳定。
> **目标**: 将 BepSession 从"功能正确"推进到"生产可观测 + 长期稳定 + 上下游可集成"。

---

## 3.1 BepSession Observability & Events（本周执行）

### 背景
今天的 Push E2E 测试暴露出两个问题：
1. **无法感知对方同步状态**：云端 Go 节点是否已完成对我们 Index 的处理？是否需要拉取块？没有精确信号。
2. **会话级别 metrics 缺失**：无法统计 per-device 的消息速率、错误率、心跳超时次数。

### 任务

#### 3.1.1 `BepSessionEvent` 枚举

```rust
pub enum BepSessionEvent {
    /// ClusterConfig exchange completed (both directions)
    ClusterConfigComplete {
        device_id: DeviceId,
        shared_folders: Vec<String>,
    },
    /// Initial Index sent to peer
    IndexSent {
        device_id: DeviceId,
        folder: String,
        file_count: usize,
    },
    /// Received Index from peer
    IndexReceived {
        device_id: DeviceId,
        folder: String,
        file_count: usize,
    },
    /// Received IndexUpdate from peer (potential sync trigger)
    IndexUpdateReceived {
        device_id: DeviceId,
        folder: String,
        file_count: usize,
    },
    /// Peer requested a block from us (push direction active)
    BlockRequested {
        device_id: DeviceId,
        folder: String,
        name: String,
        offset: i64,
        size: i32,
    },
    /// We requested a block from peer
    BlockRequestSent {
        device_id: DeviceId,
        request_id: i32,
    },
    /// Block response received (success or error)
    BlockResponseReceived {
        device_id: DeviceId,
        request_id: i32,
        success: bool,
    },
    /// Heartbeat timeout detected
    HeartbeatTimeout {
        device_id: DeviceId,
        last_recv_age: Duration,
    },
    /// Session ended (clean close or error)
    SessionEnded {
        device_id: DeviceId,
        reason: String,
    },
}
```

#### 3.1.2 `BepSessionMetrics` 结构体

Per-session counters:
- `messages_sent: AtomicU64`
- `messages_recv: AtomicU64`
- `bytes_sent: AtomicU64`
- `bytes_recv: AtomicU64`
- `blocks_requested: AtomicU64`
- `blocks_served: AtomicU64`
- `heartbeat_timeouts: AtomicU64`
- `errors: AtomicU64`

#### 3.1.3 `BepSession` 构造函数增加 `event_tx`

```rust
pub fn new(
    device_id: DeviceId,
    conn: Arc<BepConnection>,
    handler: Arc<dyn BepSessionHandler>,
    pending_responses: Arc<DashMap<i32, oneshot::Sender<Response>>>,
    event_tx: Option<mpsc::UnboundedSender<BepSessionEvent>>,
) -> Self
```

当 `event_tx` 为 `Some` 时，在关键状态转换点发送事件。

### 验收标准
- [x] `BepSessionEvent` 定义完成
- [x] `test_session_events` 单元测试：验证 ClusterConfigComplete / IndexReceived / BlockRequested 事件在正确时机触发
- [x] `daemon_runner.rs` 订阅 `BepSessionEvent`，将关键事件（BlockRequested, SessionEnded）写入 tracing 日志

---

## 3.2 Peer Sync State Exposure（本周执行）

### 背景
会议室中 devbase 的核心诉求：需要一个比 `FolderStatus::Idle` 更精确的"对方是否已同步"信号。

### 方案：`on_peer_index_update` 增强

在 `BepSessionHandler` 中增加：

```rust
/// Called when we receive an IndexUpdate from peer.
/// Implementors can compare the peer's version vector with local state
/// to determine if the peer is "in sync".
async fn on_peer_index_update(
    &self,
    device_id: DeviceId,
    update: IndexUpdate,
    peer_max_sequence: u64,
) -> Result<()>;
```

`BepSession` 在收到 `IndexUpdate` 时解析每个 `FileInfo` 的 `sequence`，取最大值作为 `peer_max_sequence`，连同 `update` 一起传给 handler。

### 同步完成判断逻辑（devbase 侧将来实现）

```rust
fn is_peer_in_sync(local_max_seq: u64, peer_max_seq: u64) -> bool {
    local_max_seq == peer_max_seq
}
```

> ⚠️ 注意：这只是"已知范围内"的同步完成。如果 peer 有未广播的本地变更，此判断会给出假阳性。但对于 devbase 的 pragmatic 需求已足够。

### 验收标准
- [x] `on_peer_index_update` 接口定义完成
- [x] `DaemonBepHandler` 实现中记录 peer max sequence（内存缓存）
- [x] REST API 新增 `/rest/db/completion?folder=X&device=Y` 返回 `{ "completion": 100 }`

---

## 3.3 Push E2E Forced Trigger Test（下周执行）

### 背景
今天云端 Go 节点未触发 Block request，推测是因为其本地文件版本已是最新。

### 方案：制造"必须拉取"条件

1. **在云端 Go 侧删除一个文件**（通过其 Web UI 或 API）
2. **在 Rust 侧保持该文件**（确保 Rust 的 Index 包含此文件）
3. **Rust 触发 scan → IndexUpdate**
4. **观察云端 Go 是否发送 Request** 来重新拉取该文件

### 验收标准
- [x] 日志中观察到 `BlockRequested` 事件（通过 3.1 的 observability）
- [x] **Push E2E**: Rust → Go 文件推送成功（Request/Response + SHA-256 验证）
- [x] **Pull E2E**: Go → Rust 文件拉取成功（`cloud_push_test.txt` 50 bytes）
- [x] 连接在测试期间不中断（从"秒断"改善到稳定维持 6+ 分钟）

---

## 3.4 72h Stress Test Infra（移交格雷远程执行）

> **状态**: 待启动。本机（Windows）夜间断电，无法执行长期无人值守测试。后续编译版本提交后，由格雷在远程 Linux 服务器上执行。

### 方案

1. **自动化脚本**（Bash）：
   - 每 10 分钟在 `test-folder` 创建一个随机内容文件
   - 每 30 分钟修改一个现有文件
   - 每小时删除一个旧文件
   - 每 5 分钟查询 REST API `/rest/system/status` 和 `/rest/db/completion`
   - 记录到 CSV/JSON 日志

2. **监控指标**：
   - 连接是否存活
   - 消息速率（msg/min）
   - 错误计数
   - 内存使用

3. **故障恢复**：
   - 如果连接断开，自动等待 5 分钟后检查是否重连成功
   - 如果 3 次重连失败，记录故障并退出

### 验收标准
- [ ] 脚本可无人值守运行
- [ ] 72 小时后日志中 0 次连接不可恢复中断
- [ ] 所有文件变更在 60 秒内传播到对端

---

## 执行记录

```
2026-04-20: Phase 3 核心功能全部完成 ✅
  3.1 BepSession observability & events — 完成
  3.2 Peer sync state exposure — 完成
  3.3 Push E2E + Pull E2E — 完成（Cloud Go 节点双向验证）

Next:
  3.4 72h stress test infra — 待启动
  Phase 3.5 收尾优化 — 待规划
  Phase 4 规划 — 待启动
```

---

## 风险

| 风险 | 可能性 | 影响 | 缓解 |
|------|--------|------|------|
| 云端 Go 节点再次不触发 Request | 中 | 高 | 准备本地 Go 节点作为 fallback test peer |
| 72h 测试期间 Windows 更新/重启 | 中 | 高 | 使用 `cargo run --release` 后台任务 + 开机自启脚本 |
| BepSessionEvent 引入性能开销 | 低 | 低 | `event_tx` 为 `Option`，无订阅者时零开销 |
