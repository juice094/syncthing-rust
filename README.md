# syncthing-rust

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MPL--2.0-blue)](./LICENSE)
[![Docs](https://img.shields.io/badge/docs-index-green)](./docs/README.md)

A Rust implementation of the [Syncthing](https://syncthing.net/) protocol stack, designed to interoperate with the official Go Syncthing daemon over the BEP (Block Exchange Protocol) wire format.

> **Status**: v0.2.0 — BEP 协议层（TLS + Hello + ClusterConfig + Index + Request/Response）在 **Tailscale 虚拟网络环境下** 已与官方 Go 节点完成双向互通验证（握手 + 文件收发）。当前处于**无 Tailscale 时无法与隔离网络中的云服务器建立连接**的阶段；网络发现层设计已完成，待实现。

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
| 2026-04-20 | **v0.2.0 Release**: BEP 协议兼容性修复（ClusterConfig 本地设备、Hello 伪装、TLS crypto provider、WireFolder label），双向文件同步验证通过。 |
| 2026-04-20 | **Network Discovery 设计完成** — Local + Global Discovery + STUN + UPnP + Relay 完整设计文档出稿，旨在解决无 Tailscale 时的设备互联问题。 |

---

## Roadmap

| Phase | Goal | Status |
|-------|------|--------|
| **Phase 1** | Core protocol (TLS, BEP Hello, ClusterConfig, Index) | ✅ Complete |
| **Phase 2** | Network abstraction (ReliablePipe, BepSession), watcher, REST API, dual-node coexistence | ✅ Complete |
| **Phase 3** | BepSession observability, peer sync state events, **Push/Pull E2E with real Go node** | ✅ Complete |
| **Phase 3.5** | Connection stability hardening, `.stignore`, config persistence | ✅ Complete |
| **Phase 4** | TUI 增强（设备/文件夹管理、实时同步状态） | 🔵 In Progress |
| **Phase 5** | **自建网络发现层** — 消除 Tailscale 依赖，实现无 VPN 环境下的设备发现与互联 | 📝 Design Complete |

> **Phase 5 详情**: 参见 [`docs/design/NETWORK_DISCOVERY_DESIGN.md`](docs/design/NETWORK_DISCOVERY_DESIGN.md)。

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

> **Note on device connectivity**: v0.2.0 的 BEP 互通验证**依赖 Tailscale 虚拟网络**（`100.x.x.x`）。当前无 Tailscale 时，**无法与隔离网络中的云服务器建立连接**。Phase 5 网络发现层（Local Discovery + Global Discovery + Relay）的目标正是消除这一外部依赖，实现零配置互联。

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
├── syncthing-net/      # TCP+TLS transport, ConnectionManager, dialer, (STUN/UPnP/Relay WIP)
├── syncthing-sync/     # SyncService, Scanner, Puller, IndexHandler, watcher
├── syncthing-api/      # REST API server (Axum)
└── syncthing-db/       # Database abstractions
docs/
├── design/             # 活跃的设计文档
├── plans/              # 计划与路线图
├── reports/            # 验证报告与实现总结
└── archive/            # 历史归档
```

---

## Features

| Feature | Status | Notes |
|---------|--------|-------|
| BEP Protocol (TLS + Hello + Index + Request/Response) | ✅ | 已与官方 Go 节点在 Tailscale 环境下互通验证 |
| TCP Transport | ✅ | |
| HTTP CONNECT / SOCKS5 Proxy | ⚠️ | 代码完整，但 `ParallelDialer` 主流程未实际使用代理 |
| Folder Scan / Pull | ⚠️ | Pull 通过 BEP Request/Response 真实拉取；`ManagerBlockSource` 向**任意**已连接设备请求块，多设备场景下非定向 |
| Push (被动响应上传) | ✅ | `block_server.rs` + `BepSession::on_block_request` 链路完整 |
| Push (主动调度) | 📝 | 主动扫描后触发对端拉取的调度逻辑待完善 |
| Conflict Resolution | ✅ | |
| Filesystem Watcher | ✅ | `notify` + 1s debounce |
| REST API | ✅ | 读接口完整，部分写接口待补充 |
| TUI | ✅ | 设备/文件夹管理、实时状态 |
| Config Persistence | ✅ | `JsonConfigStore` 支持 notify 监听 + 异步读写 |
| Local Discovery (LAN) | ⚠️ | UDP 广播/接收/run 循环已集成；IPv6 多播、网卡枚举、子网广播地址计算缺失 |
| Global Discovery | ❌ | 完全空白 |
| STUN (公网 IP 查询) | ⚠️ | Binding Request + XOR-MAPPED-ADDRESS 解析可用；NAT 类型检测、hole punching 缺失 |
| UPnP | ⚠️ | `igd` crate 集成可用；自动续约缺失 |
| NAT-PMP / PCP | ❌ | 骨架存在，未实现 |
| Relay (官方 Protocol) | ❌ | 完全空白；现有 DERP 为自研协议，无法与 Go 互通 |

---

## Documentation

项目文档按活跃程度分层存放：

- **[`docs/README.md`](docs/README.md)** — 文档导航页，说明各目录用途。
- **[`docs/design/NETWORK_DISCOVERY_DESIGN.md`](docs/design/NETWORK_DISCOVERY_DESIGN.md)** — 当前核心设计文档（网络发现层）。
- **[`docs/reports/IMPLEMENTATION_SUMMARY.md`](docs/reports/IMPLEMENTATION_SUMMARY.md)** — 架构总览与 crate 级状态。
- **[`docs/reports/VERIFICATION_REPORT_BEP_2026-04-11.md`](docs/reports/VERIFICATION_REPORT_BEP_2026-04-11.md)** — BEP 互操作测试报告。
- **[`docs/design/FEATURE_COMPARISON.md`](docs/design/FEATURE_COMPARISON.md)** — 与官方 Go Syncthing 的功能对标。

---

## License

This project is licensed under the [MPL-2.0](https://www.mozilla.org/en-US/MPL/2.0/) license.
