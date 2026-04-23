# syncthing-rust

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MPL--2.0-blue)](./LICENSE)
[![Docs](https://img.shields.io/badge/docs-index-green)](./docs/README.md)

A Rust implementation of the [Syncthing](https://syncthing.net/) protocol stack, designed to interoperate with the official Go Syncthing daemon over the BEP (Block Exchange Protocol) wire format.

> **Status**: v0.2.0 — Beta. Core file sync (Rust ↔ Go) verified; 255+ tests passing; all TODOs resolved; 0 clippy warnings.

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
| 2026-04-20 | **Phase 3 Complete**: Cloud Push E2E (Rust → Go) and Cloud Pull E2E (Go → Rust) both verified over Tailscale. Protocol compatibility fixes: deleted-file block clearing, shared-folder Index filtering, connection race handling. |
| 2026-04-20 | **v0.2.0 Release**: SOCKS5 proxy, iroh dead code cleanup, naming conflict resolution, folder lifecycle control, REST traffic stats, DERP forwarding, LRU O(1) cache, precise timestamps, PCP protocol, 0 TODOs. |

---

## Roadmap

| Phase | Goal | Status |
|-------|------|--------|
| **Phase 1** | Core protocol (TLS, BEP Hello, ClusterConfig, Index) | ✅ Complete |
| **Phase 2** | Network abstraction (ReliablePipe, BepSession), watcher, REST API, dual-node coexistence | ✅ Complete |
| **Phase 3** | BepSession observability, peer sync state events, **Push/Pull E2E with real Go node** | ✅ Complete |
| **Phase 3.5** | Connection stability hardening, `.stignore`, config persistence | ✅ Complete |
| **Phase 4** | TUI 增强（设备/文件夹管理、实时同步状态）、72h 压测（格雷远程）、生产打包 | 🔵 In Progress |

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
```

---

## Features

| Feature | Status |
|---------|--------|
| BEP Protocol (TLS + Hello + Index + Request/Response) | ✅ |
| TCP Transport | ✅ |
| SOCKS5 / HTTP Proxy | ✅ |
| DERP Relay | ✅ |
| UPnP / NAT-PMP / PCP Port Mapping | ✅ (UPnP allocate; PMP/PCP release) |
| Folder Scan / Pull / Push | ✅ |
| Conflict Resolution | ✅ |
| Filesystem Watcher | ✅ |
| REST API | ✅ |
| TUI | ✅ |
| Config Persistence | ✅ |

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
