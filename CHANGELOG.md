# Changelog

## [0.2.4] — 2026-05-12

### 🐛 Critical Bug Fixes

- **DashMap Deadlock in BEP Connection Race Resolution** (T-F1):
  `ConnectionManager::register_connection` previously held a DashMap write guard
  (`RefMut`) across `.await` on `conn.close()`. When multiple connections raced
  for the same device_id shard (common in BEP incoming/outgoing race), the
  internal `parking_lot::RwLock` would block other tokio workers, eventually
  freezing the entire runtime at ~T+180s.
  - Refactored to use `RegisterAction` enum with explicit lock-release before await.
  - Verified stable past T+8h in 72h stress test (previously consistently froze at T+180s).
  - All 86 `syncthing-net` unit tests pass.

### 🔧 Stress Test Diagnostics (T-F1)

- **Panic hook with backtrace**: All panics now write to `stress-crash.log` with `std::backtrace::Backtrace::force_capture()`.
- **Main task heartbeat**: New `stress-heartbeat.log` written every 30s by an independent task. Distinguishes runtime freeze (no panic) from external termination.
- **Monitor tick 60s**: Long-run mode tick interval reduced from 600s → 60s for early-death visibility.
- **rss_mb fix**: `processes_by_exact_name` now correctly looks for `stress_test` instead of `syncthing`.
- **Daemon stderr capture**: PowerShell launcher now redirects stderr and sets `RUST_BACKTRACE=full`.

### 🚀 Performance Benchmarks (T-A1, T-B1)

- **Full Criterion baseline** captured: scanner (1.49 GiB/s @1MiB) + puller (1.46 GiB/s @1MiB) + bep encode/decode (507 MiB/s).
- **T-B1 rayon validation**: Scanner SHA-256 parallelization gives **9-11x speedup** on multi-block files (20-core machine).
  - 16 MiB: 2.04 → 19.02 GiB/s (9.32x)
  - 256 MiB: 2.01 → 23.17 GiB/s (11.51x)
- **CI bench-smoke job** prevents benchmark rot.

### 🛡️ Code Quality (T-F2)

- **Unwrap audit complete**: Workspace production unwraps reduced from 22 → 15
  (remaining all are `.expect("...")` with documented invariants).
- Fixed runtime panic risks:
  - `parse().unwrap()` for `"0.0.0.0:0"` → `SocketAddr::new(...)` constructor
  - `relay_url.as_ref().unwrap()` → `let Some(...) else continue` pattern
  - `TcpTransport::start()` double-call panic → `SyncthingError::config(...)` Result
  - `store.rs:98` zero capacity panic → `cache_capacity.max(1)`

### 📐 File Structure (T-E1)

- Final 5 large files split into `mod.rs` + `tests.rs` pattern:
  - `messages.rs` (910) → `messages/mod.rs:697 + tests.rs:213`
  - `types.rs` (882) → `types/mod.rs:805 + tests.rs:76`
  - `connection.rs` (770) → `connection/mod.rs:695 + tests.rs:74`
  - `service.rs` (715) → `service/mod.rs:684 + tests.rs:30`
  - `session.rs` (979) → `session/mod.rs:606 + tests.rs:366`
- **types/connection.rs extracted** (805 → 639 lines)

### 🔄 CI (T-G2)

- **New `bench-smoke` job**: Compiles all benchmarks + runs short smoke tests
  with `--output-format bencher`. Prevents bench rot, provides perf data point per PR.
- **`clippy::await_holding_lock` lint enabled**: Prevents T-F1-class deadlocks at compile time.

### 🧹 Dead Code Cleanup

- Removed truly-dead items: `ConnectionManager::from_arc()` method (28 lines), `PORT_MAP_SERVICE_TIMEOUT` constant.
- Fixed 4 `redundant_clone` instances (perf micro-opts in EventBus, FolderWatcher, BlockServer).
- Fixed 1 `redundant_field_names` (config.rs).

### 📋 Documentation

- New: `docs/reports/STRESS_TEST_DEATH_INVESTIGATION_2026-05-12.md` — full T-F1 RCA writeup.
- New: `docs/reports/UNWRAP_AUDIT_2026-05-12.md` — T-F2 complete audit.
- New: `docs/reports/BASELINE_2026-05-12.md` — full Criterion baseline + T-B1 results.
- New: `docs/reports/LOCK_AWAIT_AUDIT_2026-05-12.md` — workspace-wide lock-await audit.

### Stats

