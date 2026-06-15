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

/// 当前激活的 Tab
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Devices,
    Folders,
    Logs,
}

impl Tab {
    pub fn next(self) -> Self {
        match self {
            Tab::Overview => Tab::Devices,
            Tab::Devices => Tab::Folders,
            Tab::Folders => Tab::Logs,
            Tab::Logs => Tab::Overview,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Tab::Overview => Tab::Logs,
            Tab::Devices => Tab::Overview,
            Tab::Folders => Tab::Devices,
            Tab::Logs => Tab::Folders,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Devices => "Devices",
            Tab::Folders => "Folders",
            Tab::Logs => "Logs",
        }
    }
}

/// 弹窗状态
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

/// 全局 App 状态
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
    /// 标记当前 daemon 是否由外部进程（如 Auto 模式的托盘）启动。
    /// 若为 true，TUI 不应尝试启动/停止它，避免重复实例。
    pub external_daemon: bool,
    /// 外部 daemon（托盘）的 PID。用于检测托盘是否意外退出，以便 TUI 切回本地管理。
    pub external_daemon_pid: Option<u32>,
    /// 托盘 IPC 管道名。由托盘打开 TUI 时传入，用于 F5 跨进程控制托盘 daemon。
    pub tray_pipe: Option<String>,
    /// 托盘 IPC 客户端。连接成功后 TUI 可通过它发送 StartDaemon / StopDaemon。
    pub tray_client: Option<TrayIpcClient>,
    pub connected_devices: Vec<DeviceId>,
    pub theme: Theme,

    /// 运行中的 sync_service 引用（用于配置变更通知）
    pub sync_service: Option<Arc<dyn syncthing_sync::SyncManager>>,

    /// 文件夹实时状态缓存（来自 sync engine 事件）
    pub folder_states: HashMap<String, FolderStatus>,

    /// 同步进度缓存（folder -> 0.0~1.0）
    pub sync_progress: HashMap<String, f64>,

    /// 最近一次收到 SyncProgress 事件的时间戳
    pub last_sync_progress_update: Option<Instant>,

    /// 事件接收器（由 daemon 启动时设置）
    pub event_rx: Option<tokio::sync::mpsc::Receiver<crate::tui::TuiEvent>>,

    /// 日志过滤级别（TUI 快捷键 l 切换）
    pub log_filter_level: tracing::Level,

    /// 日志视图滚动偏移（从底部往上数，0 表示最新）
    pub log_scroll_offset: usize,
    /// 当前搜索关键词；None 表示未在搜索模式
    pub log_search: Option<String>,
    /// 搜索匹配行的索引（相对于 log_lines，0 表示最旧）
    pub log_search_matches: Vec<usize>,
    /// 当前高亮的匹配索引（相对于 log_search_matches）
    pub log_search_selected: usize,

    /// 当前列表过滤词（Devices / Folders 共享）
    pub list_filter: Option<String>,
    /// Devices 列表过滤后匹配项在 config.devices 中的原始索引
    pub device_filter_matches: Vec<usize>,
    /// Folders 列表过滤后匹配项在 config.folders 中的原始索引
    pub folder_filter_matches: Vec<usize>,
    /// 过滤后列表中的当前选中位置
    pub list_filter_selected: usize,

    // 统一表单状态（有弹窗时 Some，无弹窗时 None）
    pub form: Option<FormState>,
    pub folder_device_selection: Vec<bool>,
    pub folder_device_selected: usize,

    /// 确认对话框的回调（弹出 Confirm 时设置，确认后执行）
    #[allow(clippy::type_complexity)]
    pub confirm_callback: Option<Box<dyn FnOnce(&mut App)>>,
}

impl App {
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

    pub fn push_log(&mut self, msg: String) {
        if self.log_lines.len() >= constants::LOG_BUFFER_CAP {
            self.log_lines.pop_front();
        }
        self.log_lines.push_back(msg);
        if self.log_search.is_some() {
            self.recompute_log_search_matches();
        }
    }

    /// 根据当前 `log_search` 重新计算匹配行索引。
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

    /// 根据当前 `list_filter` 重新计算 Devices 过滤匹配列表，并同步修正选中索引。
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

    /// 根据当前 `list_filter` 重新计算 Folders 过滤匹配列表，并同步修正选中索引。
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

    /// 当前日志视图允许的最大滚动偏移（保证至少保留一行可见）。
    pub fn max_log_scroll_offset(&self) -> usize {
        if self.log_search.is_some() && !self.log_search_matches.is_empty() {
            self.log_search_matches.len().saturating_sub(1)
        } else {
            self.log_lines.len().saturating_sub(1)
        }
    }

    #[allow(dead_code)]
    pub fn selected_device(&self) -> Option<&Device> {
        self.config.devices.get(self.device_selected)
    }

    #[allow(dead_code)]
    pub fn selected_folder(&self) -> Option<&Folder> {
        self.config.folders.get(self.folder_selected)
    }

    pub fn resize_form(&mut self) {
        let count = self.config.devices.len().max(1);
        self.folder_device_selection.resize(count, false);
    }
}
