//! 核心类型定义

use chrono::{DateTime, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;

/// 连接状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum ConnectionState {
    /// 初始状态
    #[default]
    Initial,
    /// 连接中
    Connecting,
    /// 已连接
    Connected,
    /// TLS握手完成
    TlsHandshakeComplete,
    /// 协议握手完成（Hello交换）
    ProtocolHandshakeComplete,
    /// 集群配置交换完成
    ClusterConfigComplete,
    /// 正在断开
    Disconnecting,
    /// 已断开
    Disconnected,
    /// 错误状态
    Error,
}

impl ConnectionState {
    /// 是否处于活跃状态（可用于传输数据）
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            ConnectionState::Connected
                | ConnectionState::TlsHandshakeComplete
                | ConnectionState::ProtocolHandshakeComplete
                | ConnectionState::ClusterConfigComplete
        )
    }
    
    /// 是否可以发送消息
    pub fn can_send(&self) -> bool {
        matches!(
            self,
            ConnectionState::ProtocolHandshakeComplete | ConnectionState::ClusterConfigComplete
        )
    }
    
    /// 是否已终止
    pub fn is_terminated(&self) -> bool {
        matches!(
            self,
            ConnectionState::Disconnected | ConnectionState::Error
        )
    }
}


/// 地址类型
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AddressType {
    /// TCP地址
    Tcp(String),
    /// QUIC地址
    Quic(String),
    /// Relay地址
    Relay(String),
    /// 动态发现
    Dynamic,
}

impl AddressType {
    /// 获取地址字符串
    pub fn as_str(&self) -> &str {
        match self {
            AddressType::Tcp(addr) => addr,
            AddressType::Quic(addr) => addr,
            AddressType::Relay(addr) => addr,
            AddressType::Dynamic => "dynamic",
        }
    }
}

impl fmt::Display for AddressType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// 连接统计信息
#[derive(Debug, Clone, Default)]
pub struct ConnectionStats {
    /// 连接建立时间
    pub connected_at: Option<DateTime<Utc>>,
    /// 最后活动时间
    pub last_activity: Option<DateTime<Utc>>,
    /// 发送的字节数
    pub bytes_sent: u64,
    /// 接收的字节数
    pub bytes_received: u64,
    /// 发送的消息数
    pub messages_sent: u64,
    /// 接收的消息数
    pub messages_received: u64,
    /// 重试次数
    pub retry_count: u32,
}

/// 连接优先级
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(Default)]
pub enum ConnectionPriority {
    /// 最低优先级
    Lowest = 0,
    /// 低优先级
    Low = 1,
    /// 正常优先级
    #[default]
    Normal = 2,
    /// 高优先级
    High = 3,
    /// 最高优先级
    Highest = 4,
}


/// 连接类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionType {
    /// 传入连接
    Incoming,
    /// 传出连接（拨号）
    Outgoing,
}

/// 重试策略配置
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// 最大重试次数
    pub max_retries: u32,
    /// 初始退避时间（毫秒）
    pub initial_backoff_ms: u64,
    /// 最大退避时间（毫秒）
    pub max_backoff_ms: u64,
    /// 退避乘数
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 10,
            initial_backoff_ms: 1000,
            max_backoff_ms: 300000, // 5分钟
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryConfig {
    /// 计算第n次重试的退避时间
    pub fn backoff_duration(&self, attempt: u32) -> std::time::Duration {
        if attempt == 0 {
            return std::time::Duration::from_millis(self.initial_backoff_ms);
        }
        
        let multiplier = self.backoff_multiplier.powi(attempt as i32);
        let backoff_ms = (self.initial_backoff_ms as f64 * multiplier) as u64;
        let backoff_ms = backoff_ms.min(self.max_backoff_ms);
        
        // 添加抖动（±25%）
        let jitter = rand::random::<f64>() * 0.5 - 0.25;
        let jittered_ms = (backoff_ms as f64 * (1.0 + jitter)) as u64;
        
        std::time::Duration::from_millis(jittered_ms.max(100))
    }
}

// ============================================
// 同步相关类型定义
// ============================================

/// 文件类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileType {
    /// 普通文件
    File,
    /// 目录
    Directory,
    /// 符号链接
    Symlink,
}

