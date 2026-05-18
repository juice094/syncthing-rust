# syncthing-rust

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![Tests](https://img.shields.io/badge/tests-309%20passed-brightgreen)]()
[![Clippy](https://img.shields.io/badge/clippy-0%20warnings-brightgreen)]()
[![Version](https://img.shields.io/badge/version-v0.2.9--rc2-blue)]()
[![License](https://img.shields.io/badge/license-MIT-blue)](./LICENSE)

A Rust implementation of the [Syncthing](https://syncthing.net/) protocol stack, designed for **zero-runtime-dependency** deployment and wire-compatible interoperability with the official Go Syncthing daemon.

**📦 Latest Release: [v0.2.9-rc2](https://github.com/juice094/syncthing-rust/releases/tag/v0.2.9-rc2)** — Centralized Constants + Transport Plugin RFC + Dual-Node E2E Infrastructure

| Platform | Binary | Size |
|----------|--------|------|
| Windows x64 | [`syncthing-v0.2.8-x86_64-pc-windows-msvc.exe`](https://github.com/juice094/syncthing-rust/releases/download/v0.2.8/syncthing-v0.2.8-x86_64-pc-windows-msvc.exe) | ~12 MB |
| Linux x64 | [`syncthing-v0.2.8-x86_64-unknown-linux-gnu`](https://github.com/juice094/syncthing-rust/releases/download/v0.2.8/syncthing-v0.2.8-x86_64-unknown-linux-gnu) | ~13 MB |
| Stress Test (Win) | [`stress_test-v0.2.8-x86_64-pc-windows-msvc.exe`](https://github.com/juice094/syncthing-rust/releases/download/v0.2.8/stress_test-v0.2.8-x86_64-pc-windows-msvc.exe) | ~6.6 MB |
| Stress Test (Linux) | [`stress_test-v0.2.8-x86_64-unknown-linux-gnu`](https://github.com/juice094/syncthing-rust/releases/download/v0.2.8/stress_test-v0.2.8-x86_64-unknown-linux-gnu) | ~6.8 MB |
| Checksums | [`SHA256SUMS.txt`](https://github.com/juice094/syncthing-rust/releases/download/v0.2.8/SHA256SUMS.txt) | — |

> 💡 [Build from source](#quick-start) if you need a different target or want to audit the code.

> ⚠️ **Current stage (2026-05-18 post-v0.2.9-rc2)**: **Alpha — WSL2↔Windows sync verified, real-network Tailscale E2E validated (one-way), Transport Plugin RFC drafted, centralized constants shipped.**
>
> - ✅ **Connection layer stable**: 12h+ single-node endurance, 0 deadlocks, 0 panics, logs 6 KB (v0.2.6 hardening verified).
> - ✅ **Protocol layer correct**: TLS, BEP Hello, ClusterConfig, Index encode/decode all working.
> - ✅ **End-to-end sync working** (T2.6 fix, 2026-05-13): `e2e_sync` test passes. Single-file two-node sync completes in ~12s on loopback.
> - ✅ **Runtime safety hardened** (v0.2.6, INC-20260514-001): config hot-reload debounce, log rotation by size/hour, all channels bounded, zero `panic!` on external input, graceful shutdown on all loops.
> - ✅ **Cross-version interop verified** (2026-05-14): Rust v0.2.8 ↔ Go v2.1.0, TLS + BEP + file sync all pass. `scripts/cross_version_test.sh` automation ready.
> - ✅ **WSL2↔Windows dual-node sync verified** (2026-05-18): Loopback two-node bidirectional file sync validated on same host. [`docs/reports/WSL2_WINDOWS_DUAL_NODE_E2E_2026-05-18.md`](docs/reports/WSL2_WINDOWS_DUAL_NODE_E2E_2026-05-18.md).
> - ✅ **Real-network Tailscale E2E validated** (2026-05-18): Windows 11 ↔ Ubuntu 24.04 via Tailscale (`100.107.247.38` ↔ `100.127.13.26`). BEP handshake + one-way file sync confirmed. [`docs/reports/REAL_NETWORK_DUAL_NODE_E2E_2026-05-18.md`](docs/reports/REAL_NETWORK_DUAL_NODE_E2E_2026-05-18.md).
> - ✅ **Transport Plugin RFC drafted** (v0.3.0 P0): Per-scheme dialing, config-driven transport registration, address pipeline preserving scheme. [`docs/rfc/TRANSPORT_PLUGIN_RFC.md`](docs/rfc/TRANSPORT_PLUGIN_RFC.md).
> - ✅ **Scanner metadata exclusion** (D-1, v0.2.8): `.stfolder`, `.stversions`, `.stignore`, `config.json`, `cert.pem`, `key.pem`, `db/`, `logs/`, `*.syncthing.tmp` auto-excluded from sync.
> - ✅ **Prometheus metrics endpoint** (`/metrics`): 9 metrics verified (build info, uptime, connected devices, bytes sent/received, folder file counts).
> - ✅ **Centralized constants** (`syncthing-core/src/constants.rs`): All magic numbers (22001, 8385) consolidated, 6+ hardcoded sites eliminated.
> - ⏳ **72h endurance test** pending: single-node 12h validated; WSL2↔Windows bidirectional + real-network one-way validated; **72h real-network endurance** required for v0.3.0 admission.
>
> **Not yet a drop-in Go Syncthing replacement**, and **not yet enterprise-ready** (no FIPS/SM crypto, no audit logging). Suitable for: BEP protocol research, Rust reference reading, controlled experiments, personal private deployment, and contributing fixes.

---

## At a Glance

| Dimension | State |
|-----------|-------|
| BEP Protocol (TLS + Hello + ClusterConfig + Index + Request/Response) | ✅ Codec + handshake verified |
| **End-to-end file sync (A→B actual transfer)** | ✅ **Working** (`e2e_sync` test passes; T2.6 fix) |
| File-sync internal modules (puller / scanner / folder_model) unit tests | ✅ 309 unit tests passing |
| Network Discovery (Local + Global + STUN + UPnP + Relay v1) | ✅ Implementation complete; ParallelDialer with RTT scoring |
| REST API (read + write, Go-layout compatible) | ✅ Read + write complete |
| Tests | **309 unit + 2 e2e passing, 4 ignored** |
| Lint | **0 clippy warnings** (incl. `await_holding_lock` + `manual_let_else`) |
| Security audit | **3 unmaintained** upstream transitive deps (accepted debt, see `.cargo/audit.toml`) |
| Binary size | ~12 MB (release, Windows x64) |

> **Current limitations (must read)**:
> - **§1 ClusterConfig first-handshake 10s timeout**: first connection cycle is delayed ~12s due to a known race (auto-reconnect always succeeds on the second cycle). **Mitigated** by T3.1b `reconnect_device` API + 60s session health check (v0.2.6).
> - **§8 Campus/enterprise firewall blocking**: default BEP TCP 22001 is blocked by many campus/enterprise egress firewalls. **Workaround**: use Tailscale/Headscale/WireGuard virtual overlay (test in progress, see `docs/plans/NEXT_STEPS_2026-05-15.md`). Native Transport plugin support is v0.3.0 P0.
> - ~~§7 Runtime safety gaps~~ **Resolved in v0.2.6**: config hot-reload debounce (500ms), log rotation by size/hour, all channels bounded, zero `panic!` on external input, graceful shutdown on all loops. See [`docs/KNOWN_ISSUES.md`](./docs/KNOWN_ISSUES.md) §7 for post-fix audit.
> - 72h stress test: single-node Linux validated (12h). **Dual-node real-network 72h** required for v0.3.0 admission (pending Headscale/Tailscale deployment).
> - ~~Go Syncthing full file-sync interoperability~~ **Verified** (2026-05-14): Rust v0.2.6 ↔ Go v2.1.0, automated via `scripts/cross_version_test.sh`.

---

## Quick Start (Windows)

```powershell
# 1. Build release binary (< 1 min on modern hardware)
cargo build --release -p syncthing

# 2. Run with interactive TUI
cargo run --release -p syncthing -- tui

# 3. Or run headless
cargo run --release -p syncthing -- run
```

First run generates an Ed25519 TLS certificate and stores it in `%LOCALAPPDATA%\syncthing-rust`.

Default ports: BEP `22001`, REST API `8385`. Loopback addresses bypass API key auth for local debugging.

### Verify it works

```powershell
# Check REST health
curl http://127.0.0.1:8385/rest/system/status | ConvertFrom-Json

# Expected: uptime > 0, folders/devices counts match your config
```

### Stress Test & Resilience Validation

A built-in 72-hour stress-test binary exercises continuous file injection, network fault injection, and memory profiling:

```powershell
# Quick 5-minute validation run
cargo run --release -p syncthing --bin stress_test -- --duration 5m --report smoke.csv

# Full 72-hour unattended run with auto-resume on reboot (via Windows Scheduled Task)
# See scripts\register-stress-task.ps1 for one-click registration
```

Full tuning plan: [`docs/plans/TUNING_PLAN_2026-05-11.md`](docs/plans/TUNING_PLAN_2026-05-11.md).

---

## What It Does (and Doesn't)

**Does**
- Establish TLS-encrypted BEP sessions with official Go Syncthing peers.
- Pull files block-by-block via `Request`/`Response` and reassemble locally.
- Passively serve block requests (upload) to connected peers.
- Scan local folders, compute SHA-256 block hashes, broadcast `IndexUpdate`.
- Watch filesystem changes (`notify` + 1s debounce → scan → broadcast in ~2s).
- Discover peers via LAN UDP broadcast, Global Discovery (HTTPS mTLS), STUN, UPnP, and Syncthing Relay v1.
- Parallel dialer races direct TCP and Relay candidates with RTT scoring.
- Expose a REST API (Go-layout compatible) with read + write endpoints (config, pause/resume, scan, restart/shutdown).
- TUI real-time sync state (folder states, device connections, sync progress) via event bridge.
- Hot-reload `config.json` changes without restart (notify-based watcher).

**Doesn't (yet)**
- Active Push scheduling (scanning triggers local index update, but does not proactively ask peers to pull).
- Web GUI (TUI only).
- QUIC transport.
- Production packaging (systemd service / MSI installer).

---

## Roadmap

| Phase | Goal | Status |
|-------|------|--------|
| **Phase 1** | Core BEP protocol (TLS, Hello, ClusterConfig, Index) | ✅ Complete |
| **Phase 2** | Network abstraction, watcher, REST API, dual-node coexistence | ✅ Complete |
| **Phase 3** | BepSession observability, Push/Pull E2E with remote peer | ✅ Complete (verified against Go v2.1.0, 2026-05-14) |
| **Phase 3.5** | Connection stability, config persistence | ✅ Complete |
| **Phase 4** | TUI hardening (event bridge, live sync state, config hot-reload) | ✅ Complete |
| **Phase 5** | Zero-Tailscale interconnection (discovery results → ConnectionManager address pool) | ✅ Tailscale穿透验证完成 (2026-05-16); Windows ↔ Linux 双向文件同步通过 |
| **Phase A** | Security debt acceptance (cargo audit) | ✅ Complete (`.cargo/audit.toml` created) |
| **Phase B** | 72h stress test | 🟡 Single-node 12h validated; dual-node real-network 功能验证通过 (2026-05-16); **72h endurance run** pending |
| **Phase C** | REST API write-path closure | ✅ Complete (override/revert implemented, scan `sub` supported, device pause/resume body active) |
| **Phase D** | Observability infrastructure (Prometheus metrics, structured logging) | 🔵 `/metrics` endpoint implemented (9 metrics); deployment validation pending |

Phase 5 design: [`docs/design/NETWORK_DISCOVERY_DESIGN.md`](docs/design/NETWORK_DISCOVERY_DESIGN.md).

Current roadmap: [`docs/plans/POST_V0_2_0_ROADMAP.md`](docs/plans/POST_V0_2_0_ROADMAP.md) — prioritized execution plan (P0~P5).
Plan index: [`docs/plans/INDEX.md`](docs/plans/INDEX.md) — navigation across all plan documents.
Plan audit: [`docs/plans/PLAN_AUDIT_2026-04-27.md`](docs/plans/PLAN_AUDIT_2026-04-27.md) — validity assessment of all historical plans.

---

## Architecture

```
cmd/syncthing/          # CLI entry point + TUI
crates/
├── syncthing-core/     # DeviceId, FileInfo, VersionVector — stable, read-only boundary
├── bep-protocol/       # BEP Hello, Request/Response, Index, ClusterConfig
├── syncthing-net/      # TCP+TLS, ConnectionManager, dialer, discovery, relay
├── syncthing-sync/     # SyncService, Scanner, Puller, IndexHandler, watcher
├── syncthing-api/      # REST API server (Axum)
└── syncthing-db/       # Metadata & block cache abstractions
docs/
├── design/             # Active ADRs and network design
├── plans/              # Roadmaps and improvement plans
├── reports/            # Verification reports, implementation summaries
└── archive/            # Historical decisions
```

> **Trust boundary**: `syncthing-core` is read-only for downstream crates. See [`docs/design/ARCHITECTURE_DECISIONS.md`](docs/design/ARCHITECTURE_DECISIONS.md).

---

## Documentation

| Document | Purpose |
|----------|---------|
| [`docs/README.md`](docs/README.md) | Documentation navigation |
| [`docs/design/ARCHITECTURE_DECISIONS.md`](docs/design/ARCHITECTURE_DECISIONS.md) | Architecture Decision Records (ADRs) |
| [`docs/design/NETWORK_DISCOVERY_DESIGN.md`](docs/design/NETWORK_DISCOVERY_DESIGN.md) | Network discovery layer design |
| [`docs/reports/IMPLEMENTATION_SUMMARY.md`](docs/reports/IMPLEMENTATION_SUMMARY.md) | Crate-level implementation status |
| [`docs/reports/WSL2_WINDOWS_DUAL_NODE_E2E_2026-05-18.md`](docs/reports/WSL2_WINDOWS_DUAL_NODE_E2E_2026-05-18.md) | WSL2↔Windows loopback dual-node sync verification |
| [`docs/reports/REAL_NETWORK_DUAL_NODE_E2E_2026-05-18.md`](docs/reports/REAL_NETWORK_DUAL_NODE_E2E_2026-05-18.md) | Real-network Tailscale dual-node E2E test report |
| [`docs/reports/VERIFICATION_REPORT_BEP_2026-04-11.md`](docs/reports/VERIFICATION_REPORT_BEP_2026-04-11.md) | BEP interoperability test report |
| [`docs/design/FEATURE_COMPARISON.md`](docs/design/FEATURE_COMPARISON.md) | Feature parity with Go Syncthing |
| [`docs/plans/INDEX.md`](docs/plans/INDEX.md) | Plan document navigation and cross-references |
| [`docs/plans/PLAN_AUDIT_2026-04-27.md`](docs/plans/PLAN_AUDIT_2026-04-27.md) | Plan validity audit and project stage recalibration |
| [`docs/plans/TUNING_PLAN_2026-05-11.md`](docs/plans/TUNING_PLAN_2026-05-11.md) | Performance / stability / architecture-debt tuning plan (cross-cut with POST_V0_2_0) |
| [`docs/ai-protocol.md`](docs/ai-protocol.md) | Cross-session state anchor for AI agents |

---

## Performance & Tuning Status

| Indicator (2026-05-15 audit) | Value | Action |
|------------------------------|-------|--------|
| Source lines (Rust) | ~32,000 / ~160 files | — |
| Files exceeding 600-line soft cap | 12 | T-E (planned) |
| `unwrap()/expect()` occurrences | ~720 (incl. tests) / **~368 production** | T-F2 (P2) |
| Scanner SHA-256 parallelism | Single-threaded | T-B1 (P1) |
| `FileSystemDatabase` storage | Per-file JSON (O(N) syscalls) | T-C (v0.4.0) |
| criterion benchmarks | **Skeleton ready** (4 benches) | T-A1 (P1) |
| 72h stress test | Single-node 12h validated; dual-node real-network功能验证通过 (2026-05-16); **72h endurance run** pending | T-F1 (P0) |
| Cross-version interop | **Verified** (Rust v0.2.8 ↔ Go v2.1.0) | Automation in `scripts/cross_version_test.sh` |
| Prometheus metrics | **`/metrics` endpoint implemented** (build info, uptime, devices, bytes, folder files) | O-1 (v0.3.0 P0, code done) |
| Transport plugin abstraction | **None** | T-Net-2 (v0.3.0 P0) |
| Enterprise/FIPS/SM crypto | **None** | v0.4.0 roadmap |

Full breakdown: [`docs/plans/TUNING_PLAN_2026-05-11.md`](docs/plans/TUNING_PLAN_2026-05-11.md).

The tuning plan is **horizontally complementary** to [`POST_V0_2_0_ROADMAP.md`](docs/plans/POST_V0_2_0_ROADMAP.md) — they share the same P0 (72h stability) but the tuning plan adds measurement infrastructure, hot-path performance work, and architecture-debt containment.

---

## Contributing

See [`CONTRIBUTING.md`](./CONTRIBUTING.md). Short version:

```powershell
# Quick validation
cargo test --workspace          # must pass: 309 passed
cargo clippy --workspace --all-targets  # must be 0 warnings

# Or run the local health check script (Windows)
.\scripts\check-health.ps1
```

---

## License

[MIT License](./LICENSE).
