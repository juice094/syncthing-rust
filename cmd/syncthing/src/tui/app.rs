use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use syncthing_core::types::{Config, Device, Folder, FolderStatus};
use syncthing_core::DeviceId;

use crate::tray_ipc::TrayIpcClient;
use crate::tui::constants;
use crate::tui::forms::FormState;
use crate::tui::theme::Theme;

/// Currently active tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Devices,
    Folders,
    Logs,
}

impl Tab {
    /// Advances to the next tab, wrapping around.
    pub fn next(self) -> Self {
        match self {
            Tab::Overview => Tab::Devices,
            Tab::Devices => Tab::Folders,
            Tab::Folders => Tab::Logs,
            Tab::Logs => Tab::Overview,
        }
    }

    /// Goes back to the previous tab, wrapping around.
    pub fn prev(self) -> Self {
        match self {
            Tab::Overview => Tab::Logs,
            Tab::Devices => Tab::Overview,
            Tab::Folders => Tab::Devices,
            Tab::Logs => Tab::Folders,
        }
    }

    /// Returns the tab title.
    pub fn title(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Devices => "Devices",
            Tab::Folders => "Folders",
            Tab::Logs => "Logs",
        }
    }
}

/// Popup state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Popup {
    None,
    AddDevice,
    AddFolder,
    EditDevice,
    EditFolder,
    Help,
    Error(String),
    Search { query: String },
    Filter { query: String },
    Confirm { title: String, message: String },
}

/// Global TUI application state.
pub struct App {
    pub config_dir: PathBuf,
    pub listen: String,
    pub device_name: String,

    pub config: Config,
    pub tab: Tab,
    pub popup: Popup,
    pub device_selected: usize,
    pub folder_selected: usize,
    pub log_lines: VecDeque<String>,

    pub daemon_running: bool,
    pub daemon_status: String,
    /// Whether the current daemon belongs to an external process (e.g. the
    /// Auto-mode tray). When true, the TUI must not start/stop it.
    pub external_daemon: bool,
    /// PID of the external (tray) daemon; used to detect tray exit so the TUI can resume local control.
    pub external_daemon_pid: Option<u32>,
    /// Tray IPC pipe name, passed when the tray opens the TUI; enables cross-process F5 daemon control.
    pub tray_pipe: Option<String>,
    /// Tray IPC client; when connected the TUI sends StartDaemon / StopDaemon through it.
    pub tray_client: Option<TrayIpcClient>,
    pub connected_devices: Vec<DeviceId>,
    pub theme: Theme,

    /// Handle to the running sync service (notified of config changes).
    pub sync_service: Option<Arc<dyn syncthing_sync::SyncManager>>,

    /// Live per-folder status cache (from sync engine events).
    pub folder_states: HashMap<String, FolderStatus>,

    /// Sync progress cache (folder -> 0.0..=1.0).
    pub sync_progress: HashMap<String, f64>,

    /// Timestamp of the most recent SyncProgress event.
    pub last_sync_progress_update: Option<Instant>,

    /// Event receiver (installed when the daemon starts).
    pub event_rx: Option<tokio::sync::mpsc::Receiver<crate::tui::TuiEvent>>,

    /// Log filter level (toggled with the `l` key).
    pub log_filter_level: tracing::Level,

    /// Log view scroll offset (counted from the bottom; 0 = newest).
    pub log_scroll_offset: usize,
    /// Current search pattern; None when not searching.
    pub log_search: Option<String>,
    /// Indices of matching lines (into `log_lines`; 0 = oldest).
    pub log_search_matches: Vec<usize>,
    /// Index of the currently highlighted match (into `log_search_matches`).
    pub log_search_selected: usize,

    /// Current list filter query (shared by Devices / Folders).
    pub list_filter: Option<String>,
    /// Original indices into `config.devices` after filtering.
    pub device_filter_matches: Vec<usize>,
    /// Original indices into `config.folders` after filtering.
    pub folder_filter_matches: Vec<usize>,
    /// Selection position within the filtered list.
    pub list_filter_selected: usize,

    /// Unified form state (Some while a form popup is open).
    pub form: Option<FormState>,
    pub folder_device_selection: Vec<bool>,
    pub folder_device_selected: usize,

    /// Callback executed when the Confirm popup is accepted.
    #[allow(clippy::type_complexity)]
    pub confirm_callback: Option<Box<dyn FnOnce(&mut App)>>,
}