/// 块信息
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlockInfo {
    /// 块大小
    pub size: i32,
    /// 块哈希（SHA-256）
    pub hash: Vec<u8>,
    /// 块在文件中的偏移量
    pub offset: i64,
}

/// 版本向量 - 用于冲突检测和解决
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Vector {
    /// 设备ID到计数器的映射
    pub counters: HashMap<u64, u64>,
}

impl Vector {
    /// 创建新的空版本向量
    pub fn new() -> Self {
        Self {
            counters: HashMap::new(),
        }
    }

    /// 添加/更新设备计数器
    pub fn with_counter(mut self, device_id: u64, counter: u64) -> Self {
        self.counters.insert(device_id, counter);
        self
    }

    /// 递增指定设备的计数器
    pub fn increment(&mut self, device_id: u64) {
        *self.counters.entry(device_id).or_insert(0) += 1;
    }

    /// 获取指定设备的计数器值
    pub fn get(&self, device_id: u64) -> u64 {
        self.counters.get(&device_id).copied().unwrap_or(0)
    }

    /// 比较两个版本向量
    pub fn compare(&self, other: &Vector) -> VersionComparison {
        let mut has_greater = false;
        let mut has_less = false;

        // 检查所有设备
        let all_devices: std::collections::HashSet<_> = self
            .counters
            .keys()
            .chain(other.counters.keys())
            .collect();

        for device in all_devices {
            let self_count = self.get(*device);
            let other_count = other.get(*device);

            if self_count > other_count {
                has_greater = true;
            } else if self_count < other_count {
                has_less = true;
            }
        }

        match (has_greater, has_less) {
            (true, true) => VersionComparison::Conflict,
            (true, false) => VersionComparison::Greater,
            (false, true) => VersionComparison::Less,
            (false, false) => VersionComparison::Equal,
        }
    }

    /// 检查此版本是否支配（dominates）另一个版本
    /// 如果对于所有设备，此版本的计数器都 >= 另一个版本的计数器，则支配
    pub fn dominates(&self, other: &Vector) -> bool {
        // 检查所有设备
        let all_devices: std::collections::HashSet<_> = self
            .counters
            .keys()
            .chain(other.counters.keys())
            .collect();

        for device in all_devices {
            let self_count = self.get(*device);
            let other_count = other.get(*device);

            if self_count < other_count {
                return false;
            }
        }

        true
    }
}

/// 版本比较结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionComparison {
    Equal,
    Greater,
    Less,
    Conflict,
}

/// 索引ID（8字节随机值）
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[derive(Default)]
pub struct IndexID(pub [u8; 8]);

impl IndexID {
    /// 生成新的随机IndexID
    pub fn random() -> Self {
        let mut bytes = [0u8; 8];
        rand::thread_rng().fill(&mut bytes);
        Self(bytes)
    }

    /// 从u64创建IndexID
    pub fn from_u64(value: u64) -> Self {
        Self(value.to_be_bytes())
    }

    /// 转换为u64
    pub fn as_u64(&self) -> u64 {
        u64::from_be_bytes(self.0)
    }
}

impl fmt::Debug for IndexID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IndexID({:016x})", self.as_u64())
    }
}


/// 索引增量
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexDelta {
    /// 文件夹ID
    pub folder: String,
    /// 索引ID
    pub index_id: IndexID,
    /// 起始序列号
    pub start_sequence: u64,
    /// 文件列表
    pub files: Vec<FileInfo>,
}

/// 文件信息
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileInfo {
    /// 文件名（相对路径）
    pub name: String,
    /// 文件类型
    pub file_type: FileType,
    /// 文件大小
    pub size: i64,
    /// 文件权限（Unix模式）
    pub permissions: u32,
    /// 修改时间（秒）
    pub modified_s: i64,
    /// 修改时间（纳秒部分）
    pub modified_ns: i32,
    /// 版本向量
    pub version: Vector,
    /// 序列号（用于索引排序）
    pub sequence: u64,
    /// 块大小
    pub block_size: i32,
    /// 块列表
    pub blocks: Vec<BlockInfo>,
    /// 符号链接目标（如果是符号链接）
    pub symlink_target: Option<String>,
    /// 删除标记
    pub deleted: Option<bool>,
}

