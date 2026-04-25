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
- Tests: 255+ passed, 0 clippy warnings
- Transports: TCP+TLS ✅ / HTTP CONNECT ⚠️（代码完整，主流程未接入） / SOCKS5 ⚠️（同上） / DERP（自研协议，非 Syncthing 官方 Relay） / UPnP ⚠️（无自动续约） / NAT-PMP ❌ / PCP ❌

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

- **跨网络互联**: 无 Tailscale 时无法与隔离网络中的节点建立连接；Global Discovery + 官方 Relay Protocol 完全空白
- **ManagerBlockSource 缺陷**: Pull 方向向**任意**已连接设备请求块，非目标定向
- **Next step**: 实现 Global Discovery 客户端 + 官方 Relay Protocol XDR 编解码

## Cross-Project Interfaces

- **clarity**: clarity-wire event bus → syncthing-rust P2P gateway → cross-instance verification
- **devbase**: `.syncdone` marker format aligned; boundary map versions synced via P2P then written to devbase OpLog

## Tech Selection Framework

All selections weighted across 7 dimensions: SDK maturity, dev efficiency, distribution cost, stack consistency, maintenance cost, dependency risk, type safety. High-necessity features must score high on **stack consistency + dependency risk**.
