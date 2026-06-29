# Releases

## v3.0.4 — Security Hardening & Relay Server (2026-06-27)

### Headline
Production security audit with 8 CRITICAL/HIGH fixes, self-hosted BEP Relay Server v1, enterprise RBAC, structured JSON logging, and full Docker/K8s deployment.

### Security
- Path traversal defense: `validate_remote_name()` on all remote file entries
- SSRF protection: `validate_outbound_addr()` filters unsafe address types
- Connection flood defense: `max_connections` enforced at TCP accept
- Key file permissions: Unix `set_permissions(0o600)` on cert/key/config
- Content leakage: all `eprintln!` in production replaced with tracing
- Mass-deletion safety: IndexHandler 50% threshold against cascade delete
- quinn-proto RUSTSEC-2026-0185 (CVSS 7.5) fixed

### New Features
- **Relay Server v1**: Protocol mode (TLS + bep-relay ALPN) + Session mode with bidirectional forwarding. CLI: `syncthing relay-server`
- **WSS transport**: TLS-over-WebSocket reusing device certificates
- **RBAC**: admin + read-only API keys, ro_key returns 403 on writes
- **JSON logging**: `--log-format json` for ELK/Splunk/Loki aggregation
- **Prometheus metrics**: 7 metrics + deep `/rest/health` checks
- **Grafana dashboard**: 9-panel monitoring dashboard
- **7 Prometheus alert rules**: connectivity, sync, DB, auth, uptime

### Deploy
- Docker multi-stage build + compose for relay server
- nginx reverse proxy config for port 443 sharing
- K8s Helm chart with ServiceMonitor
- Disaster recovery runbook
- CycloneDX SBOM generation script

### Stats
- 413 tests / 0 failures / 3 ignored
- 3 benchmark suites: Puller 6.7 GiB/s, Scanner 803µs/MB, BEP 198ns/Hello
- 14MB release binary (Linux/macOS/Windows)

## v3.0.3 — TUI Stability & Modern Tray Icons (2026-06-13)

### Headline
TUI daemon lifecycle hardened, Windows raw mode fixed, modern Phosphor-style tray icon set.

### What's New
- **TUI F5 daemon toggle lifecycle fix**: non-blocking stop with `JoinHandle` management; restart blocked until previous daemon fully exits.
- **Windows raw mode fix**: TUI body wrapped in `tokio::task::LocalSet` so crossterm thread-local console mode stays on the same OS thread.
- **Folder form keyboard navigation**: Tab/Down from the last field enters the "Share with" device list; BackTab/Up wraps from the first field.
- **Modern tray icon set**: Phosphor `git-merge` based status icons + 16×16 menu item icons.

### Stats
- **Tests**: 364 passed / 4 ignored / 0 failed
- **Clippy**: 0 warnings
- **CI**: 19/19 jobs passing

## v3.0.2 — Text Three-Way Merge (2026-06-07)

### Headline
Text files can now be auto-merged; overlapping edits produce git-style conflict markers instead of silent conflict copies.

### What's New
- **Three-way text merge**: base/local/remote merge using `similar::TextDiff`; non-overlapping changes auto-merged.
- **Conflict markers**: overlapping line modifications produce git-style `<<<<<<< / ======= / >>>>>>>` markers.
- **Binary fallback**: binary files fall back to RenameBoth (`.sync-conflict-*`).

### Stats
- **Tests**: 364 passed / 4 ignored / 0 failed
- **Clippy**: 0 warnings

## v3.0.1 — Stale-Index Cascade Deletion Fix (2026-06-07)

### Headline
Root cause of §15 stale-index mass-deletion fixed.

### What's Fixed
- `index_handler::process_files` no longer writes remote deletion markers for files that have no local DB record, preventing cascade deletion after a peer format/reinstall.

### Stats
- **Tests**: 364 passed / 4 ignored / 0 failed
- **Clippy**: 0 warnings

## v3.0.0 — Production-Grade P2P File Sync (2026-06-07)

### Headline
BEP Relay v1 activated, sequence race fixed, CI fully green; project transitions from alpha to production-ready.

### What's New
- **Sequence race fix**: per-folder `Mutex` around `FileSystemDatabase::increment_sequence` read-modify-write.
- **Relay v1 activation**: relay pool URLs persisted before dial decision; 10 relay candidates race against direct TCP.
- **Production deployment**: ROG-X (Windows 11) ↔ Gray-Cloud (Ubuntu 24.04) bidirectional sync over Tailscale.
- **WSL cross-compile + systemd service**: cloud-deploy.sh one-command Linux deployment.
- **git bundle disaster recovery**: SOP for peer format/reset scenarios.
- **Simple + Staggered versioning**: `.stversions/` archive with configurable retention.

### Quality
- **CI**: 19/19 jobs passing (clippy, test, deny, fmt, doc, e2e, bench-smoke, release-check, file-size).
- **Tests**: 341 passed / 4 ignored / 0 failed at release; current workspace 392 passed / 6 ignored / 0 failed.
- **Clippy**: 0 warnings.

