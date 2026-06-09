# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Identity

`syncthing-rust` is a Rust reimplementation of the Syncthing BEP protocol for P2P file sync — zero runtime deps, single static binary, wire-compatible with Go Syncthing. Currently **v3.0.0** (production-grade), deployed on ROG-X (Windows 11) ↔ Gray-Cloud (Ubuntu 24.04) via Tailscale.

## Build & Test

```bash
# Full CI check (run before every commit)
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --workspace --all-targets

# Build specific binaries
cargo build --release --bin syncthing
cargo build --release --bin syncthing-tray          # Windows only
cargo build --release --bin syncthing-cli
cargo build --release --bin syncthing-bench
cargo build --release --bin syncthing-mcp-bridge
cargo build --release --bin syncthing-monitor

# Run a single test
cargo test -p syncthing-net --lib -- session::tests::test_session_ping_pong

# WSL cross-compile for Linux target
cargo build --release --target x86_64-unknown-linux-musl
```

## Architecture: Crate DAG

```
cmd/syncthing           # CLI entry (clap) + TUI (ratatui) + daemon lifecycle
  ├─ syncthing-api      # REST + WebSocket (axum), EventBus, handlers
  ├─ syncthing-net      # ConnectionManager, ParallelDialer, discovery, relay
  ├─ syncthing-sync     # Scanner, Puller, IndexHandler, folder_model, watcher
  ├─ syncthing-core     # Types (DeviceId, FileInfo, Vector), traits, no deps
  ├─ syncthing-fs       # Filesystem ops, .stignore matcher, scanner/hash
  ├─ syncthing-db       # Metadata store + block cache (sled-backed)
  ├─ syncthing-versioner # Simple (keep=N) + Staggered (time windows)
  └─ bep-protocol       # Wire protocol encode/decode (prost), handshake

cmd/syncthing-tray          # Windows tray icon (Win32 API), REST client, daemon lifecycle
cmd/syncthing-cli            # generate-cert, show-id, metrics-flush
cmd/syncthing-mcp-bridge     # MCP stdio ↔ REST API bridge
```

**Coupling rules** (from AGENTS.md):
- `syncthing-core` — no internal crate deps (pure traits + types)
- `syncthing-api` — may depend on `syncthing-core` only; must NOT hold concrete `ConnectionManagerHandle` or `LocalDatabase` types
- `cmd/syncthing` — glues everything together
- File soft limit: 600 lines per module

## Daemon Lifecycle

Entry point: `cmd/syncthing/src/main.rs` → `Commands::Run`:
1. `tui::daemon_runner::start_daemon()` — TLS, ConnectionManager, SyncService, NAT, relay pool, discovery
2. `api_server::start_api_server()` — binds REST API (default `0.0.0.0:8385`)
3. `startup.future.await` — main event loop (session cleanup, reconnect checks)
4. Shutdown: `watch::Sender<bool>` propagated to all subsystems (Ctrl+C, ConsoleCtrlEvent, REST API `/rest/system/shutdown`)

**API becomes available 30–90s after daemon start** — relay pool TLS health check (100 relays) runs before bind. In dev/testing, set `relays_enabled: false` in config to skip.

## Key APIs (syncthing-api)

Handler pattern: `Result<Json<T>, (StatusCode, String)>` (preferred, see `rest/folder.rs`).
All handlers receive `State<ApiState>` via axum. `ApiState` holds `Option<Arc<dyn FolderDatabase>>`, `Option<Arc<dyn SyncModel>>`, `EventBus`.

### Implemented endpoints (notable)
- `GET /rest/system/status` — system status (my_id, uptime, folder_count)
- `GET /rest/system/connections` — connection map (`{device_id: {connected, address, ...}}`)
- `GET /rest/db/browse?folder=X&prefix=Y&levels=N` — tree browse (batch A1)
- `GET /rest/db/file?folder=X&file=Y` — single file info (batch A2)
- `GET /rest/events/poll?since=N&limit=M` — REST long-poll (batch A3, 60s timeout)
- `GET /rest/events` — WebSocket upgrade
- `POST /rest/system/shutdown` — graceful shutdown

### Missing endpoints
- `db/browse`, `db/file` were recently added (unstaged changes); verify they are committed
- `GET /rest/system/ping` does NOT exist — use `/rest/health` instead

## UX Strategy: TUI + Tray as Primary, No Web GUI

Web GUI is permanently abandoned. TUI (ratatui) + system tray (Win32) is the sole interface.

| Channel | Entry | Use Case |
|---------|-------|----------|
| **Tray** | Double-click `syncthing-tray.exe` | Desktop user — always-on tray icon, auto-start daemon, right-click menu |
| **TUI** | `syncthing` (no args) | Power user — daemon auto-started in background, TUI in foreground |
| **CLI** | `syncthing status/devices/folders/logs` | Scripting/automation — JSON output available |
| **SSH** | `ssh server && syncthing` | Remote server — TUI in terminal over SSH |

The tray binary handles first-run auto-init (launches `syncthing init` if no config detected).

## CLI Commands (syncthing)

```
syncthing                            # Auto: start daemon + TUI (no args = default)
syncthing run                        # Start daemon only (foreground, headless)
syncthing tui                        # Launch TUI (connect to existing daemon)
syncthing init                       # Interactive config wizard
syncthing status [--json]            # Query daemon status via REST API
syncthing devices list               # List configured devices with online status
syncthing folders list [--status]    # List folders with sync state
syncthing logs --tail N              # Tail log file
syncthing install-autostart          # Windows: register in Run key
syncthing uninstall-autostart        # Windows: remove from Run key
```

## Known Test Flake

`test_session_block_request_response` — known flaky (Ping vs Response message race). Not a regression. 308/309 pass consistently.

## Facts Register

`docs/KNOWN_ISSUES.md` is the authoritative project-wide bug tracker. When making status claims, cross-reference it — it takes precedence over handoffs and NEXT_STEPS documents. Newly discovered defects must be explicitly registered there.

## Config

Defaults: BEP `0.0.0.0:22001`, REST API `0.0.0.0:8385`.
Config stored in `$LOCALAPPDATA/syncthing-rust/config.json` on Windows, `~/.config/syncthing-rust/config.json` on Linux.
Key options: `global_announce_enabled` (now respected), `relays_enabled`, `transports: ["tcp"]`.
`GET /rest/system/ping` does NOT exist — use `/rest/health` instead.
