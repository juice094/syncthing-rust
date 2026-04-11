//! Module: syncthing-sync
//! Worker: Agent-C
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! 索引管理模块
//!
//! 该模块负责维护本地和远程文件索引，处理索引的接收、存储和合并。
//! 索引是同步决策的基础，记录了文件夹中所有文件的元数据。

use std::collections::{HashMap, HashSet};

use std::sync::Arc;


use tokio::sync::RwLock;
use tracing::{debug, info, trace};

use std::fmt;

use syncthing_core::traits::BlockStore;
use syncthing_core::types::{DeviceId, FileInfo, FolderId};
use syncthing_core::Result;

/// 索引管理器
///
/// 维护本地和远程设备的文件索引，提供索引查询和更新功能
pub struct IndexManager {
    /// 文件夹 ID
    folder_id: FolderId,
    /// 本地设备 ID
    local_device: DeviceId,
    /// 本地文件索引（从数据库加载）
    local_index: Arc<RwLock<HashMap<String, FileInfo>>>,
    /// 远程设备索引映射: device_id -> file_name -> FileInfo
    remote_indices: Arc<RwLock<HashMap<DeviceId, HashMap<String, FileInfo>>>>,
    /// 块存储引用
    block_store: Arc<dyn BlockStore>,
}

impl IndexManager {
    /// 创建新的索引管理器
    ///
    /// # 参数
    /// * `folder_id` - 文件夹 ID
    /// * `local_device` - 本地设备 ID
    /// * `block_store` - 块存储接口
    pub fn new(
        folder_id: FolderId,
        local_device: DeviceId,
        block_store: Arc<dyn BlockStore>,
    ) -> Self {
        Self {
            folder_id,
            local_device,
            local_index: Arc::new(RwLock::new(HashMap::new())),
            remote_indices: Arc::new(RwLock::new(HashMap::new())),
            block_store,
        }
    }

    /// 从数据库加载本地索引
    pub async fn load_local_index(&self) -> Result<()> {
        info!("加载本地索引: folder={}", self.folder_id);

        let files = self.block_store.get_index(&self.folder_id).await?;
        let mut index = self.local_index.write().await;

        for file in files {
            index.insert(file.name.clone(), file);
        }

        info!("本地索引加载完成: {} 个文件", index.len());
        Ok(())
    }

    /// 更新本地索引
    ///
    /// # 参数
    /// * `files` - 要更新的文件列表
    pub async fn update_local_index(&self, files: Vec<FileInfo>) -> Result<()> {
        debug!("更新本地索引: {} 个文件", files.len());

        // 更新内存中的索引
        {
            let mut index = self.local_index.write().await;
            for file in &files {
                index.insert(file.name.clone(), file.clone());
            }
        }

        // 持久化到数据库
        self.block_store
            .update_index_delta(&self.folder_id, files)
            .await?;

        Ok(())
    }

    /// 替换整个本地索引（用于完整扫描后）
    pub async fn replace_local_index(&self, files: Vec<FileInfo>) -> Result<()> {
        info!("替换本地索引: {} 个文件", files.len());

        // 替换内存中的索引
        {
            let mut index = self.local_index.write().await;
            index.clear();
            for file in &files {
                index.insert(file.name.clone(), file.clone());
            }
        }

        // 持久化到数据库
        self.block_store
            .update_index(&self.folder_id, files)
            .await?;

        Ok(())
    }

    /// 获取本地文件信息
    pub async fn get_local_file(&self, name: &str) -> Option<FileInfo> {
        let index = self.local_index.read().await;
        index.get(name).cloned()
    }

    /// 获取所有本地文件
    pub async fn get_all_local_files(&self) -> Vec<FileInfo> {
        let index = self.local_index.read().await;
        index.values().cloned().collect()
    }

    /// 接收远程索引（完整索引）
    ///
    /// # 参数
    /// * `device` - 远程设备 ID
    /// * `files` - 远程文件列表
    pub async fn receive_full_index(
        &self,
        device: DeviceId,
        files: Vec<FileInfo>,
    ) -> Result<()> {
        info!("接收完整索引: device={}, files={}", device.short_id(), files.len());

        let mut remote_indices = self.remote_indices.write().await;
        let mut index = HashMap::new();

        for file in files {
            trace!("远程索引文件: {} (size={}, version={:?})",
                file.name, file.size, file.version);
            index.insert(file.name.clone(), file);
        }

        remote_indices.insert(device, index);
        Ok(())
    }

    /// 接收远程索引更新（增量）
    ///
    /// # 参数
    /// * `device` - 远程设备 ID
    /// * `files` - 更新的文件列表
    pub async fn receive_index_update(
        &self,
        device: DeviceId,
        files: Vec<FileInfo>,
    ) -> Result<()> {
        debug!("接收索引更新: device={}, files={}", device.short_id(), files.len());

        let mut remote_indices = self.remote_indices.write().await;
        let index = remote_indices.entry(device).or_insert_with(HashMap::new);

        for file in files {
            trace!("更新远程索引: {} (version={:?})", file.name, file.version);
            index.insert(file.name.clone(), file);
        }

        Ok(())
    }

