# Changelog

## [Unreleased]

### Fixed
- **BEP Ping storm**: `BepSession` no longer replies to received Ping messages. BEP Ping is unidirectional keepalive (no Pong exists); two syncthing-rust nodes previously ping-ponged at wire speed (thousands of Pings/sec, ~16MB debug logs in 16 minutes). Ping is now only sent by the 30s heartbeat timer. Reported via Obsidian bridge Android deployment (battery/log impact).
- **TUI Add Folder silently dropped**: `submit_add_folder` built the `Folder` but never pushed it into the config. Folder additions via TUI (`a`/`Ins` on Folders tab) now persist and hot-reload into the running daemon. Stale share selections from a previous Edit Folder no longer leak into new folders.
- **`set_block_source` after `add_folder` had no effect**: `FolderModel` captured `block_source` only at creation, so pulls failed with "No block source configured". `Puller` now holds the source behind a lock and `SyncService::set_block_source` propagates to all existing folder models.
- **Config changes now trigger session renegotiation**: `update_config`/`add_device`/`remove_device`/`add_folder`/`remove_folder` fire a renegotiation hook with the connected device list; the daemon reconnects those devices so ClusterConfig is re-exchanged (matching Go Syncthing behavior). `syncthing-sync` stays network-free via the injected hook.

### Changed
- **Single source of truth for versions**: root `Cargo.toml` `[workspace.package] version = "3.0.4"`; all 14 member crates inherit via `version.workspace = true`.
- **Removed stale `acceptance-tests/` package**: referenced long-removed APIs (e.g. `IrohDiscovery`); coverage superseded by `cmd/syncthing/tests/e2e_*` and `syncthing-net` discovery unit tests.
- **Removed stray git tags `main` and `latest`** (broke `git push origin main` refspec resolution).

### Docs
- Archived 7 completed/superseded files from `docs/plans/` to `docs/archive/plans/`; `docs/plans/` now holds only 3 active plans + the audit report, with a rewritten `INDEX.md`.

### Stats
- **Tests**: 428 passed / 6 ignored / 0 failed (+36 vs v3.0.4 baseline 392; includes 11 TUI event-layer tests, ping no-reply, block_source propagation, renegotiation hook)
- **Clippy**: 0 warnings

## [3.0.4] — 2026-06-27

### Headline: v3.0.4 security hardening, Relay Server v1, structured observability, and Docker deployment.

### Security
- **Path traversal defense**: `validate_remote_name()` rejects `..`, `\0`, absolute paths, empty segments, and backslashes in remote file names at puller/index-handler entry points (was only on block-server side).
- **SSRF protection**: `validate_outbound_addr()` filters multicast, unspecified, and link-local addresses from discovery/relay address pools; relay session addresses additionally filter loopback.
- **Connection flood defense**: `max_connections` (default 1000) now enforced at TCP accept time via `can_accept_connection()`.
- **Key file permissions**: Unix `set_permissions(0o600)` on `cert.pem`, `key.pem`, and `config.json`.
- **Content leakage fix**: 10 `eprintln!` calls in production puller replaced with `tracing` macros; file contents no longer written to stderr.
- **RBAC read-only API key**: REST API now supports a separate read-only API key (`ro_api_key`) that only allows GET/HEAD/OPTIONS requests. Admin API key retains full access.
- **Mass-deletion safety threshold**: `IndexHandler` refuses to interpret a remote full index as "peer deleted everything" when the remote file count drops below 50% of the local DB count (and local has >10 files). Prevents §15-style cascade deletion after peer format/reinstall.

### Network / Protocol
- **BEP Relay Server v1**: new `relay-server` subcommand and `crates/syncthing-net/src/relay/server.rs` provide a standalone relay listener with TLS, session joining, and device-id based routing.
- **WebSocket/WSS transport improvements** and proxy-aware dialing hardening.

### Observability
- **Structured JSON logging**: daemon, TUI, and tray support `--log-format json` for ELK/Splunk/Loki aggregation. Text format remains the default.
- **Prometheus metrics endpoint**: `/metrics` emits build info, uptime, configured devices/folders, connected peers, bytes sent/received, and DB file count/size estimates without pulling in the `prometheus` crate.
- **Deep health check**: `/rest/health` now reports per-subsystem status (database, connection manager, config store).

