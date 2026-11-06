<div align="center">
  <img src="./docs/assets/logo-512.png" alt="syncthing-rust logo" width="140" style="border-radius: 20px;">

# syncthing-rust

**Syncthing BEP Protocol in Rust**

Zero runtime dependencies · Wire-compatible with Go Syncthing · Single static binary

[English](./README.md) · [中文](./README-zh.md)

<a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust" alt="Rust"></a>
<a href="https://github.com/juice094/syncthing-rust/actions/workflows/ci.yml"><img src="https://github.com/juice094/syncthing-rust/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
<a href="./LICENSE"><img src="https://img.shields.io/badge/license-MIT%20%2B%20Commercial-blue" alt="License"></a>
</div>

---

## Introduction

A Rust implementation of the Syncthing BEP protocol. It provides end-to-end file synchronization with zero runtime
dependencies, compiled into a single static binary that is wire-compatible with the official Go daemon.

This project is currently in a **Production** phase. Core protocols, end-to-end sync, Windows system tray, and TUI are
stable. For the latest release notes and changelog, please refer to
the [Releases](https://github.com/juice094/syncthing-rust/releases) page.

---

## Quick Start

**Prerequisites:** Rust 1.85 or higher.

```bash
# Build from source
git clone https://github.com/juice094/syncthing-rust.git
cd syncthing-rust
cargo build --release

# Initialize configuration
cargo run --release -- init

# Start the daemon
cargo run --release -- run --config-dir ~/.syncthing
```

An Ed25519 TLS certificate is generated automatically on first run. By default, the BEP protocol listens on port `22001`
and the REST API on `8385`.

To explore other commands such as TUI, Tray, or Relay Server, run:

```bash
cargo run --release -- --help
```

---

## Core Features

- **Full BEP Protocol**: prost codec, TLS handshake, ClusterConfig, and Index with LZ4 compression.
- **End-to-End Sync**: Block-level pull/push, SHA-256 scanning, and bidirectional sync with sub-second change detection.
- **Multi-Path Discovery**: LAN UDP, Global HTTPS mTLS, STUN, UPnP, and Relay v1.
- **Versioning**: Simple (keep=N) and Staggered (time windows) with `.stversions/` archiving.
- **Go Interop**: Strictly wire-compatible with the official Go Syncthing daemon.
- **Observability & Integration**: Real-time TUI, Prometheus metrics, structured JSON logging, and an MCP Bridge for AI
  agents.

> For advanced orchestration (concurrency limits, predictive health checks) and deployment options, see the
> Documentation.

---

## Known Limitations

- **High-latency / unstable networks**: Bulk large-file transfers may drop behind firewalls. Mitigated via
  auto-reconnect and keepalive.
- **No symlink sync**: Symlinks are currently skipped silently.
- **No Web GUI**: Interfaces are limited to TUI, system tray, and REST API.
- **No QUIC transport**: TCP and Relay v1 are the only supported transport layers.

---

## Documentation

To keep this README focused and stable, detailed technical documentation has been separated:

- [Project Topology & Architecture](docs/design/topology.md)
- [Agent Guidance & Constraints](docs/agent/index.md)
- [Known Issues & Defect Register](docs/KNOWN_ISSUES.md)
- [Roadmap & Plans](docs/plans/INDEX.md)

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for detailed guidelines. Before submitting a PR, ensure the following checks
pass:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock
cargo fmt --all -- --check
```

---

## Community & Support

- **Bug Reports & Feature Requests**: [GitHub Issues](https://github.com/juice094/syncthing-rust/issues)
- **Usage Questions**: [GitHub Discussions](https://github.com/juice094/syncthing-rust/discussions)
- **Security Vulnerabilities**: See [SECURITY.md](SECURITY.md) (do not open a public issue)

---

## License

Licensed under [MIT + Commercial](./LICENSE). See [LICENSE-COMMERCIAL.md](./LICENSE-COMMERCIAL.md) for commercial
support.