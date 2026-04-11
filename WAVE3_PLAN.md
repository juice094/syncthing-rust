# Wave 3 子集群执行计划

**目标**: 连接管理与运行时基础设施（Connection Management & Runtime）  
**基调**: 基于现有骨架（`ConnectionManager`、`ConnectionEntry`、`PendingConnection`）进行功能填充与连接，避免推倒重来。

---

## 背景：当前代码状态

在 `syncthing-net` 中，以下结构已经存在但大多处于**未使用或 stub 状态**：

- `ConnectionManager` (`manager.rs`)
  - 已有 `ConnectionEntry`（含 `connected_at`, `last_activity`, `retry_count`）
  - 已有 `PendingConnection`（含 `device_id`, `addresses`, `last_attempt`）
  - 已有 `cleanup_stale_connections()` 方法，但未被调用
- `ConnectionInner` (`connection.rs`)
  - 已有 `last_ping: RwLock<Instant>`，但未被读取
- `TcpTransport` (`tcp_transport.rs`)
  - BEP codec 已部分接入（`BepCodec`），但仍有 JSON 占位残留
- `PortMapper` + `StunClient`
  - Wave 1 已完成，但尚未与 `ConnectionManager` 生命周期关联

---

## 任务拆分（3 个并/串行子代理）

### Task 1: NET-REBIND — 网络变更重绑定
**代理类型**: `coder`  
**预计耗时**: 1 轮（60–120 分钟）  
**依赖**: 无

#### 目标
监听操作系统网络接口变化（IP 变动、Wi-Fi 切换等），触发已有 TCP 连接的健康检查，并在必要时重拨关键对端。

#### 参照
- Tailscale `netmon` 包的设计模式：通过平台特定机制获取网络变化事件（Windows: `NotifyAddrChange` / WMI / netdev crate；跨平台: `netdev` 的接口监听）
- Go Syncthing `lib/connections` 中的 rebind 逻辑

#### 交付物
1. 新建 `crates/syncthing-net/src/netmon.rs`
   - `NetMonitor` 结构体，提供 `subscribe() -> Receiver<NetChangeEvent>`
   - 跨平台实现：优先使用 `netdev` crate 的接口变化监听（如果 `netdev` 已有监听 API），否则降级为定时轮询 `netdev::get_interfaces()`
2. 修改 `ConnectionManager`
   - 新增 `async fn handle_net_change(&self)`
   - 当收到 `NetChangeEvent` 时：
     a. 调用 `cleanup_stale_connections()` 清理已失效连接
     b. 对本地 `known_devices` 中的每个设备，如果当前无活跃连接，则触发后台重拨（reuse 现有 `dial` 逻辑）
3. 测试
   - `test_netmon_detects_interface_change`：模拟接口列表变化，验证事件通道收到通知
   - `test_rebind_triggers_redial`：注入 NetChangeEvent，验证 ConnectionManager 对离线设备发起新的 PendingConnection

---

### Task 2: NET-DIALER — 地址质量评分 + 并行拨号
**代理类型**: `coder`  
**预计耗时**: 1–2 轮  
**依赖**: Task 1（或独立，最终需与 ConnectionManager 集成）

#### 目标
为单个设备的多个候选地址（direct TCP、relay、UPnP 映射后的公网地址）建立并行拨号与 RTT/成功率评分机制，优先使用最快路径。

#### 参照
- iroh 的 `endpoint.rs` / `socket::remote_map` 中的路径选择与 RTT 排序
- Go Syncthing `lib/connections` 的 `dialParallel` / 地址优先级（LAN > WAN > Relay）

#### 交付物
1. 新建 `crates/syncthing-net/src/dialer.rs`
   - `AddressScore`：权重因子（LAN 奖励、历史 RTT、TLS 握手耗时、上次成功时间）
   - `ParallelDialer::dial(device_id, addresses) -> Result<BepConnection, SyncthingError>`
     - 并发限制：最多同时发起 N 个连接（N=3）
     - 竞速机制：首个成功 TLS + BEP Hello 的连接获胜，其余立即取消
     - 评分更新：记录每个地址的握手耗时与成功/失败次数
2. 修改 `ConnectionManager`
   - 用 `ParallelDialer` 替换/封装现有的简单顺序拨号逻辑
   - 维护 `device_addr_scores: DashMap<DeviceId, Vec<ScoredAddress>>`
   - 在 `dial_device` 时按评分排序地址列表
