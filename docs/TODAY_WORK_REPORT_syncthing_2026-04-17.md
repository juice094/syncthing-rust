# Work Report — 2026-04-17

## Session Focus

Phase 2 Network Abstraction: decouple BEP protocol logic from TCP transport, establish `ReliablePipe` abstraction, and verify correctness via `MemoryPipe` unit tests.

---

## Completed Work

### 1. `ReliablePipe` trait definition (`syncthing-core`)
- Added `ReliablePipe: AsyncRead + AsyncWrite + Send + Sync + Unpin`
- Methods: `local_addr()`, `peer_addr()`, `path_quality()`, `transport_type()`
- Added `PathQuality` and `TransportType` enums
- Added `BoxedPipe = Box<dyn ReliablePipe>` type alias

### 2. `TcpBiStream` implements `ReliablePipe`
- `syncthing-net/src/connection.rs`: `impl syncthing_core::traits::ReliablePipe for TcpBiStream`
- `transport_type()` returns `TransportType::Tcp`

### 3. `BepConnection` decoupled from concrete TCP
- Changed `BepConnection::new` signature from `TcpBiStream` to `syncthing_core::traits::BoxedPipe`
- Internal storage switched from `Arc<Mutex<TcpBiStream>>` to split `ReadHalf<BoxedPipe>` / `WriteHalf<BoxedPipe>`
- All callers in `tcp_transport.rs` and `dialer.rs` updated to wrap with `Box::new(...)`

### 4. `BepHandshaker` extracted
- New file: `crates/syncthing-net/src/handshaker.rs`
- Provides `server_handshake()` and `client_handshake()` using generic `AsyncRead + AsyncWrite + Unpin`
- `tcp_transport.rs` inlined hello logic replaced with `BepHandshaker` calls
- Added unit test `test_handshake_over_memory_pipe`

### 5. `ConnectionManager` multi-path support
- Changed connection pool from `DashMap<DeviceId, ConnectionEntry>` to `DashMap<DeviceId, DashMap<Uuid, ConnectionEntry>>`
- Added `disconnect_connection(conn_id)` and `get_connection_by_id(conn_id)`
- `cleanup_stale_connections()` now operates per-connection instead of per-device
- Stats calculation updated to aggregate across nested maps

### 6. `MemoryPipe` acceptance test
- `syncthing-test-utils` already implements `ReliablePipe` for `MemoryPipe`
- Added `test_bep_connection_over_memory_pipe` in `connection.rs` tests
- Verified Ping message round-trip over in-memory pipe without real TCP sockets

---

## Verification

```bash
cargo test
# Result: all crates pass, 0 failed
```

Specific crate results:
- `syncthing-core`: 18 passed
- `bep-protocol`: 24 passed
- `syncthing-net`: 44 passed, 1 ignored
- `syncthing-sync`: 36 passed
- `syncthing-db`: 12 passed
- `syncthing`: 5 passed
- Doc-tests: all green

---

## Phase 2.6 — Push E2E Confirmation

- Started Rust daemon with `test-folder` shared to cloud Go peer (格雷).
- Observed successful BEP handshake, ClusterConfig exchange, and Index transmission (9 files) via the new `BepSession`.
- Cloud Go peer sent its own Index and IndexUpdate to Rust.
- **No block requests were observed from the cloud peer within the 3-minute test window.** Likely explanation: the cloud node already considers the folder in sync, or its pull scheduler has not yet queued the new file.
- **Status**: BepSession operates correctly; Push path code (`handle_block_request` → Response) is verified at unit-test level. Full end-to-end confirmation of the cloud peer actively pulling blocks remains pending on the remote node's sync state.

## Phase 2.7 — TUI Device Deletion Verification

- Inspected `cmd/syncthing/src/tui/events.rs:58-69`.
- `KeyCode::Char('d')` on the Devices tab removes the selected device, updates `app.config.devices`, and calls `save_and_log()` which persists to `config.json`.
- **Status**: Already implemented. Marked the active issue as stale and closed.

## Phase 2.8 — `syncthing-core::traits::BepConnection` Alignment