impl FileInfo {
    /// 创建新的 FileInfo（仅设置文件名，其余为默认值）
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            file_type: FileType::File,
            size: 0,
            permissions: 0,
            modified_s: 0,
            modified_ns: 0,
            version: Vector::new(),
            sequence: 0,
            block_size: 0,
            blocks: Vec::new(),
            symlink_target: None,
            deleted: Some(false),
        }
    }

    /// 检查文件是否被删除
    pub fn is_deleted(&self) -> bool {
        self.deleted.unwrap_or(false)
    }

    /// 标记文件为已删除
    pub fn mark_deleted(&mut self) {
        self.deleted = Some(true);
    }
}

/// 文件夹类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FolderType {
    /// 发送接收（双向同步）
    SendReceive,
    /// 仅发送
    SendOnly,
    /// 仅接收
    ReceiveOnly,
    /// 接收加密
    ReceiveEncrypted,
}

impl FolderType {
    /// 是否可以发送变更
    pub fn can_send(&self) -> bool {
        matches!(self, FolderType::SendReceive | FolderType::SendOnly)
    }

    /// 是否可以接收变更
    pub fn can_sync(&self) -> bool {
        matches!(
            self,
            FolderType::SendReceive | FolderType::ReceiveOnly | FolderType::ReceiveEncrypted
        )
    }
}

/// 文件夹状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FolderStatus {
    /// 空闲
    Idle,
    /// 等待扫描
    ScanWaiting,
    /// 正在扫描
    Scanning,
    /// 等待同步
    SyncWaiting,
    /// 正在同步（拉取）
    Pulling,
    /// 正在同步（推送）
    Pushing,
    /// 同步完成
    Synced,
    /// 暂停
    Paused,
    /// 错误
    Error,
}

/// 压缩模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Compression {
    #[default]
    Metadata,
    Always,
    Never,
}

/// API 设备配置
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub id: String,
    pub name: String,
    pub addresses: Vec<String>,
    pub paused: bool,
    pub introducer: bool,
}

/// API 文件夹配置
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FolderConfig {
    pub id: String,
    pub label: String,
    pub path: String,
    pub devices: Vec<String>,
    pub rescan_interval_secs: u32,
    pub versioning: VersioningConfig,
}

/// GUI 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiConfig {
    pub enabled: bool,
    pub address: String,
    pub api_key: String,
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            address: "0.0.0.0:8385".to_string(),
            api_key: String::new(),
        }
    }
}

/// 选项配置
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Options {
    pub listen_addresses: Vec<String>,
    pub global_announce_enabled: bool,
    pub local_announce_enabled: bool,
    pub relays_enabled: bool,
}

/// 版本控制配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[derive(Default)]
pub enum VersioningConfig {
    #[default]
    None,
    Simple { params: HashMap<String, String> },
    Staggered { params: HashMap<String, String> },
    External { params: HashMap<String, String> },
}


/// 文件夹配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    /// 文件夹ID
    pub id: String,
    /// 文件夹路径
    pub path: String,
    /// 文件夹标签（可选）
    pub label: Option<String>,
    /// 文件夹类型
    pub folder_type: FolderType,
    /// 是否暂停
    pub paused: bool,
    /// 重新扫描间隔（秒）
    pub rescan_interval_secs: i32,
    /// 设备列表（哪些设备共享此文件夹）
    pub devices: Vec<crate::DeviceId>,
    /// 忽略模式
    pub ignore_patterns: Vec<String>,
    /// 版本控制配置
    pub versioning: Option<VersioningConfig>,
}

impl Folder {
    /// 创建新的文件夹配置
    pub fn new(id: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            path: path.into(),
            label: None,
            folder_type: FolderType::SendReceive,
            paused: false,
            rescan_interval_secs: 3600, // 默认1小时
            devices: Vec::new(),
            ignore_patterns: Vec::new(),
            versioning: None,
        }
    }
}

/// 设备配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    /// 设备ID
    pub id: crate::DeviceId,
    /// 设备名称
    pub name: Option<String>,
    /// 设备地址列表
    pub addresses: Vec<AddressType>,
    /// 是否暂停
    pub paused: bool,
    /// 是否 introducer
    pub introducer: bool,
}