3. 测试
   - `test_parallel_dialer_race`：mock 多个地址，验证最快成功者被选中
   - `test_address_score_preference`：LAN 地址 vs WAN 地址评分比较
   - `test_dialer_cancels_slow_connections`：慢地址被及时取消

---

### Task 3: SYNC-SUPERVISOR — Rust 版 `suture.Supervisor`
**代理类型**: `coder`  
**预计耗时**: 1–2 轮  
**依赖**: 无（但需与 `syncthing-sync` 中的 service/actor 集成）

#### 目标
为同步引擎中的长生命周期任务（FolderService、ConnectionManager、IndexHandler）提供统一的监督树：崩溃自动重启、指数退避、最大重启次数限制。

#### 参照
- Go `github.com/thejerf/suture` 的 Service / Supervisor / Spec 概念
- `tokio::task::JoinSet` 或 `tokio::spawn` + `select!` 的 actor 监督模式

#### 交付物
1. 新建 `crates/syncthing-sync/src/supervisor.rs`
   - `Supervisor`：管理多个 `SupervisedTask`
   - `SupervisedTask`：包装一个 `Future` 或 `tokio::task::JoinHandle`
   - 配置：
     - `restart_policy`: `Always | OnFailure | Never`
     - `backoff`: 指数退避（初始延迟、最大延迟、重置窗口）
     - `max_restarts`: 单位时间内的最大重启次数（如 5 次/60秒）
   - 行为：
     - 当子任务 `panic` 或返回 `Err` 时，根据策略决定是否重启
     - 若超过 `max_restarts`，则将该子任务标记为 `Failed` 并向上传播错误（或调用用户回调）
2. 集成点（修改现有文件）
   - `service.rs` 中的 `SyncService::run()`：用 `Supervisor` 启动 folder services
   - `syncthing-net/src/manager.rs`：可选地让 `ConnectionManager` 的 background loop 也受监督
3. 测试
   - `test_supervisor_restarts_on_panic`：子任务 panic，验证自动重启
   - `test_supervisor_backoff_increases`：验证重启间隔呈指数增长
   - `test_supervisor_max_restarts_exceeded`：验证超过阈值后停止重启并触发回调
   - `test_supervisor_graceful_shutdown`：验证 `Supervisor::shutdown()` 能优雅终止所有子任务

---

## 执行顺序建议

```
           ┌─────────────────┐
           │   Start Wave 3  │
           └────────┬────────┘
                    │
     ┌──────────────┼──────────────┐
     ▼              ▼              ▼
 NET-REBIND    NET-DIALER    SYNC-SUPERVISOR
 (并行)         (并行)        (并行)
     │              │              │
     └──────────────┴──────────────┘
                    │
                    ▼
           最终集成与全量测试
```

**说明**: 三个任务之间耦合度低，可以**并行启动子代理**。唯一的共享点是 `ConnectionManager`，因此：
- Task 1 和 Task 2 都会修改 `manager.rs`，若并行执行需在完成后手动解决冲突（或串行执行）。
- **推荐**: Task 1 与 Task 3 并行；Task 2 在 Task 1 完成后串行（避免 `manager.rs` 冲突）。

---

## 验收标准

1. `cargo test -p syncthing-net` 全部通过（新增测试 ≥ 3 个/任务）
2. `cargo test -p syncthing-sync` 全部通过（新增测试 ≥ 4 个）
3. 无新增 `error` 级别编译错误；`warning` 数量不显著增加
4. 每个任务需在对应文件顶部或 `lib.rs` 中补充简短模块文档

---

## 风险与降级方案

| 风险 | 影响 | 降级方案 |
|------|------|----------|
| `netdev` 接口监听 API 在 Windows 下不稳定 | NET-REBIND 延迟 | 改用 5s 轮询 `netdev::get_interfaces()` + IP 列表 diff |
| `ParallelDialer` 竞速取消 TLS 握手较复杂 | NET-DIALER 超时 | 简化为顺序拨号，但保留评分排序；竞速留待后续优化 |
| Supervisor panic 捕获需要 `std::panic::catch_unwind` + `AssertUnwindSafe` | 跨 await 点边界复杂 | 仅监督 `tokio::spawn` 的 JoinHandle，不捕获 Future 内部 panic（依赖 task panic 钩子） |
