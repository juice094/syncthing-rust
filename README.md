# syncthing-rust

A Rust implementation of the [Syncthing](https://syncthing.net/) protocol stack, designed to interoperate with the official Go Syncthing daemon over the BEP (Block Exchange Protocol) wire format.

> **Status**: Work in progress — Phase 2 (watcher, REST API observability, and long-term connection stability).

---

## Current Milestones

| Date | Milestone |
|------|-----------|
| 2026-04-09 | First successful TLS + BEP Hello handshake with a real Go Syncthing peer. |
| 2026-04-09 | Bidirectional `ClusterConfig` / `Index` exchange loop established. |
| 2026-04-11 | Cross-network interoperability verified: full file download via `Request`/`Response` over Tailscale. |
| 2026-04-15 | Filesystem watcher integrated (`notify` crate), 1s debounce → scan → `IndexUpdate` broadcast in ~2s. |
| 2026-04-15 | REST API with real uptime / connection enumeration; default port migration to `22001/8385`. |

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

# Folder sync status
curl "http://127.0.0.1:8385/rest/db/status?folder=test-folder"
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

- [`IMPLEMENTATION_SUMMARY.md`](docs/IMPLEMENTATION_SUMMARY.md) — Architecture and crate-level status.
- [`VERIFICATION_REPORT_BEP_2026-04-11.md`](docs/VERIFICATION_REPORT_BEP_2026-04-11.md) — Cross-network BEP interop test results.
- [`FEATURE_COMPARISON.md`](docs/FEATURE_COMPARISON.md) — Comparison with official Go Syncthing.
- [`MVP_RECOVERY_PLAN.md`](docs/MVP_RECOVERY_PLAN.md) — Recovery plan from earlier project stages.

---

## License

This project is licensed under the [MPL-2.0](https://www.mozilla.org/en-US/MPL/2.0/) license.
