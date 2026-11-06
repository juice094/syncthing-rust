# Tray ↔ TUI ↔ Daemon interaction sequence

> Source of truth for the tray/TUI/daemon choreography referenced by
> `docs/design/ADR-20261105-windows-tui-launch.md`. Keep in sync with
> `cmd/syncthing/src/main.rs` (Auto mode, daemon_controller, tray helpers) and
> `cmd/syncthing/src/tray_ipc.rs`.

## Auto-mode startup

```mermaid
sequenceDiagram
    participant M as MainThread
    participant C as DaemonControllerTask
    participant S as TrayIpcServer
    participant W as TrayWin32Thread
    participant D as DaemonPlusApi
    M->>M: acquire single-instance lock (C-UX-5)
    M->>C: spawn (mpsc DaemonCommand)
    M->>S: spawn TrayIpcServer (named pipe)
    M->>W: tray::spawn() (hidden window + message loop)
    M->>C: Start
    C->>D: start_daemon_services (daemon + REST API)
    C->>W: set_daemon_running(true)
    C->>C: spawn tray_status_loop (5s poll REST -> icon/tooltip)
```

## Open TUI from tray

```mermaid
sequenceDiagram
    participant U as User
    participant W as TrayWin32Thread
    participant M as MainThread
    participant T as TuiProcess
    participant S as TrayIpcServer
    U->>W: WM_LBUTTONDBLCLK / Open TUI menu
    W->>M: TrayEvent::OpenTui (mpsc)
    M->>M: spawn_tui_from_tray
    alt syncthing-tui.pid alive
        M->>T: focus_window_by_pid
    else
        M->>T: cmd /c start pwsh -EncodedCommand "...tui..." (fallback chain)
        T->>M: TrayIpcClient OpenTui / Ping (named pipe)
        S-->>T: TrayIpcResponse
    end
```

## F5 daemon toggle (tray-managed TUI)

```mermaid
sequenceDiagram
    participant T as TuiProcess
    participant S as TrayIpcServer
    participant M as MainThread
    participant C as DaemonControllerTask
    participant D as DaemonPlusApi
    T->>S: TrayIpcRequest::StartDaemon / StopDaemon
    S->>M: forward via mpsc
    M->>C: DaemonCommand::Start / Stop
    alt Start
        C->>D: start_daemon_services
    else Stop
        C->>D: shutdown_tx.send(true), await handles
    end
```
