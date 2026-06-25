# CLAUDE.md

This file provides quick guidance to Claude Code (claude.ai/code) when working with code in this repository.

> **Detailed reference** (build commands, feature flags, daemon lifecycle, CLI commands, TUI bindings, REST APIs) lives in [`docs/agent/claude_reference.md`](docs/agent/claude_reference.md).  
> **Canonical Agent constraints** (crate boundaries, forbidden items, testing requirements, security, operations) live in [`docs/agent/`](docs/agent/index.md).  
> **Project topology and architecture** live in [`docs/design/topology.md`](docs/design/topology.md).

---

## Project Identity

`syncthing-rust` is a Rust reimplementation of the Syncthing BEP protocol for P2P file sync — zero runtime deps, single static binary, wire-compatible with Go Syncthing. Currently **v3.0.3** (production-grade), deployed on ROG-X (Windows 11) ↔ Gray-Cloud (Ubuntu 24.04) via Tailscale.

---

## Build & Test (Pre-Submit)

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace          # 392 passed / 6 ignored / 0 failed
cargo audit
```

Release builds:
- Windows desktop (tray): `cargo build --release --bin syncthing --features tray`
- Headless/server: `cargo build --release --bin syncthing --no-default-features`

See [`docs/agent/claude_reference.md`](docs/agent/claude_reference.md) for full build matrix, feature flags, and cross-compilation.

---

## Architecture at a Glance

```
cmd/syncthing
  ├─ syncthing-api
  ├─ syncthing-net
  ├─ syncthing-sync (consumes syncthing-db, syncthing-fs internally)
  ├─ syncthing-core   # types & traits only — no internal deps
  ├─ syncthing-versioner
  ├─ bep-protocol
  └─ syncthing-test-utils
```

Full DAG, runtime components, and key entry points: [`docs/design/topology.md`](docs/design/topology.md).

---

## Hard Rules

- No `unwrap()` / `expect()` in production paths.
- No breaking-change dependency upgrades just to silence cargo audit.
- No sled-specific APIs exposed from `syncthing-db`.
- No QUIC / MagicSocket / Web GUI / consensus / reputation / custom crypto.
- Network-layer changes must be validated with dual `TestNode` instances.
- New end-to-end behavior needs integration or E2E tests, not just `#[cfg(test)]` unit tests.

Full constraints: [`docs/agent/constraints.md`](docs/agent/constraints.md).

---

## Daemon Lifecycle

1. `tui::daemon_runner::start_daemon()` — TLS, ConnectionManager, SyncService, discovery
2. `api_server::start_api_server()` — binds REST API from `config.gui.address` (default `127.0.0.1:8385`)
3. `startup.future.await` — main event loop
4. Shutdown via `watch::Sender<bool>` (Ctrl+C, ConsoleCtrlEvent, `POST /rest/system/shutdown`)

Details: [`docs/agent/claude_reference.md`](docs/agent/claude_reference.md).

---

## Quick CLI / TUI / API

```bash
syncthing run                  # daemon foreground
syncthing tui                  # TUI client
syncthing init                 # interactive config wizard
syncthing status [--json]      # query daemon status
```

Default ports: BEP `22001`, REST API `8385` (loopback-only).

TUI keys: `F5` toggle daemon, `Tab`/`←→` switch tabs, `q` quit, `?` help, `a/e/d` add/edit/delete.

REST highlights: `GET /rest/health`, `GET /rest/system/status`, `GET /rest/events/poll`, `POST /rest/system/shutdown`.

Full reference: [`docs/agent/claude_reference.md`](docs/agent/claude_reference.md).

---

## Facts Register

`docs/KNOWN_ISSUES.md` is the authoritative project-wide bug tracker. It takes precedence over handoffs and NEXT_STEPS documents when verifying claims. New defects must be registered there.

For operational constraints and checklists, also consult [`docs/agent/`](docs/agent/index.md).

---

*Last updated: 2026-06-25 (trimmed to quick reference; detailed content moved to docs/agent/ OKF bundle).*