### Known Limitations
- §1 ClusterConfig first-handshake timeout mitigated by auto-reconnect.
- §14 High-latency/unstable network block transfer failures.
- §15 Stale DB index after peer format — recovery SOP exists, automatic detection pending.
- 72h stress test pending (v3.1.0 readiness gate).

---

## v0.2.10-rc3 — Phase 0 Complete + Production Readiness (2026-06-03)

### Protocol
- Hello→prost: 删除 180 行手写 protobuf, `#[derive(prost::Message)]`
- LZ4 写入压缩: 4B 原始长度 + LZ4 block
- `wire_compat.rs`: 10 个 BEP 协议一致性测试

### Sync Core
- `rename_with_retry`: Windows 3 层 rename 回退 (remove→rename→指数退避×5)
- 本地文件主动推送: index_handler 发布 LocalIndexUpdated
- Scanner 增强日志: scanned/new/modified/changed 计数器
- Watcher 反馈循环修复: `.syncthing.*.tmp` 过滤 + 5s debounce

### Versioning (New Crate)
- `syncthing-versioner`: Simple (keep=N) + Staggered (4 时间窗口)

### Connection Stability
- `retry_count` 独立 map (不再被重置为 1)
- TCP keepalive (SO_KEEPALIVE 60s/10s/3probes)

### Tests: 382 passed, 0 failed

## v0.2.6 — Runtime Safety Hardening (2026-05-14)

### 🔒 Security & Stability Hotfix

This is a **pure-fix release** with zero new features. It addresses the root causes identified in post-incident review `INC-20260514-001` (config hot-reload watcher entered a tight loop under notify event storms, producing 21 GB of logs in 19 hours).

**Six hardening patches applied (H-1 through H-6):**

| Patch | File(s) | What changed |
|-------|---------|--------------|
| **H-1** | `daemon_runner.rs`, `config.rs` | Config hot-reload now has a **500 ms debounce** (resetting timer) and mtime comparison. Hot-reload logs downgraded from `info!` → `debug!`. |
| **H-2** | `main.rs` | Daemon log rotation switched from **daily → hourly / 100 MB size cap**, preventing single-day log explosion. |
| **H-3** | `syncthing-net`, `syncthing-sync`, `daemon_runner` | All 7 `unbounded_channel()` sites replaced with **bounded `channel(1024)`**. Senders use `try_send`; full queues drop and emit `warn!`. |
| **H-4** | `relay_listener`, `tcp_transport`, `bep_adapter` | Cleaned up dropped-receiver patterns that leaked memory. |
| **H-5** | `events.rs`, `block_cache`, `derp/server`, `registry` | All `panic!` / `unreachable!()` on **external input** converted to `error! + Err(...)`. |
| **H-6** | `daemon_runner`, `discovery_tasks`, `relay_listener`, `events` | Pure `interval.tick()` loops now have `select! { _ = shutdown.changed() => break }` for graceful termination. |

### 🔬 Cross-Version Interoperability Verified

Rust v0.2.6 ↔ Go Syncthing v2.1.0 **fully verified** on local loopback (2026-05-14):

- TLS 1.3 handshake (`TLS_AES_128_GCM_SHA256`) ✅
- BEP Hello exchange ✅
- ClusterConfig exchange ✅
- Index broadcast / receive ✅
- Block Request / Response ✅
- File materialization on receiver ✅

Automation: `scripts/cross_version_test.sh` (281 lines, Linux/Windows dual-platform).

### 📊 Quality Metrics

```
cargo test --workspace        → 296 passed, 0 failed, 1 ignored (e2e_sync)
cargo clippy --workspace      → 0 warnings (-D warnings)
cargo machete                 → 0 unused deps (4 removed in this cycle)
criterion benches             → 4 skeletons (device_id, scanner, hash_parallel, encode_decode)
```

### 📦 Binaries

| Platform | Target | File | Size |
|----------|--------|------|------|
| Windows | x86_64-pc-windows-msvc | `syncthing-v0.2.6-x86_64-pc-windows-msvc.exe` | ~12 MB |
| Linux | x86_64-unknown-linux-gnu | `syncthing-v0.2.6-x86_64-unknown-linux-gnu` | ~11 MB |
| Stress test | x86_64-pc-windows-msvc | `stress_test-v0.2.6-x86_64-pc-windows-msvc.exe` | ~6.6 MB |

> **Build from source** (any platform with Rust 1.85+):
> ```bash
> cargo build --release -p syncthing
> ```

### ⚠️ Known Limitations

- **§1 ClusterConfig race**: First handshake may stall ~10 s; auto-reconnect succeeds on second cycle. Mitigated by 60 s session health check. `e2e_sync` integration test marked `#[ignore]` under parallel load.
- **72 h endurance**: Scripts ready (`scripts/72h_*.sh`), Linux deployment pending.
- **Tailscale field validation**: Core discovery integration complete; real-world CGNAT/MTU testing deferred.

### 🔗 References

- Full incident write-up: `docs/plans/NEXT_STEPS_2026-05-14.md` §0
- Known issues registry: `docs/KNOWN_ISSUES.md`
- Tuning plan: `docs/plans/TUNING_PLAN_2026-05-11.md`
- Cross-version test script: `scripts/cross_version_test.sh`