    /// 获取远程设备的文件信息
    pub async fn get_remote_file(
        &self,
        device: &DeviceId,
        name: &str,
    ) -> Option<FileInfo> {
        let remote_indices = self.remote_indices.read().await;
        remote_indices
            .get(device)
            .and_then(|index| index.get(name).cloned())
    }

    /// 获取远程设备的所有文件
    pub async fn get_remote_files(&self, device: &DeviceId) -> Vec<FileInfo> {
        let remote_indices = self.remote_indices.read().await;
        remote_indices
            .get(device)
            .map(|index| index.values().cloned().collect())
            .unwrap_or_default()
    }

    /// 计算需要同步的文件差异
    ///
    /// 比较本地索引和所有远程索引，找出需要从远程设备下载的文件
    ///
    /// # 返回
    /// 需要同步的文件列表，每个文件包含最佳来源设备
    pub async fn calculate_needed_files(&self) -> Vec<NeededFile> {
        let local_index = self.local_index.read().await;
        let remote_indices = self.remote_indices.read().await;

        let mut needed = Vec::new();
        let mut all_remote_files: HashSet<(DeviceId, String)> = HashSet::new();

        // 收集所有远程文件
        for (device, index) in remote_indices.iter() {
            for file_name in index.keys() {
                all_remote_files.insert((*device, file_name.clone()));
            }
        }

        // 检查每个远程文件是否需要同步
        for (device, file_name) in all_remote_files {
            if let Some(remote_file) = remote_indices
                .get(&device)
                .and_then(|idx| idx.get(&file_name))
            {
                let need_sync = match local_index.get(&file_name) {
                    None => {
                        // 本地没有此文件，需要下载
                        trace!("需要同步（新文件）: {}", file_name);
                        true
                    }
                    Some(local_file) => {
                        // 比较版本向量
                        use std::cmp::Ordering;
                        match remote_file.version.compare(&local_file.version) {
                            Some(Ordering::Greater) => {
                                trace!("需要同步（远程版本更新）: {}", file_name);
                                true
                            }
                            None => {
                                trace!("检测到冲突: {}", file_name);
                                true // 冲突也需要处理
                            }
                            _ => false,
                        }
                    }
                };

                if need_sync {
                    needed.push(NeededFile {
                        file_info: remote_file.clone(),
                        source_device: device,
                    });
                }
            }
        }

        info!("计算完成: {} 个文件需要同步", needed.len());
        needed
    }

    /// 查找拥有特定文件块的设备
    ///
    /// # 参数
    /// * `hash` - 块哈希
    ///
    /// # 返回
    /// 拥有该块的设备列表
    pub async fn find_devices_with_block(
        &self,
        hash: syncthing_core::types::BlockHash,
    ) -> Vec<DeviceId> {
        let remote_indices = self.remote_indices.read().await;
        let mut devices = Vec::new();

        for (device, index) in remote_indices.iter() {
            for file in index.values() {
                if file.blocks.iter().any(|b| b.hash == hash) {
                    devices.push(*device);
                    break;
                }
            }
        }

        devices
    }

    /// 获取文件夹统计信息
    pub async fn get_stats(&self) -> Result<IndexStats> {
        let local = self.local_index.read().await;
        let remote = self.remote_indices.read().await;

        let local_files = local.len() as u64;
        let local_bytes: u64 = local.values().map(|f| f.size).sum();

        let remote_devices = remote.len() as u32;

        Ok(IndexStats {
            local_files,
            local_bytes,
            remote_devices,
        })
    }

    /// 清理已断开连接的设备的索引
    pub async fn cleanup_device(&self, device: &DeviceId) {
        let mut remote_indices = self.remote_indices.write().await;
        if remote_indices.remove(device).is_some() {
            info!("清理设备索引: {}", device.short_id());
        }
    }
}

impl fmt::Debug for IndexManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IndexManager")
            .field("folder_id", &self.folder_id)
            .field("local_device", &self.local_device)
            .finish_non_exhaustive()
    }
}

/// 需要同步的文件
#[derive(Debug, Clone)]
pub struct NeededFile {
    /// 文件信息
    pub file_info: FileInfo,
    /// 来源设备
    pub source_device: DeviceId,
}

/// 索引统计信息
#[derive(Debug, Clone, Default)]
pub struct IndexStats {
    /// 本地文件数量
    pub local_files: u64,
    /// 本地总字节数
    pub local_bytes: u64,
    /// 已知的远程设备数量
    pub remote_devices: u32,
}

/// 索引差异比较器
pub struct IndexDiffer;

