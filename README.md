<div align="center">
  <img src="docs/assets/logo-512.png" width="150" alt="syncthing-rust logo" style="border-radius: 20px;">

# syncthing-rust

**Production-grade Syncthing BEP protocol in Rust.**

Zero runtime dependencies, single static binary, and strictly wire-compatible with the official Go daemon.

[中文](./README-zh.md) · [Documentation](docs/design/topology.md) · [Contribute](CONTRIBUTING.md) · [Support](SUPPORT.md)

<a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust" alt="Rust"></a>
<a href="https://github.com/juice094/syncthing-rust/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/juice094/syncthing-rust/ci.yml?label=CI&color=22c55e" alt="CI"></a>
<a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT%20%2B%20Commercial-blue" alt="License"></a>
</div>

---

## What syncthing-rust does

Unlike lightweight wrappers, `syncthing-rust` provides a full-stack, production-ready implementation of the Syncthing
protocol.

- **Wire-compatible**: Drops seamlessly into existing Syncthing ecosystems, verified against the official Go daemon.
- **Zero-dependency deployment**: Compiles into a single static binary. No runtime, no shared libraries, just run it.
- **Enterprise-grade sync**: Block-level SHA-256 hashing, TLS handshakes, and multi-path discovery (LAN UDP, Global
  mTLS, STUN, UPnP, Relay v1).
- **Observability & Control**: Real-time TUI, Windows system tray, Prometheus metrics, and an MCP Bridge for AI agent
  orchestration.

## Quick Start

Requires Rust 1.85 or higher.

```bash
# Build from source
git clone https://github.com/juice094/syncthing-rust.git
cd syncthing-rust
cargo build --release

# Initialize configuration and run
cargo run --release -- init
cargo run --release -- run --config-dir ~/.syncthing
```

An Ed25519 TLS certificate is generated automatically on first run. By default, the BEP protocol listens on port `22001`
and the REST API on `8385`.

## Tech Stack

- **Protocol**: BEP over TLS (prost + LZ4 compression).
- **Async Runtime**: Tokio.
- **TLS**: rustls + ed25519-dalek.
- **Storage**: sled (metadata + block-cache abstraction).
- **REST API**: Axum (Go-layout compatible) + RBAC API keys.
- **TUI & CLI**: ratatui + crossterm + clap.

## Example

### Lightweight Demo

The fastest way to verify local bidirectional sync is to spin up two nodes on the same machine and watch the file
propagate.

### Production Scenario

In real-world deployments, `syncthing-rust` acts as a resilient cluster node. It automatically discovers peers across
NATs (using STUN/Relay), adapts pull concurrency based on RTT, and exposes `/metrics` for Prometheus alerting, while the
TUI allows real-time monitoring without a GUI.

## Capabilities and Configuration

- **Folder Orchestrator**: Unified multi-folder scan/pull scheduling with concurrency limits and priority.
- **Versioning**: Simple (keep=N) and Staggered (time windows) with `.stversions/` archiving.
- **Predictive Health Checks**: Real-time failure-rate assessment with automatic throttling.
- **Incremental Scanning**: Watcher dirty-path set triggers subtree or single-file scans.
- **RBAC API Keys**: Separate admin and read-only API keys for the REST API.

## Known Limitations

- **No Web GUI**: Interfaces are intentionally restricted to TUI, system tray, and REST API.
- **No symlink sync**: Symlinks are currently skipped silently.
- **No QUIC transport**: TCP and Relay v1 are the only supported transport layers.
- **High-latency networks**: Bulk large-file transfers may drop behind firewalls (mitigated via auto-reconnect and
  keepalive).

## Documentation

To keep this README focused, detailed technical documentation has been separated:

- [Project Topology & Architecture](docs/design/topology.md)
- [Agent Guidance & Constraints](docs/agent/index.md)
- [Known Issues & Defect Register](docs/KNOWN_ISSUES.md)
- [Roadmap & Plans](docs/plans/INDEX.md)

## Development and Tests

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock
cargo fmt --all -- --check
```

## Contributing

Issues and pull requests are welcome. Read the [contributing guide](CONTRIBUTING.md) before opening a PR. For security
reports, see [SECURITY.md](SECURITY.md).

## License

Licensed under [MIT + Commercial](./LICENSE). See [LICENSE-COMMERCIAL.md](./LICENSE-COMMERCIAL.md) for commercial
support.