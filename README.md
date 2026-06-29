<div align="center">

# 🔄 syncthing-rust

> **Syncthing BEP Protocol in Rust**

[English](./README.md) · [中文](./README-zh.md)

Zero runtime dependencies · Wire-compatible with Go Syncthing · Single static binary (~13 MB)

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![CI](https://github.com/juice094/syncthing-rust/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/juice094/syncthing-rust/actions/workflows/ci.yml)
[![Tests](https://img.shields.io/badge/tests-413%20passed-brightgreen)](https://github.com/juice094/syncthing-rust/actions)
[![Version](https://img.shields.io/badge/version-v3.0.4-blue)](https://github.com/juice094/syncthing-rust)
[![License](https://img.shields.io/badge/license-MIT%20%2B%20Commercial-blue)](./LICENSE)

</div>

---

## 📋 Introduction

A Rust implementation of the [Syncthing](https://syncthing.net/) BEP protocol — zero runtime dependencies, single static binary, wire-compatible with the official Go daemon.

**Current phase**: Production. Core protocol, end-to-end sync, Windows system tray, and TUI are stable.

> [Latest release: v3.0.4](https://github.com/juice094/syncthing-rust)

---

## 📚 Table of Contents

- [Highlights](#-highlights)
- [Tech Stack](#-tech-stack)
- [Project Structure](#-project-structure)
- [Documentation](#-documentation)
- [Known Limitations](#-known-limitations)
- [Quick Start](#-quick-start)
- [Contributing](#-contributing)
- [Community & Support](#-community--support)
- [License](#-license)

---

## 🌟 Highlights

| Feature | Description |
|:---|:---|
| 🔐 **Full BEP Protocol** | prost codec, TLS + Hello + ClusterConfig + Index, LZ4 compression |
| 📁 **End-to-End File Sync** | Block-level pull/push, SHA-256 scanning, bidirectional sync detects changes in ~1 s |
| 🚀 **Proactive Push** | Local changes pushed immediately to connected peers |
| 🗂️ **Versioning** | Simple (keep=N) + Staggered (4 time windows), `.stversions/` archive |
| 🌐 **Multi-Path Discovery** | LAN UDP · Global HTTPS mTLS · STUN · UPnP · Relay v1 |
| 🖥️ **Real-Time TUI** | Event-driven live sync status with hot-reload config |
| 🔌 **Go Interop** | Wire-compatible with Go Syncthing v2.1.0 (cross-version verified) |
| ⚙️ **Folder Orchestrator** | Unified multi-folder scan/pull scheduling with concurrency limits, jitter, and priority |
| 🔮 **Predictive Health Checks** | Real-time failure-rate / watcher-loss / state-flip assessment with automatic throttling |
| 📈 **Adaptive Pull Concurrency** | Dynamic downloads/blocks concurrency based on block-request RTT |
| 🧩 **Incremental Scanning** | Watcher dirty-path set triggers subtree/single-file incremental scans |
| 📦 **Single Static Binary** | Release build ~13 MB, zero runtime dependencies |
| 🔄 **BEP Relay Server v1** | Standalone relay listener with TLS and device-id routing |
| 🔑 **RBAC API Keys** | Separate admin + read-only API keys for REST API |
| 📜 **Structured JSON Logging** | `--log-format json` for ELK/Splunk/Loki aggregation |
| 📊 **Prometheus Metrics** | `/metrics` endpoint with build/uptime/connection/folder stats |
| 🐳 **Docker & Compose** | Multi-stage Dockerfile + relay compose stack + Grafana dashboard |

---

## 🔧 Tech Stack

| Component | Technology |
|:---|:---|
| Protocol | BEP over TLS (prost + LZ4) |
| Async Runtime | Tokio |
| TLS | rustls + ed25519-dalek |
| Networking | Tokio + rustls + ParallelDialer + Relay v1 |
| Discovery | UDP broadcast + HTTPS mTLS + STUN + UPnP |
| Storage | sled (metadata + block-cache abstraction) |
| REST API | Axum (Go-layout compatible) + RBAC API keys |
| Metrics | Prometheus text format (`/metrics`) |
| Logging | tracing + tracing-subscriber (text / JSON) |
| TUI | ratatui + crossterm |
| CLI | clap |

---

## 📁 Project Structure

```
syncthing-rust/
├── cmd/syncthing/          # CLI entrypoint + TUI + daemon runner + tray
├── crates/
│   ├── syncthing-core/     # Core types (DeviceId, FileInfo, VersionVector)
│   ├── bep-protocol/       # BEP codec (prost) + handshake
│   ├── syncthing-net/      # TCP+TLS, ConnectionManager, dial, discovery, Relay
│   ├── syncthing-sync/     # Scanner, Puller, IndexHandler, watcher
│   ├── syncthing-fs/       # Filesystem abstraction (ignore, scanner, watcher)
│   ├── syncthing-api/      # REST API (Axum, Go-layout compatible)
│   ├── syncthing-db/       # Metadata and block cache
│   └── syncthing-versioner/# File versioning (Simple + Staggered)
├── deploy/                 # Docker, compose, nginx, Grafana dashboard
├── docs/                   # Design docs, plans, reports
└── scripts/                # Health checks, cloud-deploy, stress tests
```

### Full Architecture

- Project topology and runtime architecture: [`docs/design/topology.md`](docs/design/topology.md)
- Agent development constraints and coding standards: [`docs/agent/constraints.md`](docs/agent/constraints.md)

---

## 📖 Documentation

| Document | Content |
|:---|:---|
| [`docs/design/topology.md`](docs/design/topology.md) | Project topology, crate dependency graph, runtime components, key entry points |
| [`docs/agent/index.md`](docs/agent/index.md) | Agent guidance overview (constraints, testing, security, operations) |
| [`docs/agent/constraints.md`](docs/agent/constraints.md) | Crate boundary red lines, forbidden items, coding standards |
| [`docs/KNOWN_ISSUES.md`](docs/KNOWN_ISSUES.md) | Authoritative defect register and fact-checking source |
| [`docs/plans/INDEX.md`](docs/plans/INDEX.md) | Roadmap and plan index |

---

## ⚠️ Known Limitations

| Limitation | Impact | Mitigation |
|:---|:---|:---|
| High-latency / unstable networks | Bulk large-file transfers may drop behind firewalls | Auto-reconnect + keepalive; network tuning in [KNOWN_ISSUES §14](docs/KNOWN_ISSUES.md) |
| No symlink sync | Symlinks are silently skipped | Planned for a future release |
| No Web GUI | TUI + system tray + REST API are the primary interfaces (frozen per [`docs/agent/constraints.md`](docs/agent/constraints.md)) | — |
| No QUIC transport | TCP + Relay v1 only (frozen per [`docs/agent/constraints.md`](docs/agent/constraints.md)) | Under future evaluation |

---

## 🚀 Quick Start

```bash
git clone https://github.com/juice094/syncthing-rust.git && cd syncthing-rust
cargo build --release

# Initialize configuration
cargo run --release -- init

# Start the daemon
cargo run --release -- run --config-dir ~/.syncthing

# Or start the TUI
cargo run --release -- tui --config-dir ~/.syncthing
```

An Ed25519 TLS certificate is generated automatically on first run. Default ports: BEP `22001`, REST API `8385`.

---

## 🤝 Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) and [`docs/agent/constraints.md`](docs/agent/constraints.md).

Before submitting, run:

```bash
cargo test --workspace        # 392 passed / 6 ignored / 0 failed
cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock
cargo fmt --all -- --check
```

---

## 💬 Community & Support

| Scenario | Entry |
|:---|:---|
| Bug report | [GitHub Issues](https://github.com/juice094/syncthing-rust/issues/new?template=bug_report.md) |
| Feature request | [GitHub Issues](https://github.com/juice094/syncthing-rust/issues/new?template=feature_request.md) |
| Usage question / discussion | [GitHub Discussions](https://github.com/juice094/syncthing-rust/discussions) |
| Security vulnerability | See [SECURITY.md](SECURITY.md) (do not open a public issue) |
| Commercial support | See [SUPPORT.md](SUPPORT.md) / [LICENSE-COMMERCIAL.md](LICENSE-COMMERCIAL.md) |

---

## 📄 License

[MIT + Commercial](./LICENSE) · [Commercial License](./LICENSE-COMMERCIAL.md)

---

<div align="center">

**[⭐ Star](https://github.com/juice094/syncthing-rust) · [🐛 Issues](https://github.com/juice094/syncthing-rust/issues) · [💬 Discussions](https://github.com/juice094/syncthing-rust/discussions) · [🤝 Contribute](CONTRIBUTING.md)**

</div>
