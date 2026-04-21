use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use syncthing_core::types::{Config, Device, Folder};
use syncthing_core::DeviceId;

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
    Help,
    Error(String),
}

/// 输入表单状态
#[derive(Debug, Clone, Default)]
pub struct FormState {
    pub fields: Vec<String>,
    pub focus: usize,
}

impl FormState {
    pub fn new(fields: Vec<String>) -> Self {
        Self { fields, focus: 0 }
    }
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
    pub connected_devices: Vec<DeviceId>,
    pub theme: Theme,

    /// 运行中的 sync_service 引用（用于配置变更通知）
    pub sync_service: Option<Arc<dyn syncthing_sync::SyncModel>>,

    // 表单
    pub device_form: FormState,
    pub folder_form: FormState,
    pub folder_device_selection: Vec<bool>, // 对应 config.devices 的多选
    pub folder_device_selected: usize,      // 当前高亮的设备列表项
}

impl App {
    pub fn new(config_dir: PathBuf, listen: String, device_name: String, config: Config) -> Self {
        let device_count = config.devices.len().max(1);
        Self {
            config_dir,
            listen,
            device_name,
            config,
            tab: Tab::Overview,
            popup: Popup::None,
            device_selected: 0,
            folder_selected: 0,
            log_lines: VecDeque::with_capacity(100),
            daemon_running: false,
            daemon_status: "Stopped".to_string(),
            connected_devices: Vec::new(),
            theme: Theme::default(),
            device_form: FormState::new(vec![String::new(), String::new(), String::new()]),
            folder_form: FormState::new(vec![String::new(), String::new()]),
            folder_device_selection: vec![false; device_count],
            folder_device_selected: 0,
            sync_service: None,
        }
    }

    pub fn push_log(&mut self, msg: String) {
        if self.log_lines.len() >= 100 {
            self.log_lines.pop_front();
        }
        self.log_lines.push_back(msg);
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
