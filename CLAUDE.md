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
cargo audit

# Build release (Windows desktop — includes tray)
cargo build --release --bin syncthing --features tray

# Build release (headless / server / embedded — no tray)
cargo build --release --bin syncthing --no-default-features

# Build auxiliary binaries
cargo build --release --bin syncthing-tray     # Thin wrapper (Windows only)
cargo build --release --bin syncthing-monitor  # Resource/health monitor
cargo build --release --bin syncthing-cli      # generate-cert, show-id, metrics-flush
cargo build --release --bin syncthing-bench    # Benchmarks
cargo build --release --bin syncthing-mcp-bridge  # MCP stdio ↔ REST API

# Run a single test
cargo test -p syncthing-net --lib -- session::tests::test_session_ping_pong

# Cross-compile Linux (musl)
cargo build --release --bin syncthing --no-default-features --target x86_64-unknown-linux-musl
```

## Feature Flags

| Feature | Default | Effect |
|---------|---------|--------|
| `tray` | ✅ | Windows system tray integration (`#![windows_subsystem = "windows"]`, Win32 `Shell_NotifyIconW`) |
| `websocket` | ❌ | Enables WebSocket transport in `syncthing-net` |

Headless builds exclude all Windows UI code:
```bash
cargo build --release --no-default-features  # Pure daemon, ~13MB
```

## Architecture: Crate DAG

```
cmd/syncthing              # CLI (clap) + TUI (ratatui) + daemon lifecycle + tray
  ├─ syncthing-api         # REST + WebSocket (axum), EventBus, handlers
  ├─ syncthing-net         # ConnectionManager, ParallelDialer, discovery, relay
  ├─ syncthing-sync        # Scanner, Puller, IndexHandler, folder_model, watcher
  ├─ syncthing-core        # Types (DeviceId, FileInfo, Vector), traits — no internal deps
  ├─ syncthing-fs          # Filesystem ops, .stignore matcher, scanner/hash
  ├─ syncthing-db          # Metadata store + block cache (sled-backed)
  ├─ syncthing-versioner   # Simple (keep=N) + Staggered (time windows)
  └─ bep-protocol          # Wire protocol encode/decode (prost), handshake

cmd/syncthing-tray          # 15-line thin wrapper — spawns syncthing.exe (backward compat)
cmd/syncthing-cli            # generate-cert, show-id, metrics-flush
cmd/syncthing-bench          # Criterion benchmarks
cmd/syncthing-mcp-bridge     # MCP stdio ↔ REST API bridge
```

**Coupling rules:**
- `syncthing-core` — no internal crate deps (pure traits + types)
- `syncthing-api` — depends on `syncthing-core` only; must NOT hold concrete `ConnectionManagerHandle` or `LocalDatabase`
- `cmd/syncthing` — glues everything together; tray code is behind `#[cfg(all(windows, feature = "tray"))]`
- File soft limit: 600 lines per module

## Daemon Lifecycle

Entry point: `cmd/syncthing/src/main.rs`:
1. `tui::daemon_runner::start_daemon()` — TLS, ConnectionManager, SyncService, NAT, relay pool, discovery
2. `api_server::start_api_server()` — binds REST API (default `0.0.0.0:8385`)
3. `startup.future.await` — main event loop (session cleanup, reconnect checks)
4. Shutdown: `watch::Sender<bool>` propagated to all subsystems (Ctrl+C, ConsoleCtrlEvent, REST API `/rest/system/shutdown`)

**API becomes available 30–90s after daemon start** — relay pool TLS health check (100 relays) runs before bind. In dev/testing, set `relays_enabled: false` in config to skip.

## CLI Commands