impl IndexDiffer {
    /// 比较两个索引，找出差异
    ///
    /// # 参数
    /// * `local` - 本地索引
    /// * `remote` - 远程索引
    ///
    /// # 返回
    /// 索引差异结果
    pub fn compare(
        local: &HashMap<String, FileInfo>,
        remote: &HashMap<String, FileInfo>,
    ) -> IndexDiff {
        let mut added = Vec::new();
        let mut modified = Vec::new();
        let mut deleted = Vec::new();
        let mut unchanged = Vec::new();

        // 检查远程文件（新增、修改）
        for (name, remote_file) in remote {
            match local.get(name) {
                None => added.push(name.clone()),
                Some(local_file) => {
                    use std::cmp::Ordering;
                    match remote_file.version.compare(&local_file.version) {
                        Some(Ordering::Greater) => modified.push(name.clone()),
                        Some(Ordering::Equal) => unchanged.push(name.clone()),
                        _ => modified.push(name.clone()), // 冲突也视为需要处理
                    }
                }
            }
        }

        // 检查本地文件（删除）
        for name in local.keys() {
            if !remote.contains_key(name) {
                deleted.push(name.clone());
            }
        }

        IndexDiff {
            added,
            modified,
            deleted,
            unchanged,
        }
    }
}

/// 索引差异结果
#[derive(Debug, Clone, Default)]
pub struct IndexDiff {
    /// 新增的文件
    pub added: Vec<String>,
    /// 修改的文件
    pub modified: Vec<String>,
    /// 删除的文件（在远程不存在）
    pub deleted: Vec<String>,
    /// 未变化的文件
    pub unchanged: Vec<String>,
}

impl IndexDiff {
    /// 检查是否有任何变化
    pub fn has_changes(&self) -> bool {
        !self.added.is_empty() || !self.modified.is_empty() || !self.deleted.is_empty()
    }

    /// 获取变化文件总数
    pub fn total_changes(&self) -> usize {
        self.added.len() + self.modified.len() + self.deleted.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncthing_core::types::{DeviceId, FileInfo};
    use syncthing_core::version_vector::VersionVector;

    fn create_test_device(id: u8) -> DeviceId {
        let mut bytes = [0u8; 32];
        bytes[0] = id;
        DeviceId::from_bytes(bytes)
    }

    fn create_test_file(name: &str, size: u64, version_counter: u64) -> FileInfo {
        let device = create_test_device(1);
        let mut version = VersionVector::new();
        for _ in 0..version_counter {
            version.increment(device);
        }

        FileInfo {
            name: name.to_string(),
            size,
            modified: std::time::SystemTime::now(),
            version,
            blocks: vec![],
            permissions: 0o644,
            is_directory: false,
            is_symlink: false,
            symlink_target: None,
            sequence: version_counter,
        }
    }

    #[test]
    fn test_index_differ_added() {
        let local = HashMap::new();
        let mut remote = HashMap::new();

        remote.insert("new_file.txt".to_string(), create_test_file("new_file.txt", 100, 1));

        let diff = IndexDiffer::compare(&local, &remote);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0], "new_file.txt");
        assert!(diff.modified.is_empty());
        assert!(diff.deleted.is_empty());
    }

    #[test]
    fn test_index_differ_modified() {
        let mut local = HashMap::new();
        let mut remote = HashMap::new();

        local.insert("file.txt".to_string(), create_test_file("file.txt", 100, 1));
        remote.insert("file.txt".to_string(), create_test_file("file.txt", 200, 2));

        let diff = IndexDiffer::compare(&local, &remote);
        assert!(diff.added.is_empty());
        assert_eq!(diff.modified.len(), 1);
        assert_eq!(diff.modified[0], "file.txt");
        assert!(diff.deleted.is_empty());
    }

    #[test]
    fn test_index_differ_deleted() {
        let mut local = HashMap::new();
        let remote = HashMap::new();

        local.insert("old_file.txt".to_string(), create_test_file("old_file.txt", 100, 1));

        let diff = IndexDiffer::compare(&local, &remote);
        assert!(diff.added.is_empty());
        assert!(diff.modified.is_empty());
        assert_eq!(diff.deleted.len(), 1);
        assert_eq!(diff.deleted[0], "old_file.txt");
    }

    #[test]
    fn test_index_differ_unchanged() {
        let mut local = HashMap::new();
        let mut remote = HashMap::new();

        let file = create_test_file("file.txt", 100, 1);
        local.insert("file.txt".to_string(), file.clone());
        remote.insert("file.txt".to_string(), file);

        let diff = IndexDiffer::compare(&local, &remote);
        assert!(diff.added.is_empty());
        assert!(diff.modified.is_empty());
        assert!(diff.deleted.is_empty());
        assert_eq!(diff.unchanged.len(), 1);
    }

    #[test]
    fn test_index_diff_has_changes() {
        let diff = IndexDiff {
            added: vec!["a.txt".to_string()],
            modified: vec![],
            deleted: vec![],
            unchanged: vec![],
        };
        assert!(diff.has_changes());

        let diff_no_changes = IndexDiff {
            added: vec![],
            modified: vec![],
            deleted: vec![],
            unchanged: vec!["a.txt".to_string()],
        };
        assert!(!diff_no_changes.has_changes());
    }
}