/// 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// 版本
    pub version: i32,
    /// 监听地址
    #[serde(default = "default_listen")]
    pub listen_addr: String,
    /// 设备名称
    #[serde(default = "default_device_name")]
    pub device_name: String,
    /// 文件夹列表
    pub folders: Vec<Folder>,
    /// 设备列表
    pub devices: Vec<Device>,
    /// 本地设备ID
    pub local_device_id: Option<crate::DeviceId>,
    /// GUI 配置
    #[serde(default)]
    pub gui: GuiConfig,
    /// 选项配置
    #[serde(default)]
    pub options: Options,
}

fn default_listen() -> String { "0.0.0.0:22001".to_string() }
fn default_device_name() -> String { "syncthing-rust".to_string() }

impl Config {
    /// 创建新的空配置
    pub fn new() -> Self {
        Self {
            version: 1,
            listen_addr: default_listen(),
            device_name: default_device_name(),
            folders: Vec::new(),
            devices: Vec::new(),
            local_device_id: None,
            gui: GuiConfig::default(),
            options: Options::default(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

/// 完整索引消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    /// 文件夹ID
    pub folder: String,
    /// 文件列表
    pub files: Vec<FileInfo>,
}

/// 索引更新消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexUpdate {
    /// 文件夹ID
    pub folder: String,
    /// 更新的文件列表
    pub files: Vec<FileInfo>,
}

/// Folder identifier
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FolderId(String);

impl FolderId {
    /// Create from string
    pub fn new<S: Into<String>>(id: S) -> Self {
        Self(id.into())
    }

    /// Get as string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for FolderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FolderId({})", self.0)
    }
}

impl fmt::Display for FolderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Block hash (SHA-256)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlockHash([u8; 32]);

impl BlockHash {
    /// Create from raw bytes
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Get raw bytes
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Calculate hash from data
    pub fn from_data(data: &[u8]) -> Self {
        let hash = Sha256::digest(data);
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&hash);
        Self(bytes)
    }

    /// Convert to Vec<u8>
    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }
}

impl fmt::Debug for BlockHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlockHash({})", hex::encode(&self.0[..8]))
    }
}

impl fmt::Display for BlockHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

/// Folder sync summary
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FolderSummary {
    /// Total files
    pub files: u64,
    /// Total directories
    pub directories: u64,
    /// Total symlinks
    pub symlinks: u64,
    /// Total bytes
    pub bytes: u64,
    /// Files needing sync
    pub need_files: u64,
    /// Directories needing sync
    pub need_directories: u64,
    /// Bytes needing sync
    pub need_bytes: u64,
    /// Pull errors
    pub pull_errors: u32,
}

impl FolderSummary {
    /// Check if folder is in sync
    pub fn is_synced(&self) -> bool {
        self.need_files == 0 && self.need_directories == 0 && self.need_bytes == 0
    }

    /// Calculate sync percentage
    pub fn sync_percent(&self) -> f64 {
        if self.bytes == 0 {
            return 100.0;
        }
        let synced = self.bytes - self.need_bytes;
        (synced as f64 / self.bytes as f64) * 100.0
    }
}

/// Event types for the event system
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Event {
    /// Folder state changed
    FolderSummary {
        /// Folder ID
        folder: FolderId,
        /// Summary data
        summary: FolderSummary,
    },
    /// File was downloaded
    ItemFinished {
        /// Folder ID
        folder: FolderId,
        /// Item name
        item: String,
        /// Error message if any
        error: Option<String>,
    },
    /// Device connected
    DeviceConnected {
        /// Device ID
        device: crate::DeviceId,
        /// Connection address
        addr: String,
    },
    /// Device disconnected
    DeviceDisconnected {
        /// Device ID
        device: crate::DeviceId,
        /// Error message if any
        error: Option<String>,
    },
    /// Local index updated
    LocalIndexUpdated {
        /// Folder ID
        folder: FolderId,
        /// Updated items
        items: Vec<String>,
    },
    /// Remote index received
    RemoteIndexUpdated {
        /// Device ID
        device: crate::DeviceId,
        /// Folder ID
        folder: FolderId,
        /// Number of items
        items_count: usize,
    },
}
