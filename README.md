<div align="center">

# 🔄 syncthing-rust

> **Rust implementation of the Syncthing protocol stack**

Zero-runtime-dependency · Wire-compatible with Go Syncthing · Single static binary (~12 MB)

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![Tests](https://img.shields.io/badge/tests-319%20passed-brightgreen)](https://github.com/juice094/syncthing-rust/actions)
[![Clippy](https://img.shields.io/badge/clippy-0%20warnings-brightgreen)](https://github.com/juice094/syncthing-rust/actions)
[![Version](https://img.shields.io/badge/version-v0.2.8-blue)](https://github.com/juice094/syncthing-rust/releases)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

</div>

---

## 📋 简介

A Rust implementation of the [Syncthing](https://syncthing.net/) BEP protocol — zero-runtime-dependency, single static binary, wire-compatible with the official Go daemon.

**Current stage**: Alpha. Core protocol complete, E2E sync verified, production hardening in progress. Not yet a drop-in Go replacement.

> [Latest Release: v0.2.9-rc2](https://github.com/juice094/syncthing-rust/releases/tag/v0.2.9-rc2) — Centralized Constants + Transport Plugin RFC + Dual-Node E2E Infrastructure

---

## 🌟 核心亮点

| 亮点 | 说明 |
|:---|:---|
| 🔐 **Full BEP Protocol** | TLS + Hello + ClusterConfig + Index + Request/Response, codec verified |
| 📁 **End-to-end File Sync** | Block-level pull/push, SHA-256 scanning, ~12s two-node loopback transfer |
| 🌐 **Multi-path Discovery** | LAN UDP · Global HTTPS mTLS · STUN · UPnP · Relay v1 with RTT-scored dialer |
| 🖥️ **Real-time TUI** | Live sync state via event bridge, config hot-reload without restart |
| 🔄 **Go Interop** | Wire-compatible with Go Syncthing v2.1.0 (cross-version verified) |

> [完整里程碑与路线图 → docs/plans/INDEX.md](docs/plans/INDEX.md)

---

## 🔧 技术栈

| 组件 | 技术 |
|:---|:---|
| Protocol | BEP over TLS (custom codec) |
| Networking | Tokio + rustls + custom dialer |
| Discovery | UDP broadcast + HTTPS mTLS + STUN + UPnP + Relay v1 |
| Storage | Metadata & block cache abstractions |
| REST API | Axum (Go-layout compatible) |
| TUI | Custom event bridge |

---

## 📁 项目结构

```
syncthing-rust/
├── cmd/syncthing/          # CLI + TUI entry point
├── crates/
│   ├── syncthing-core/     # DeviceId, FileInfo, VersionVector
│   ├── bep-protocol/       # BEP codec & handshake
│   ├── syncthing-net/      # TCP+TLS, ConnectionManager, discovery
│   ├── syncthing-sync/     # Scanner, Puller, IndexHandler, watcher
│   ├── syncthing-api/      # REST API (Axum)
│   └── syncthing-db/       # Metadata & block cache
└── docs/                   # ADRs, plans, verification reports
```

> **Trust boundary**: `syncthing-core` is read-only for downstream crates. See [docs/design/ARCHITECTURE_DECISIONS.md](docs/design/ARCHITECTURE_DECISIONS.md).

---

## 🚀 快速开始

```bash
# 1. Clone
git clone https://github.com/juice094/syncthing-rust.git && cd syncthing-rust

# 2. Build (< 1 min)
cargo build --release -p syncthing

# 3. Run
cargo run --release -p syncthing -- tui    # interactive TUI
# or: cargo run --release -p syncthing -- run  # headless
```

First run generates Ed25519 TLS cert in `%LOCALAPPDATA%\syncthing-rust`.  
Default ports: BEP `22001`, REST API `8385`.

```powershell
# Verify it's working
curl http://127.0.0.1:8385/rest/system/status | ConvertFrom-Json
```

---

## 🤝 参与贡献

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full guide. Quick validation:

```bash
cargo test --workspace        # ~319 tests
cargo clippy --workspace --all-targets  # 0 warnings
```

---

## 📄 License

[MIT License](LICENSE).

---

<div align="center">

**[⭐ Star](https://github.com/juice094/syncthing-rust) · [🐛 Issues](https://github.com/juice094/syncthing-rust/issues) · [🤝 Contribute](CONTRIBUTING.md)**

</div>
