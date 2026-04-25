---
name: bep-protocol
description: P2P file synchronization via BEP protocol. Use when working with syncthing-rust BEP messages, TCP+TLS transport, NAT traversal, or the MCP Bridge. Covers Rust↔Go interop, handshake flow, and cross-instance verification extensions.
---

# BEP Protocol Skill

## Quick Commands

```bash
cargo test --workspace              # 255+ tests
cargo clippy --workspace            # 0 warnings
```

## Status

- Version: **v0.2.0 Beta**
- Tests: 255+ passed, 0 TODOs, 0 clippy warnings
- Transports: TCP+TLS / SOCKS5 / DERP relay / UPnP / NAT-PMP / PCP

## BEP Messages

Core message types for Go interop:

| Message | Direction | Notes |
|---------|-----------|-------|
| `Hello` | bidirectional | Device ID + name + capabilities |
| `ClusterConfig` | bidirectional | Folders + devices; `WireFolder.label` is `String` (not `repeated string`) |
| `Index` | bidirectional | File metadata updates |
| `Request` / `Response` | bidirectional | Block-level file transfer |

## MCP Bridge

- **Process**: `syncthing-mcp-bridge` (standalone)
- **Protocol**: hand-written JSON-RPC 2.0 (~200 LOC), zero third-party MCP SDK deps
- **Tools**: 11 tools + 3 resources
- **Path**: Kimi/Claude ← MCP stdio ← Bridge ← REST API → syncthing-rust

## Current Blocker

- **格雷端网络**: Go Syncthing not listening on Tailscale IP (`100.99.240.98:22000`); Rust dial refused (os error 10061)
- **Next step**: confirm Go node status / listening address, or provide alternative address

## Cross-Project Interfaces

- **clarity**: clarity-wire event bus → syncthing-rust P2P gateway → cross-instance verification
- **devbase**: `.syncdone` marker format aligned; boundary map versions synced via P2P then written to devbase OpLog

## Tech Selection Framework

All selections weighted across 7 dimensions: SDK maturity, dev efficiency, distribution cost, stack consistency, maintenance cost, dependency risk, type safety. High-necessity features must score high on **stack consistency + dependency risk**.
