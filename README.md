# syncthing-rust

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![Tests](https://img.shields.io/badge/tests-308%20passed-brightgreen)]()
[![Clippy](https://img.shields.io/badge/clippy-0%20warnings-brightgreen)]()
[![License](https://img.shields.io/badge/license-MIT-blue)](./LICENSE)

A Rust implementation of the [Syncthing](https://syncthing.net/) protocol stack, designed for **zero-runtime-dependency** deployment and wire-compatible interoperability with the official Go Syncthing daemon.

> ⚠️ **Current stage (2026-05-13 recalibration)**: **Early alpha / not production-ready.**
>
> - ✅ **Connection layer stable**: 9h+ stress test, 758 connection cycles, 0 deadlocks, 0 panics (T-F1 fix verified).
> - ✅ **Protocol layer correct**: TLS, BEP Hello, ClusterConfig, Index encode/decode all working.
> - ❌ **End-to-end sync does NOT complete**: The new `e2e_sync` diagnostic test (2026-05-13) exposes a missing trigger in the puller / index_handler chain (see [`docs/KNOWN_ISSUES.md`](./docs/KNOWN_ISSUES.md) §2). Files do **not** actually transfer between nodes until this is fixed in v0.2.5.
> - ⏳ **72h endurance test** and **cross-version interop automation** are both incomplete.
>
> **Do not use this project as a Go Syncthing replacement yet.** It is currently suitable only for: BEP protocol research, Rust reference reading, and contributing fixes.

---

## At a Glance

| Dimension | State |
|-----------|-------|
| BEP Protocol (TLS + Hello + ClusterConfig + Index + Request/Response) | ✅ Codec + handshake verified |
| **End-to-end file sync (A→B actual transfer)** | ❌ **Broken** (`e2e_sync` fails to materialize file on B; KNOWN_ISSUES §2) |
| File-sync internal modules (puller / scanner / folder_model) unit tests | ✅ 295 unit tests passing |
| Network Discovery (Local + Global + STUN + UPnP + Relay v1) | ✅ Implementation complete; ParallelDialer with RTT scoring |
| REST API (read + write, Go-layout compatible) | ✅ Read + write complete |
| Tests | **295 unit + 1 e2e passed, 1 ignored (e2e_sync diagnostic)** |
| Lint | **0 clippy warnings** (incl. `await_holding_lock` + `manual_let_else`) |
| Security audit | **3 unmaintained** upstream transitive deps (accepted debt, see `.cargo/audit.toml`) |
| Binary size | ~12 MB (release, Windows x64) |

> **Current limitations (must read)**:
> - **§2 end-to-end sync broken**: the project's core promise (file synchronization) is not working. See [`docs/KNOWN_ISSUES.md`](./docs/KNOWN_ISSUES.md).
> - **§1 ClusterConfig first-handshake 10s timeout**: connection stability impacted (auto-reconnect saves it).
> - 72h stress test on Windows desktop is infeasible (sleep kills nohup children); requires Linux.
> - Go Syncthing full file-sync interoperability was hand-tested once on 2026-04-11, no automation.

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
| **Phase 3** | BepSession observability, Push/Pull E2E with remote peer | ✅ Complete (verified against earlier pre-fix Rust build; Go node pending) |
| **Phase 3.5** | Connection stability, config persistence | ✅ Complete |
| **Phase 4** | TUI hardening (event bridge, live sync state, config hot-reload) | ✅ Complete |
| **Phase 5** | Zero-Tailscale interconnection (discovery results → ConnectionManager address pool) | 🔵 Core integrated; field validation pending |
| **Phase A** | Security debt acceptance (cargo audit) | ✅ Complete (`.cargo/audit.toml` created) |
| **Phase B** | 72h stress test | 🟡 In progress (started 2026-05-11; auto-resume via Windows Scheduled Task) |
| **Phase C** | REST API write-path closure | ✅ Complete (override/revert implemented, scan `sub` supported, device pause/resume body active) |

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
| [`docs/reports/VERIFICATION_REPORT_BEP_2026-04-11.md`](docs/reports/VERIFICATION_REPORT_BEP_2026-04-11.md) | BEP interoperability test report |
| [`docs/design/FEATURE_COMPARISON.md`](docs/design/FEATURE_COMPARISON.md) | Feature parity with Go Syncthing |
| [`docs/plans/INDEX.md`](docs/plans/INDEX.md) | Plan document navigation and cross-references |
| [`docs/plans/PLAN_AUDIT_2026-04-27.md`](docs/plans/PLAN_AUDIT_2026-04-27.md) | Plan validity audit and project stage recalibration |
| [`docs/plans/TUNING_PLAN_2026-05-11.md`](docs/plans/TUNING_PLAN_2026-05-11.md) | Performance / stability / architecture-debt tuning plan (cross-cut with POST_V0_2_0) |
| [`docs/ai-protocol.md`](docs/ai-protocol.md) | Cross-session state anchor for AI agents |

---

## Performance & Tuning Status

| Indicator (2026-05-11 audit) | Value | Action |
|------------------------------|-------|--------|
| Source lines (Rust) | 31,046 / 152 files | — |
| Files exceeding 600-line soft cap | 12 | T-E (planned) |
| `unwrap()/expect()` occurrences | 718 (incl. tests) | T-F2 (planned) |
| Scanner SHA-256 parallelism | Single-threaded | T-B1 (planned) |
| `FileSystemDatabase` storage | Per-file JSON (O(N) syscalls) | T-C (planned) |
| criterion benchmarks | **None** | T-A1 (P0 prerequisite) |
| 72h stress test | **In progress** (started 2026-05-11) | T-F1 (P0, running with fault injection) |

Full breakdown: [`docs/plans/TUNING_PLAN_2026-05-11.md`](docs/plans/TUNING_PLAN_2026-05-11.md).

The tuning plan is **horizontally complementary** to [`POST_V0_2_0_ROADMAP.md`](docs/plans/POST_V0_2_0_ROADMAP.md) — they share the same P0 (72h stability) but the tuning plan adds measurement infrastructure, hot-path performance work, and architecture-debt containment.

---

## Contributing

See [`CONTRIBUTING.md`](./CONTRIBUTING.md). Short version:

```powershell
# Quick validation
cargo test --workspace          # must pass: 308 passed
cargo clippy --workspace --all-targets  # must be 0 warnings

# Or run the local health check script (Windows)
.\scripts\check-health.ps1
```

---

## License

[MIT License](./LICENSE).