- **13 commits** since v0.2.3
- **295 unit tests** passing
- **0 clippy warnings** (including new `await_holding_lock`)
- **72h stress test running** since 13:07 (currently T+8h+ healthy with deadlock fix)

## [0.2.3] — 2026-05-11

### Infrastructure & Quality

- **CI Full Green**: Fixed workspace-wide `cargo fmt` and `cargo clippy` violations (118 files, 10 clippy auto-fixes). All 7 CI jobs now pass on `ubuntu-latest` and `windows-latest`.

## [0.2.2] — 2026-05-11

### Infrastructure & Quality

- **72h Stress Test Launched**: `stress_test` binary now runs with Windows Scheduled Task auto-resume (fault injection every 30 min, file injection every 5 min). See `scripts/register-stress-task.ps1`.
- **GitHub Community Health**: Added `.github/ISSUE_TEMPLATE/`, `.github/PULL_REQUEST_TEMPLATE.md`, `.github/workflows/ci.yml`, and `CODE_OF_CONDUCT.md`.
- **README Maintenance**: Fixed stale "72h not started" status, removed encoding artifacts (`格雷侧`), added Stress Test quick-start block.

### Documentation

- Tuning plan published: `docs/plans/TUNING_PLAN_2026-05-11.md` — cross-cut performance/stability/architecture-debt roadmap.

---

## [0.2.0] — 2026-04-26

### Overview
Beta release. REST API write endpoints, TUI real-time observability, config hot-reload, and Relay-integrated parallel dialer. 279 tests passing, 0 clippy warnings.

### What's Working
- **REST API Write Endpoints**: `PUT /rest/config`, `POST /rest/system/{config,restart,shutdown,pause,resume}`, `POST /rest/db/scan`
- **TUI Real-time State**: Event bridge from sync engine → TUI (folder states, device connect/disconnect, sync progress, config changes)
- **Config Hot-reload**: `notify`-based `config.json` watcher reloads running daemon without restart
- **Relay Parallel Dialer**: Relay URLs now race alongside direct TCP addresses in `ParallelDialer` with unified RTT scoring
- **E2E Test Harness**: `TestNode` spawns temporary nodes with auto-generated certs for handshake/integration tests

### Architecture Milestones
- Phase 3-A — Relay addresses integrated into `ParallelDialer` scoring/racing ✅
- Phase 4 — TUI event bridge + live sync state + config hot-reload ✅
- Phase 5 — Discovery results (Global query + Local broadcast) dynamically feed `ConnectionManager` address pool ✅

### Known Limitations
- Cross-network auto-discovery without Tailscale still in integration (Phase 5)
- QUIC / full ICE not yet implemented
- Web GUI not planned (TUI only)
- 72h stress test not started

---

## [0.1.0] — 2026-04-20

### Overview
First alpha release. Core BEP file synchronization between Rust and official Go Syncthing is verified end-to-end. 257 unit tests passing, release build clean.

### What's Working
- **File Sync**: Bidirectional Push/Pull E2E verified (Rust ↔ Go over Tailscale)
- **Protocol**: TLS handshake, BEP Hello, ClusterConfig, Index, Request/Response
- **Network**: TCP+TLS transport with `ReliablePipe` abstraction; WebSocket, Proxy, DERP Relay transports implemented
- **Watcher**: Filesystem watcher (`notify` crate) with 1s debounce → scan → IndexUpdate
- **REST API**: `/rest/system/status`, `/rest/system/connections`, `/rest/db/status`, `/rest/db/completion`, device/folder CRUD
- **TUI**: Interactive terminal UI with device/folder management, real-time logs, help page
- **Config**: JSON-based configuration persistence; TUI changes notify running daemon

### Architecture Milestones
- Phase 1 — Identity decoupling (`Identity` trait, `TlsIdentity`) ✅
- Phase 2 — Transport decoupling (`Transport` trait, `RawTcp`/`WebSocket`/`Proxy`/`DerpTransport`) ✅
- Phase 3 — DERP Relay protocol + integration ✅

### Known Limitations
- Long-term connection stability pending 72h stress test validation
- TUI and REST API config instances are not fully synchronized at runtime (restart required)
- QUIC / full ICE (STUN+TURN+hole punching) not yet implemented
- Some TUI widgets (spinner, progress gauge) are reserved for future use

### Test Results
```
cargo test --workspace --lib  → 257 passed, 0 failed, 1 ignored
cargo build --release         → 0 errors, 0 warnings
```
