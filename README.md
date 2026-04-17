# syncthing-rust

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MPL--2.0-blue)](./LICENSE)
[![Docs](https://img.shields.io/badge/docs-index-green)](./docs/README.md)

A Rust implementation of the [Syncthing](https://syncthing.net/) protocol stack, designed to interoperate with the official Go Syncthing daemon over the BEP (Block Exchange Protocol) wire format.

> **Status**: Work in progress — Phase 2 complete (watcher, REST API observability, and long-term connection stability). Phase 3 (workspaces, 72h stress test) pending.

---

## Current Milestones

| Date | Milestone |
|------|-----------|
| 2026-04-09 | First successful TLS + BEP Hello handshake with a real Go Syncthing peer. |
| 2026-04-09 | Bidirectional `ClusterConfig` / `Index` exchange loop established. |
| 2026-04-11 | Cross-network interoperability verified: full file download via `Request`/`Response` over Tailscale. |
| 2026-04-15 | Filesystem watcher integrated (`notify` crate), 1s debounce → scan → `IndexUpdate` broadcast in ~2s. |
| 2026-04-15 | REST API with real uptime / connection enumeration; default port migration to `22001/8385`. |
| 2026-04-16 | REST API `/rest/db/status` now returns real per-folder file counts and byte totals. |
| 2026-04-17 | Phase 2 Network Abstraction: `ReliablePipe` trait decouples BEP from TCP; `MemoryPipe` tests pass; `ConnectionManager` supports multi-path per device. |

---

## Roadmap

| Phase | Goal | Status |
|-------|------|--------|
| **Phase 1** | Core protocol (TLS, BEP Hello, ClusterConfig, Index) | ✅ Complete |
| **Phase 2** | Watcher, REST API, dual-node coexistence, >2h stability | ✅ Complete |
| **Phase 3** | 72h long-connection stress test, workspace migration, real folder sync | 🟡 In Progress |
| **Phase 4** | Push/pollish, GUI or Web frontend, production packaging | 🔵 Planned |

---

## Quick Start

```bash
# Build the daemon
cargo build --release -p syncthing

# Run with TUI (recommended for interactive setup)
cargo run --release -p syncthing -- tui

# Or run headless
cargo run --release -p syncthing -- run
```

The daemon will:
- Generate an Ed25519 TLS certificate on first run (stored in `%LOCALAPPDATA%\syncthing-rust` on Windows, or `~/.local/share/syncthing-rust` on Linux).
- Listen for BEP connections on `0.0.0.0:22001` (falling back to a random port if occupied).
- Serve the REST API on `0.0.0.0:8385` (loopback addresses bypass API key auth for local debugging).

---

## REST API (Local Observability)

The REST API is compatible with the Go Syncthing endpoint layout:

```bash
# System status — includes real uptime and configured folder/device counts
curl http://127.0.0.1:8385/rest/system/status

# Active BEP connections
curl http://127.0.0.1:8385/rest/connections

# Folder sync status — now returns real file counts and bytes
curl "http://127.0.0.1:8385/rest/db/status?folder=test-folder"
```

Example response for `/rest/db/status`:
```json
{
  "folder": "test-folder",
  "files": 7,
  "directories": 0,
  "bytes": 264,
  "state": "idle"
}
```

---

## Project Structure

```
cmd/syncthing/          # CLI entry point and TUI
crates/
├── syncthing-core/     # DeviceId, FileInfo, VersionVector, core types
├── bep-protocol/       # BEP Hello, Request/Response, Index, ClusterConfig
├── syncthing-net/      # TCP+TLS transport, ConnectionManager, dialer, STUN
├── syncthing-sync/     # SyncService, Scanner, Puller, IndexHandler, watcher
├── syncthing-api/      # REST API server (Axum)
└── syncthing-db/       # Database abstractions
dev/third_party/iroh/   # Optional iroh QUIC transport (see below)
```

---

## Optional: iroh Transport

`syncthing-net` includes an optional `iroh` Cargo feature for TLS-over-QUIC transport. The feature expects the `iroh` crate at `dev/third_party/iroh/iroh`. Because `iroh` is a large workspace and its crates.io releases currently have transitive dependency conflicts with this project, it is **not enabled by default** and the dependency line is commented out in `crates/syncthing-net/Cargo.toml`. To enable the feature:

```bash
# 1. Clone iroh locally
git clone https://github.com/n0-computer/iroh.git dev/third_party/iroh

# 2. Uncomment the iroh dependency and feature in crates/syncthing-net/Cargo.toml
# 3. Build with the feature
cargo build -p syncthing --features iroh
```

---

## Documentation

Project reports and design documents are kept under [`docs/`](docs/):

- [`docs/README.md`](docs/README.md) — Documentation index and reading guide.
- [`docs/IMPLEMENTATION_SUMMARY.md`](docs/IMPLEMENTATION_SUMMARY.md) — Architecture and crate-level status.
- [`docs/VERIFICATION_REPORT_BEP_2026-04-11.md`](docs/VERIFICATION_REPORT_BEP_2026-04-11.md) — Cross-network BEP interop test results.
- [`docs/FEATURE_COMPARISON.md`](docs/FEATURE_COMPARISON.md) — Comparison with official Go Syncthing.
- [`docs/MVP_RECOVERY_PLAN.md`](docs/MVP_RECOVERY_PLAN.md) — Recovery plan from earlier project stages.

---

## License

This project is licensed under the [MPL-2.0](https://www.mozilla.org/en-US/MPL/2.0/) license.
