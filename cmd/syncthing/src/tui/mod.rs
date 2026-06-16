pub mod app;
pub mod bep_handler;
pub mod constants;
pub mod daemon_controller;
pub mod daemon_runner;
pub mod discovery_tasks;
pub mod events;
pub mod forms;
pub mod index_dispatcher;
pub mod log_loader;
pub mod nat_tasks;
pub mod popups;
pub mod relay_listener;
pub mod session_logger;
pub mod theme;
pub mod ui;
pub mod views;
pub mod watchdog;
pub mod widgets;

/// Sync engine → TUI 事件
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