```
syncthing                        # No args: daemon + tray (Windows) or daemon only (Linux)
syncthing run                    # Daemon only (foreground, no tray)
syncthing tui                    # TUI client (connects to existing daemon)
syncthing init                   # Interactive config wizard
syncthing status [--json]        # Query daemon status via REST API
syncthing devices list           # List configured devices with online status
syncthing folders list [--status]  # List folders with sync state
syncthing logs --tail N          # Tail log file (last N lines)
syncthing install-autostart      # Windows: register HKCU Run key
syncthing uninstall-autostart    # Windows: remove HKCU Run key
```

## Windows Desktop Mode

`syncthing.exe` (no args) starts daemon + in-process tray icon. Process model:

```
Main thread (tokio)
  ├── daemon_runner::start_daemon()  ← BEP sync engine
  ├── api_server::start_api_server() ← REST API on :8385
  └── tokio::spawn(tray_status_loop) ← 5s polling, icon/tooltip updates

Background thread
  └── Win32 message loop (hidden window + Shell_NotifyIconW + context menu)
```

`syncthing-tray.exe` is a thin wrapper that finds `syncthing.exe` in the same directory and spawns it. It exists solely for backward compatibility with existing shortcuts and autostart registry entries.

Tray sources in `cmd/syncthing/src/`:
- `tray.rs` — Win32 `Shell_NotifyIconW`, hidden window, context menu, icon/tooltip/notification helpers
- `tray_api.rs` — `DaemonClient` REST client for status polling
- `build.rs` — generates 32×32 hard-drive ICO in `OUT_DIR`

## TUI Key Bindings

| Key | Context | Action |
|-----|---------|--------|
| `F5` | Global | Start / Stop daemon |
| `Tab` / `←→` | Global | Switch tab (Overview / Devices / Folders / Logs) |
| `l` | Global | Cycle log filter level (Error→Warn→Info→Debug→Trace) |
| `q` | No popup | Quit TUI |
| `?` | No popup | Help overlay |
| `Insert` / `a` | Devices / Folders tab | Add new item |
| `Enter` / `e` | Devices / Folders tab | Edit selected item |
| `Delete` / `d` | Devices / Folders tab | Delete selected item |
| `i` | Folders tab | Open `.stignore` in system editor |
| `↑↓` | List views | Navigate items |

Popups (Add/Edit forms): `Tab`/`↑↓` navigate fields, `Space` toggles device checkboxes, `Ctrl+V` pastes, `Enter` saves, `Esc` cancels.

## Config

Defaults: BEP `0.0.0.0:22001`, REST API `0.0.0.0:8385`.
Config: `$LOCALAPPDATA/syncthing-rust/config.json` (Windows), `~/.config/syncthing-rust/config.json` (Linux).
Key options: `global_announce_enabled`, `relays_enabled`, `transports: ["tcp"]`.
`GET /rest/health` is the ping endpoint — `/rest/system/ping` does not exist.

## Key REST APIs (syncthing-api)

Handler pattern: `Result<Json<T>, (StatusCode, String)>`. All handlers receive `State<ApiState>`.

Notable endpoints:
- `GET /rest/health` — daemon liveness check
- `GET /rest/system/status` — my_id, uptime, folder_count
- `GET /rest/system/connections` — `{device_id: {connected, address, ...}}`
- `GET /rest/db/browse?folder=X&prefix=Y&levels=N` — tree browse
- `GET /rest/db/file?folder=X&file=Y` — single file metadata
- `GET /rest/events/poll?since=N&limit=M` — REST long-poll (60s timeout)
- `GET /rest/events` — WebSocket upgrade
- `POST /rest/system/shutdown` — graceful shutdown

## Known Test Flake

`test_session_block_request_response` — Ping vs Response message race. Not a regression. `test_two_node_single_file_sync` — ClusterConfig race under parallel test load (T3.1b health check mitigates in production, verified against Go syncthing v2.1.0).

## Facts Register

`docs/KNOWN_ISSUES.md` is the authoritative project-wide bug tracker. It takes precedence over handoffs and NEXT_STEPS documents when verifying claims. New defects must be registered there.