- Attempted to implement `syncthing_core::traits::BepConnection` for `Arc<BepConnection>`.
- **Structural conflict identified**: `request_block` requires a pending-response map and a concurrent `recv_message` loop, which collides with the single-reader `BepSession` steady-state loop.
- **Resolution**: Marked `BepConnection` trait and `BepMessage` enum as `#[deprecated]` in `syncthing-core`. Removed `SyncModel::handle_connection` and its stub implementation in `SyncService`. The canonical architecture is now `ReliablePipe` + `BepSession`.

---

## Phase 3 Plan — 已制定

详见 `docs/PHASE3_PLAN.md`。

**核心方向**：将 BepSession 从"功能正确"推进到"生产可观测 + 长期稳定 + 上下游可集成"。

**本周执行 (3.1 + 3.2)**：
- `BepSessionEvent` 枚举 + per-session metrics（解决今天 Push E2E 无法观测对端行为的问题）
- `on_peer_index_update` 回调 + REST API `/rest/db/completion`（回应 devbase 的"同步完成"信号需求）

**下周执行 (3.3 + 3.4)**：
- Push E2E forced trigger test（在云端 Go 侧删除文件，迫使其回发 Request）
- 72h stress test infra（自动化变更脚本 + 监控 + 故障恢复）

---

## Phase 3.1 完成 — BepSession Observability

### 代码变更
- `crates/syncthing-net/src/session.rs`:
  - `BepSessionEvent` 扩展 6 个变体 + `SessionEnded`
  - `BepSessionMetrics` 8 个 `AtomicU64` 计数器
  - `emit()` / `metrics()` 暴露给外部
  - `handle_message` 每个分支都更新 metrics + 发射事件
  - 心跳超时检测：270s idle → `HeartbeatTimeout` + 断连
- `daemon_runner.rs`: 接入 `with_events`，TUI 状态栏可实时显示 `BlockRequested`/`IndexUpdateReceived` 计数

### 验证
```bash
cargo check -p syncthing-net   # 0 errors
cargo test -p syncthing-net --lib  # 46 passed, 0 failed
```

---

## Phase 3.2 完成 — Peer Sync State + REST API `/rest/db/completion`

### 代码变更
- `crates/syncthing-sync/src/service.rs`:
  - `peer_sync_states: DashMap<(DeviceId, String), usize>` 跟踪每个 (device, folder) 的 needed files
  - `handle_index` / `handle_index_update` 处理完远程索引后自动更新状态
  - `get_folder_completion(device_id, folder_id)` 公共方法
  - `impl syncthing_core::traits::SyncModel for SyncService` 覆盖 `folder_completion()`，基于 total_files - needed 计算百分比
- `crates/syncthing-core/src/traits.rs`:
  - `SyncModel` trait 新增 `folder_completion(&self, folder, device) -> Result<u64>` 默认方法（返回 100）
- `crates/syncthing-net/src/session.rs`:
  - `BepSessionEvent` 新增 `PeerSyncState { device_id, folder }` 变体
  - `handle_message` Index/IndexUpdate 分支 emit `PeerSyncState`
- `cmd/syncthing/src/tui/daemon_runner.rs`:
  - 事件消费任务处理 `PeerSyncState`，记录日志
- `crates/syncthing-api/src/rest.rs`:
  - 新增 `GET /rest/db/completion?folder=xxx&device=xxx` 端点
  - 调用 `sync_model.folder_completion()` 返回 JSON `{ "completion": 95, "device": "...", "folder": "..." }`

### 验证
```bash
cargo check --workspace   # 0 errors
cargo test --workspace    # 245+ passed, 0 failed
```

---

## 下一步：Phase 3.3 Push E2E Forced Trigger

- 在云端 Go 侧删除文件，迫使其向 Rust 节点回发 Block Request
- 验证 Push 方向（Rust 作为块服务器）在生产环境中的真实工作

## 当前阻塞
无。

---

## 杂项
4. **Dependabot security alerts** — Browser-based manual confirmation for GitHub Security tab.
