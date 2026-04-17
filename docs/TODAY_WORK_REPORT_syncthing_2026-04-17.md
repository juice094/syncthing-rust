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

## Next Steps

1. **72h stress test** — Run long-duration stability test with local Go node.
2. **Push E2E retry** — Force a file version conflict or pause/resume the cloud peer to trigger an inbound block request.
3. **Phase 3 planning** — Workspace migration and production folder sync validation.4. **Dependabot security alerts** — Browser-based manual confirmation for GitHub Security tab.
