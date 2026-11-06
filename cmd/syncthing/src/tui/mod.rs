/// Application state (`App`, `Tab`, `Popup`).
pub mod app;
/// BEP session handler bridging the sync service.
pub mod bep_handler;
/// Central TUI constants.
pub mod constants;
/// Daemon lifecycle helpers for the TUI (health check, F5 toggle).
pub mod daemon_controller;
/// Daemon assembly.
pub mod daemon_runner;
/// Global/local discovery tasks.
pub mod discovery_tasks;
/// Keyboard event dispatch.
pub mod events;
/// Unified form state, key handling, and rendering.
pub mod forms;
/// Local index update propagation to connected peers.
pub mod index_dispatcher;
/// Log file discovery and tail reading.
pub mod log_loader;
/// NAT traversal background tasks (port mapping, STUN).
pub mod nat_tasks;
/// Popup renderers.
pub mod popups;
/// Permanent-mode relay listeners.
pub mod relay_listener;
/// Per-session BEP event logging.
pub mod session_logger;
/// Semantic color theme.
pub mod theme;
/// Top-level frame layout.
pub mod ui;
/// Tab views (overview, devices, folders, logs).
pub mod views;
/// Internal health watchdog.
pub mod watchdog;
/// Reusable widgets (header, status bar, progress, log lines).
pub mod widgets;

/// Events bridged from the sync engine to the TUI.
#[derive(Debug, Clone)]
pub enum TuiEvent {
    FolderStateChanged {
        folder: String,
        status: syncthing_core::types::FolderStatus,
    },
    DeviceConnected {
        device_id: syncthing_core::DeviceId,
    },
    DeviceDisconnected {
        device_id: syncthing_core::DeviceId,
    },
    // TODO: TUI real-time sync progress bar
    #[allow(dead_code)]
    SyncProgress {
        folder: String,
        progress: f64,
    },
}

pub use runner::run_tui;

mod runner;