### Operations
- **Docker + Relay deployment**: added `deploy/Dockerfile`, `deploy/docker-compose.relay.yml`, `deploy/nginx-relay.conf`, and `deploy/grafana-dashboard.json`.
- **Disaster recovery manual**: `docs/operations/DISASTER_RECOVERY.md` documents the peer-format/reinstall recovery SOP.
- **SBOM generation**: `scripts/generate-sbom.sh` produces CycloneDX JSON for release audits.

### Stats
- **Tests**: 392 passed / 0 failed / 3 ignored
- **Clippy**: 0 warnings

## [3.0.3] — 2026-06-16

### Headline: Orchestration, observability & adaptive concurrency upgrades.

### Sync Core
- **Incremental scanning from watcher events**: `FolderModel` maintains `dirty_changes`/`dirty_deletes` sets and triggers incremental subtree/file scans on debounced watcher events, falling back to full scans when the dirty set exceeds `max(100, local_files/10)`.
- **Folder Orchestrator**: new `FolderOrchestrator` in `syncthing-sync` limits global concurrent scans and pulls via `tokio::sync::Semaphore`, staggers first scans deterministically by folder id, and supports per-folder priorities (Low/Normal/High/Critical). All folder scan/pull loops acquire orchestrator permits before running.
- **Adaptive Puller concurrency**: `ConcurrencyPolicy` + `RttTracker` sample block-request RTTs and dynamically adjust puller `downloads`/`blocks` limits every 30 s based on link quality.
- **Predictive health checks**: new `HealthPredictor` subscribes to `SyncEvent`s and periodically evaluates scan failure rate, pull failure rate, watcher dropped events, folder state flapping, and incremental-scan ratio. When trends look unhealthy it warns and throttles the `FolderOrchestrator`; it recovers throttle automatically when metrics normalize.

### Network / Protocol
- **IndexUpdate chunking**: `index_dispatcher.rs` splits outgoing `IndexUpdate` messages into chunks whose encoded size is ≤ 1 MiB, sends them in sequence, and updates the per-device/folder indexed map after each chunk.
- **Zero-byte file block list fix**: BEP expects every regular file (including 0-byte files) to have at least one block. Scanner now synthesizes a single `size=0` block with `SHA256("")`; `generate_index` and `index_dispatcher` also defensively backfill stale DB entries before sending, preventing Syncthing-Fork v2.1.1 from closing the connection with `protocol error on index: ... file with empty block list`.

### Operations
- **syncthing-monitor rewrite**: split into 8 modules (`args`, `sample`, `api`, `log_parser`, `format`, `alerts`, `util`, `main`), added JSONL telemetry output, REST API polling, daemon log parsing for scan/pull/InvalidFile events, CSV header deduplication, RSS linear-regression prediction, and sync-backlog prediction alerts.

### Stats
- **Tests**: 392 passed / 0 failed / 6 ignored
- **Clippy**: 0 warnings

## [3.0.3] — 2026-06-13

### Headline: TUI stability & modern tray icons.

### TUI
- **F5 daemon toggle lifecycle fix**: `toggle_daemon` now keeps the daemon `JoinHandle`, shows `Stopping...` while the daemon shuts down asynchronously, and blocks restart until the previous instance fully exits to prevent port conflicts.
- **Windows raw mode fix**: Wrapped the TUI body in `tokio::task::LocalSet` so crossterm's thread-local console mode state stays on the same OS thread, eliminating the "Initial console modes not set" error and restoring F5/q keyboard handling.
- **Folder form keyboard navigation**: `Tab`/`Down` from the last text field now enters the "Share with" device list; `BackTab`/`Up` from the first field wraps into the list.

### Tray / UI
- **Modern tray icon set**: Replaced hand-drawn overlapping-circle icons with Phosphor `git-merge` based status icons (gray default, green idle, blue syncing, red error) and matching Phosphor menu icons.

### Stats
- **Tests**: 364 passed / 0 failed / 4 ignored
- **Clippy**: 0 warnings

## [3.0.2] — 2026-06-07

### Headline: Text file three-way merge for conflict resolution.

