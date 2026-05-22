//! 核心类型定义

use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;

mod connection;
pub use connection::*;

mod folder;
pub use folder::*;

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
        let all_devices: std::collections::HashSet<_> =
            self.counters.keys().chain(other.counters.keys()).collect();

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
        let all_devices: std::collections::HashSet<_> =
            self.counters.keys().chain(other.counters.keys()).collect();

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
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
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
    /// 最后修改者设备短 ID（BEP 兼容字段）
    pub modified_by: Option<u64>,
    /// 块列表哈希（用于快速比较块变化）
    pub blocks_hash: Option<Vec<u8>>,
    /// 无权限标记（BEP 兼容字段）
    pub no_permissions: Option<bool>,
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
            modified_by: None,
            blocks_hash: None,
            no_permissions: None,
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
    /// 启用的传输层列表，如 ["tcp", "websocket", "relay"]
    #[serde(default = "default_transports")]
    pub transports: Vec<String>,
}

/// 版本控制配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[derive(Default)]
pub enum VersioningConfig {
    #[default]
    None,
    Simple {
        params: HashMap<String, String>,
    },
    Staggered {
        params: HashMap<String, String>,
    },
    External {
        params: HashMap<String, String>,
    },
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

fn default_listen() -> String {
    crate::constants::DEFAULT_LISTEN_ADDR.to_string()
}
fn default_transports() -> Vec<String> {
    vec!["tcp".to_string()]
}
fn default_device_name() -> String {
    "syncthing-rust".to_string()
}

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

    /// Convert to Vec\<u8\>
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

#[cfg(test)]
mod tests;
