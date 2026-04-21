# Changelog

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