impl App {
    /// Creates the initial app state for the given config directory and identity.
    pub fn new(config_dir: PathBuf, listen: String, device_name: String, config: Config) -> Self {
        let device_count = config.devices.len().max(1);
        let device_filter_matches: Vec<usize> = (0..config.devices.len()).collect();
        let folder_filter_matches: Vec<usize> = (0..config.folders.len()).collect();
        Self {
            config_dir,
            listen,
            device_name,
            config,
            tab: Tab::Overview,
            popup: Popup::None,
            device_selected: 0,
            folder_selected: 0,
            log_lines: VecDeque::with_capacity(constants::LOG_BUFFER_CAP),
            daemon_running: false,
            daemon_status: "Stopped".to_string(),
            external_daemon: false,
            external_daemon_pid: None,
            tray_pipe: None,
            tray_client: None,
            connected_devices: Vec::new(),
            theme: Theme::default(),
            form: None,
            folder_device_selection: vec![false; device_count],
            folder_device_selected: 0,
            confirm_callback: None,
            sync_service: None,
            folder_states: HashMap::new(),
            sync_progress: HashMap::new(),
            last_sync_progress_update: None,
            event_rx: None,
            log_filter_level: tracing::Level::INFO,
            log_scroll_offset: 0,
            log_search: None,
            log_search_matches: Vec::new(),
            log_search_selected: 0,
            list_filter: None,
            device_filter_matches,
            folder_filter_matches,
            list_filter_selected: 0,
        }
    }

    /// Appends a line to the log buffer, evicting the oldest when full.
    pub fn push_log(&mut self, msg: String) {
        if self.log_lines.len() >= constants::LOG_BUFFER_CAP {
            self.log_lines.pop_front();
        }
        self.log_lines.push_back(msg);
        if self.log_search.is_some() {
            self.recompute_log_search_matches();
        }
    }

    /// Recomputes match indices for the current `log_search`.
    pub fn recompute_log_search_matches(&mut self) {
        self.log_search_matches.clear();
        if let Some(pattern) = &self.log_search {
            if !pattern.is_empty() {
                let lower = pattern.to_lowercase();
                for (idx, line) in self.log_lines.iter().enumerate() {
                    if line.to_lowercase().contains(&lower) {
                        self.log_search_matches.push(idx);
                    }
                }
            }
        }
        if self.log_search_selected >= self.log_search_matches.len() {
            self.log_search_selected = self.log_search_matches.len().saturating_sub(1);
        }
    }

    /// Recomputes the filtered Devices match list for `list_filter` and fixes up the selection.
    pub fn recompute_device_filter_matches(&mut self) {
        self.device_filter_matches.clear();
        if let Some(ref q) = self.list_filter {
            let lower = q.to_lowercase();
            for (i, d) in self.config.devices.iter().enumerate() {
                let name = d.name.as_deref().unwrap_or("").to_lowercase();
                let id = d.id.to_string().to_lowercase();
                let addr_match = d
                    .addresses
                    .iter()
                    .any(|a| a.as_str().to_lowercase().contains(&lower));
                if name.contains(&lower) || id.contains(&lower) || addr_match {
                    self.device_filter_matches.push(i);
                }
            }
        } else {
            self.device_filter_matches
                .extend(0..self.config.devices.len());
        }
        if self.list_filter_selected >= self.device_filter_matches.len() {
            self.list_filter_selected = self.device_filter_matches.len().saturating_sub(1);
        }
        if let Some(&idx) = self.device_filter_matches.get(self.list_filter_selected) {
            self.device_selected = idx;
        }
    }

    /// Recomputes the filtered Folders match list for `list_filter` and fixes up the selection.
    pub fn recompute_folder_filter_matches(&mut self) {
        self.folder_filter_matches.clear();
        if let Some(ref q) = self.list_filter {
            let lower = q.to_lowercase();
            for (i, f) in self.config.folders.iter().enumerate() {
                let id = f.id.to_lowercase();
                let path = f.path.to_lowercase();
                let devs = f
                    .devices
                    .iter()
                    .map(|d| d.to_string().to_lowercase())
                    .collect::<Vec<_>>()
                    .join(" ");
                if id.contains(&lower) || path.contains(&lower) || devs.contains(&lower) {
                    self.folder_filter_matches.push(i);
                }
            }
        } else {
            self.folder_filter_matches
                .extend(0..self.config.folders.len());
        }
        if self.list_filter_selected >= self.folder_filter_matches.len() {
            self.list_filter_selected = self.folder_filter_matches.len().saturating_sub(1);
        }
        if let Some(&idx) = self.folder_filter_matches.get(self.list_filter_selected) {
            self.folder_selected = idx;
        }
    }

    /// Maximum scroll offset for the current log view (keeps one line visible).
    pub fn max_log_scroll_offset(&self) -> usize {
        if self.log_search.is_some() && !self.log_search_matches.is_empty() {
            self.log_search_matches.len().saturating_sub(1)
        } else {
            self.log_lines.len().saturating_sub(1)
        }
    }

    /// Returns the currently selected device, if any.
    #[allow(dead_code)]
    pub fn selected_device(&self) -> Option<&Device> {
        self.config.devices.get(self.device_selected)
    }

    /// Returns the currently selected folder, if any.
    #[allow(dead_code)]
    pub fn selected_folder(&self) -> Option<&Folder> {
        self.config.folders.get(self.folder_selected)
    }

    /// Resizes the folder device-selection vector to the device count.
    pub fn resize_form(&mut self) {
        let count = self.config.devices.len().max(1);
        self.folder_device_selection.resize(count, false);
    }
}