#### Conflict Resolution
- **Text file auto-merge**: When both sides modify the same `.md`/`.txt`/`.rs`/etc. file concurrently, non-overlapping changes (different paragraphs) are now auto-merged. Overlapping changes (same line) produce git-style conflict markers instead of `.sync-conflict-*` copies. Binary files fall back to RenameBoth (conflict copies). Uses `similar` crate for line-level diff. ([`5bf3fe8`](https://github.com/juice094/syncthing-rust/commit/5bf3fe8))

## [3.0.1] — 2026-06-07

### Headline: §15 stale-index cascade deletion root cause fixed.

#### Bug Fixes
- **§15 root cause**: `index_handler::process_files` was writing remote deletion markers to the local DB even for files never present locally. This caused the scanner to treat those DB entries as "local files were deleted" and emit `LocalIndexUpdated` with deletion flags back to the peer. On a peer with an empty/fresh DB, this triggered a cascade where the local node deleted its own files (the 2043-file mass deletion in KNOWN_ISSUES §15). Fix: ignore remote deletion markers for files that have no local DB record. ([`af229d5`](https://github.com/juice094/syncthing-rust/commit/af229d5))

## [3.0.0] — 2026-06-07

### Headline: Production-grade P2P file sync. BEP Relay v1 activated, sequence race fixed, CI fully green.

This release marks the transition from alpha to production-ready. syncthing-rust is now running bidirectional sync between Windows 11 (ROG-X) and Ubuntu 24.04 VPS (Gray-Cloud) over Tailscale.

### Sync Core
- **Sequence race condition fixed**: `FileSystemDatabase::increment_sequence` read-modify-write was unprotected. `tokio::fs::write` truncates before writing, so concurrent scanner+puller could read an empty file between truncate and write, producing `"Invalid sequence: cannot parse integer from empty string"`. Fixed with per-folder `Mutex` locking the entire read-increment-write cycle. ([`6778837`](https://github.com/juice094/syncthing-rust/commit/6778837))

### BEP Relay v1
- **Relay activation**: Fixed critical ordering bug where relay URLs were stored AFTER the `is_connected` check. On disconnect, `schedule_reconnect` read empty relay URLs → dialed with 0 relay candidates. Relay pool URLs now persisted before the dial decision. ([`7187001`](https://github.com/juice094/syncthing-rust/commit/7187001))
- **10 relay candidates** with parallel dial racing against direct TCP

### CI / Quality
- **Full CI green**: 19/19 jobs passing (was 7 failing). Fixed clippy warnings (10 across 5 files), benchmark compilation (missing `FileInfo` fields), `cargo fmt`, and `cargo-deny` configuration. ([`67ba7a2`](https://github.com/juice094/syncthing-rust/commit/67ba7a2), [`1c60271`](https://github.com/juice094/syncthing-rust/commit/1c60271))
- **deny.toml**: License auditing with allowlist (MIT, Apache-2.0, MPL-2.0, BSD, ISC, etc.) and advisory ignores for unmaintained transitive deps (paste, instant, fxhash via sled)

### Operations
- **Production deployment**: ROG-X (Windows 11, Tailscale 100.107.247.38) ↔ Gray-Cloud (Ubuntu 24.04, Tailscale 100.113.140.121) bidirectional sync
- **WSL cross-compilation**: Linux ELF binary produced in WSL Ubuntu (`cargo build --release`, ~6min), SCP deployed to cloud
- **systemd service**: `/etc/systemd/system/syncthing.service` with auto-restart
- **git bundle disaster recovery**: `git bundle` → SCP → cloud `git clone` to rebuild authoritative workspace after peer format, avoiding stale DB index contamination
- **syncthing-ops skill**: 4-scenario SOP (disaster recovery, anomaly diagnosis, new node deployment, git coexistence)

### Bug Fixes
- **Stale DB index mass deletion**: Peer format/reset left stale index entries in local DB, causing puller to interpret "missing on peer" as "peer deleted it" and batch-delete local files (2043 files confirmed). Recovery SOP established: dual-end DB reset + git bundle restore.
- **Relay health check blocking auto-dial**: TLS deep health check on 10 relays before dial ~2min in campus firewall environments. Known limitation, optimization pending.
- **`.stignore` self-exclusion**: `.stignore` cannot sync itself (line 43 excludes it). Manual SCP documented as standard procedure.

### Versioning & File Preservation
- **SimpleVersioner**: keep=N oldest versions in `.stversions/`
- **StaggeredVersioner**: 4 time windows (30s → 1h → 1d → 1w), maxAge configurable

### Known Limitations
- §1: ClusterConfig first-handshake ~10s timeout (auto-reconnect always succeeds on second cycle)
- §14: High-latency/unstable network block transfer failures (campus firewall)
- §15: Stale DB index after peer format — recovery SOP exists but no automatic detection yet
- Relay health check blocking auto-dial ~2min (network-bound, not code bug)
- `test_udp_broadcast_roundtrip` flakes on macOS CI runners (listener timeout)
- 72h stress test pending (P0 production readiness gate)

### Stats
- **Tests**: 341 passed / 0 failed / 4 ignored
- **CI**: 19 jobs, all green
- **LOC**: ~40K Rust across 14 crates
- **Dependencies**: 0 warnings (clippy, rustc, cargo-deny)

## [0.2.10] — 2026-06-04（生产部署 + 灾备协议）

### Headline: ROG-X ↔ Gray-Cloud 双端工作区同步上线，确立灾备恢复 SOP。

### Operations
- **生产部署**: ROG-X (Windows 11) ↔ Gray-Cloud (Ubuntu 24.04) BEP 双向同步 `.kimi_openclaw/workspace` ↔ `/root/.openclaw/workspace`
- **WSL 交叉编译**: WSL Ubuntu 内 `cargo build --release` 产出 Linux ELF (14MB, 3m32s)，SCP 上传至云端
- **systemd service**: `/etc/systemd/system/syncthing.service`，enabled + auto-restart
- **git bundle 灾备**: `git bundle` → SCP → cloud `git clone` 重建权威 workspace，避免旧索引污染

### Bug Fixes
- **Stale DB index 误删**: 对侧格式化/重装后，本地 DB 残留旧索引导致 (a) puller 按 peer index 判断"对侧已删除"并批量删除本地文件（2043 files confirmed），(b) 对侧请求不存在文件返回 BEP error code 3 风暴。修复：确立双端 DB 重置 + git bundle 恢复协议
- **Relay health check 阻塞 auto-dial**: relay pool TLS 深度健康检查在 auto-dial 之前执行，校网环境下卡住 ~2min。待优化为并行或超时降级

### Configuration
- **init wizard**: `relays_enabled: true`（已完成于 rc3）
- **cloud config**: `local_device_id` 由 daemon 自动从 TLS cert 派生修正
- **Tailscale IP 变更**: 云端格式化后 IP 100.127.13.26 → 100.113.140.121

## [0.2.10-rc3] — 2026-06-03（Phase 0 Complete + Production Readiness）

### Headline: 382 tests, 0 failures. BEP protocol hardened, E2E sync verified, production-ready foundations.

### Protocol
- **Hello→prost**: Replaced 180 lines of hand-written protobuf with `#[derive(prost::Message)]`
- **LZ4 write compression**: Payload compression on the write path (4B size prefix + LZ4 block)
- **wire_compat**: 10 protobuf wire-format conformance tests

### Sync Core
- **rename_with_retry**: 3-layer Windows rename fallback (remove→rename→exponential backoff)
- **Local push on reconnect**: Files created while peer is offline are now pushed on reconnection
- **Scanner logging**: `scanned/new/modified/changed` counters for diagnostics
- **Watcher feedback loop fix**: `.syncthing.*.tmp` events filtered; 5s debounce + 5s min scan gap

### Ignore Patterns
- `**/` arbitrary-depth matching
- `//`→`#` comment syntax (Go Syncthing compatibility)
- `#include` checked before `#` comment parsing

### Versioning
- **New crate**: `syncthing-versioner`
- **SimpleVersioner**: keep=N, `.stversions/` archive, auto-pruning
- **StaggeredVersioner**: 4 time windows (30s/1h/1d/1w), maxAge configurable

### Connection Stability
- **Reconnect backoff fix**: `retry_count` independent map prevents reset to 1
- **TCP keepalive**: SO_KEEPALIVE 60s idle / 10s interval / 3 probes on all connections

### Operations
- **cloud-deploy.sh**: One-command cloud deployment (--full/--compile-only/--deploy-only)
- **Dual-license**: MIT + commercial license available

## [0.2.8] — 2026-05-16（Scanner 元数据排除 + CI/文档清理）

### 🎯 Headline: Scanner 默认排除 syncthing 元数据，消除新手部署陷阱

修复 DUAL_NODE_TEST_2026-05-15 **D-1**：Scanner 不自动排除元数据文件。
同步目录与配置目录重合时，`db/`、`logs/`、`config.json` 等被索引并同步，
导致 `Pull error: Is a directory` 递归灾难。

### 🔧 Fixes
- **Scanner 默认排除列表** (`syncthing-sync/src/scanner.rs`):
  - 硬编码排除 `.stfolder`、`.stversions`、`.stignore`、`config.json`、
    `cert.pem`、`key.pem`、`db`、`logs`
  - 硬编码排除后缀 `.syncthing.tmp`、`~syncthing~`
  - 在 `.stignore` 检查之前生效，确保元数据永不进入索引

### 🏗️ Engineering Debt
- 修复 pre-existing clippy error: `clippy::only_used_in_recursion` (`scanner.rs`)
- 修复 6 个 pre-existing rustdoc warnings（未闭合 HTML tag、私有文档链接、裸 URL）
- 修复 CI workflow 中重复的 `file-size` job 导致 YAML 解析失败
- `cargo fmt --all` 清理 4 个 pre-existing 格式问题文件

### ✅ Verification
- CI 全部 11 个 job 通过（含 Windows Clippy/Test/Release）
- 本地 `cargo test --workspace`：309 passed, 0 failed

---

## [0.2.6] — 2026-05-14（运行时安全 hotfix）

### 🎯 Headline: 运行时安全加固 — 防止资源耗尽雪崩

`INC-20260514-001` 事故复盘后系统性代码审查发现：配置热重载 watcher 无 debounce、
日志无单日上限、多处 `unbounded_channel`、生产代码 `panic!` 路径、部分 loop 无优雅终止。
v0.2.6 为纯修复版本，零功能新增。

### 🔒 Security / Runtime Safety

- **H-1 Config hot-reload debounce** (`daemon_runner.rs`, `config.rs`):
  增加 500ms debounce（重置计时器模式），避免 `notify` 事件风暴导致 100μs 级死循环。
  热重载日志从 `info!` 降为 `debug!`；`JsonConfigStream` 对比 mtime，无变化跳过 reload。
- **H-2 Daemon log rotation by size/hour** (`main.rs`):
  将 `Rotation::DAILY` 改为 `Rotation::HOURLY` 或 size-based（100MB），
  防止单日内日志无限膨胀（事故中 19h 21G）。
- **H-3 Bounded channels** (`syncthing-net`, `syncthing-sync`, `daemon_runner`):
  将 7 处 `unbounded_channel()` 替换为有界 `channel(1024)`，发送端改用 `try_send`，
  满时丢弃并 `warn!`。消除对端恶意发包 / 事件风暴导致的 OOM 风险。
- **H-4 Dropped receiver cleanup** (`relay_listener`, `tcp_transport`, `bep_adapter`):
  移除或修复丢弃 `_event_rx` 但仍用 `event_tx` 发送的无界 channel，避免内存泄漏。
- **H-5 Panic elimination** (`events.rs`, `block_cache`, `derp/server`, `registry`):
  将对外部输入的 `panic!` / `unreachable!()` 全部改为 `error! + Err` 返回。
- **H-6 Graceful shutdown for interval loops** (`daemon_runner`, `discovery_tasks`, `relay_listener`, `api/events`):
  为纯 `loop { interval.tick().await }` 增加 `select! { _ = shutdown.changed() => break }`。

---

## [0.2.5] — 2026-05-13

### 🎯 Headline: End-to-end sync chain unblocked

v0.2.4 stabilized the connection layer (T-F1 deadlock fix, 9h+ stress test);
v0.2.5 now closes the actual file-sync loop. A two-node single-file sync
completes in ~12s end-to-end (TLS → Hello → ClusterConfig → Index → Block →
file materialized on receiver). The previously-`#[ignore]`-pinned
`e2e_sync` diagnostic test is now part of the regular suite.

### 🐛 Critical Bug Fix

- **Runtime-added folders never synchronized** (T2.6):
  `SyncManager::add_folder` created the `FolderModel` but did **not** spawn
  the scan/pull/watcher tokio tasks. Only `SyncManager::start()` did, via
  `start_folder_loops()`. As a result, any folder added at runtime — via
  REST API `POST /rest/config/folders`, TUI commands, or test harnesses —
  was silently inactive. A remote `Index` arriving for such a folder would
  call `folder_model.handle_remote_index()` and `pull_notify.notify_one()`,
  but no task was awaiting the notify, so the signal was dropped on the
  floor. `Puller::pull_folder()` was never invoked, no `Request` was ever
  sent, no `Response` was ever received, no file ever materialized.

  **Fix** (one-line addition in `service::add_folder`): call the idempotent
  `start_folder_internal` after `add_folder_internal`. `start_folder_internal`
  already early-returns if the folder is already running, so this is safe to
  call unconditionally.

  **Verification**:
  - `cargo test --test e2e_sync` — 1 passed in 12.02s (was `#[ignore]`)
  - 295 unit tests still pass
  - 0 clippy warnings

  This was the project's P0 release blocker. See `docs/KNOWN_ISSUES.md` §2
  for the full root-cause writeup.

### 🧹 Refactor

- **T2.3 — `service/mod.rs` business split** (RFC-001): 695 → 60 lines.
  Extracted into four sibling files by responsibility:
  - `lifecycle.rs` (197 lines) — constructors / `start` / `stop` / internal
    helpers
  - `sync_manager.rs` (225 lines) — `impl SyncManager for SyncService`
  - `network_bridge.rs` (119 lines) — BEP transport callbacks
    (`handle_index`, `handle_block_request`, …)
  - `sync_model.rs` (164 lines) — `impl syncthing_core::traits::SyncModel`
    (FFI boundary)

  Pure location refactor — no API surface or runtime semantics changed.
  All field visibility scoped to `pub(super)`. Tests pass unchanged after
  trivially adding `use crate::model::SyncManager;` to `tests.rs`.

- **T2.5 — `TestNode` BEP bridge harness**:
  New `crates/syncthing-test-utils/src/bep_bridge.rs` (344 lines) wires a
  `TestBepHandler` + `TestBlockSource` on top of `ConnectionManager`
  callbacks, so integration tests can drive the full BEP pipeline
  (ClusterConfig + Index + Block) instead of stopping at the Hello
  handshake. Enabled the `e2e_sync` regression test.

### 🛠 Stress Test Tooling (T2.2)

- **Log rotation**: `bin/stress_test.rs` now uses
  `tracing_appender::rolling` for daily log files (keep 7 days), preventing
  multi-day runs from accumulating gigabyte-scale log files. Added
  `--log-dir <PATH>` CLI flag (defaults to `stress-logs/`).
- **CSV timestamps**: Replaced the broken hand-rolled `fmt_system_time` (which
  produced strings like `"20585T05:07:55Z"`) with `chrono` ISO 8601
  formatting. Stress-test report CSVs are now machine-parseable.

### 📚 Documentation

- **`docs/KNOWN_ISSUES.md`**: §2 (end-to-end sync broken) marked as fixed,
  with the actual root cause documented (not the originally-suspected
  puller/index_handler bug, but the `add_folder` loop-spawn omission).
- **`docs/reports/STRESS_TEST_REPORT_2026-05-13.md`**: 191-line analysis of
  the 9h11m stress run that died from Windows desktop sleep, with the
  observation that originally raised T2.5 ("0 ClusterConfig events in
  9 hours — harness never drives BEP session").
- **`docs/operations/TAILSCALE_GUIDE.md` (225 lines)**: Three paths to
  Tailscale-based NAT traversal — zero-code (use Tailscale as L3),
  embedded DERP client, or run own DERP server (project already has 1480
  lines of DERP impl).
- **`docs/operations/PROXY_GUIDE.md` (198 lines)**: How to use the
  existing `SOCKS5_PROXY` / `HTTP_PROXY` env-var support in
  `transport/proxy.rs` with Watt Toolkit, clash, or similar tools.
- **`docs/drafts/RFC-001-service-split.md`**: T2.3 architecture decision
  record (split rationale, target file structure, verification checklist).
- **`README.md` / `README-zh.md`**: Updated stage banner from
  "early alpha / not production-ready" to "alpha — core sync chain
  verified". End-to-end sync row in the at-a-glance table flipped from ❌
  to ✅.

### 🗂 Project Hygiene

- **Phase A file splits — 6/6 complete** (T1.1–T1.6):
  - `dialer.rs` 621 → 452 + tests 168 (`90e6d3b`)
  - `block_cache.rs` 556 → 322 + tests 234 (`5fd42b0`)
  - `types/mod.rs` 639 → 477 + `types/folder.rs` 176 (`f21c8fe`)
  - `daemon_runner.rs` 596 → 476 + helpers 176 (`0140851`)
  - `traits.rs` 574 → 463 + `traits/transport.rs` 127 (`66589c2`)
  - `service/mod.rs` 695 → 60 + 4 children 705 (T2.3 above)

- **Clippy nursery — `manual_let_else`** enabled and applied across 10
  sites in 9 files (T1.4, `921cf0f`).

### ⚠️ Known Limitations (unchanged from v0.2.4)

- **§1 ClusterConfig first-handshake 10s timeout**: first connection cycle
  is delayed ~12s due to a known race. Auto-reconnect always succeeds on
  the second cycle. Tracked in `docs/KNOWN_ISSUES.md`.
- 72h stress test on Windows desktop infeasible (sleep kills nohup
  children); awaits Linux platform repeat.
- Go Syncthing cross-version interop: hand-tested once on 2026-04-11; no
  automation.

---

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
