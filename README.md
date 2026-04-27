# syncthing-rust

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![Tests](https://img.shields.io/badge/tests-294%20passed-brightgreen)]()
[![Clippy](https://img.shields.io/badge/clippy-0%20warnings-brightgreen)]()
[![License](https://img.shields.io/badge/license-MIT-blue)](./LICENSE)

A Rust implementation of the [Syncthing](https://syncthing.net/) protocol stack, designed for **zero-runtime-dependency** deployment and wire-compatible interoperability with the official Go Syncthing daemon.

> **Current stage**: Functionally complete Beta. Core BEP messages, multi-path networking, REST API read-path, and TUI observation are operational. Pending long-term stability validation (72h stress test) and cross-version Rust interoperability verification.
>
> **Value proposition**: If you need a single static binary (< 10 MB) that speaks BEP over TLS and can sync folders with official Syncthing nodes — without Go runtime or CGO — this is it.

---

## At a Glance

| Dimension | State |
|-----------|-------|
| BEP Protocol (TLS + Hello + ClusterConfig + Index + Request/Response) | ✅ Core messages implemented and handshake verified |
| File Sync (Pull via BEP blocks, passive Push upload) | ✅ Pull verified; passive upload implemented |
| Network Discovery (Local + Global + STUN + UPnP + Relay v1) | ✅ Core implementation complete; ParallelDialer with RTT scoring |
| REST API (read + write, Go-layout compatible) | ✅ Read-path complete; write-path partial (override/revert stub) |
| Tests | **294 passed, 3 ignored, 0 failed** |
| Lint | **0 clippy warnings** |
| Security audit | **3 unmaintained** upstream transitive deps (accepted debt, see `.cargo/audit.toml`) |
| Binary size | ~8 MB (release, Windows x64) |

> **Current limitations**:
> - 72h long-term stability test not yet executed (pending).
> - Cross-version Rust interoperability (new `main` ↔ old pre-fix build) pending格雷侧 network verification.
> - Go Syncthing full file-sync interoperability not yet validated (handshake verified only).

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
| **Phase 3** | BepSession observability, Push/Pull E2E with remote peer | ✅ Complete (verified against格雷侧 pre-fix Rust build; Go node pending) |
| **Phase 3.5** | Connection stability, config persistence | ✅ Complete |
| **Phase 4** | TUI hardening (event bridge, live sync state, config hot-reload) | ✅ Complete |
| **Phase 5** | Zero-Tailscale interconnection (discovery results → ConnectionManager address pool) | 🔵 Core integrated; field validation pending |
| **Phase A** | Security debt acceptance (cargo audit) | ✅ Complete (`.cargo/audit.toml` created) |
| **Phase B** | 72h stress test | ⏳ Infrastructure ready (`bin/stress_test.rs` exists); execution pending |
| **Phase C** | REST API write-path closure | ⏳ Partial (override/revert still stub) |

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
| [`docs/ai-protocol.md`](docs/ai-protocol.md) | Cross-session state anchor for AI agents |

---

## Contributing

See [`CONTRIBUTING.md`](./CONTRIBUTING.md). Short version:

```powershell
cargo test --workspace          # must pass
cargo clippy --workspace --all-targets  # must be 0 warnings
```

---

## License

[MIT License](./LICENSE).
