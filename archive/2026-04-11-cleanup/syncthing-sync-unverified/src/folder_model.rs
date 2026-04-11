//! Module: syncthing-sync
//! Worker: Agent-Sync
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! FolderModel 实现 - 文件夹级别的 Model trait 实现
//!
//! 该模块实现 FolderModel 结构体，为单个文件夹提供完整的 BEP 协议消息处理能力。

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, info, trace, warn};

use syncthing_core::traits::{BlockStore, FileSystem};
use syncthing_core::types::{BlockHash, DeviceId, FileInfo, FolderId};
use syncthing_core::Result;

use crate::conflict::{ConflictManager, ConflictResolver};
use crate::index::IndexManager;
use crate::model_trait::{
    ClusterConfig, DownloadProgress, FolderState, RemoteDeviceState,
};
use crate::model_trait::Model;
use crate::puller::PullTask;

/// 文件夹模型
///
/// 为单个文件夹实现 Model trait，处理所有 BEP 协议消息
pub struct FolderModel {
    /// 文件夹 ID
    folder_id: FolderId,
    /// 文件夹本地路径
    folder_path: PathBuf,
    /// 文件系统抽象
    filesystem: Arc<dyn FileSystem>,
    /// 块存储
    block_store: Arc<dyn BlockStore>,
    /// 本地设备 ID
    #[allow(dead_code)]
    local_device: DeviceId,
    /// 索引管理器
    index_manager: Arc<IndexManager>,
    /// 冲突解决器
    conflict_resolver: ConflictResolver,
    /// 冲突管理器
    conflict_manager: Arc<Mutex<ConflictManager>>,
    /// 远程设备状态
    remote_devices: RwLock<HashMap<DeviceId, RemoteDeviceState>>,
    /// 文件夹状态
    state: RwLock<FolderState>,
    /// 拉取任务队列发送端
    pull_queue_tx: mpsc::Sender<PullTask>,
}

impl fmt::Debug for FolderModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FolderModel")
            .field("folder_id", &self.folder_id)
            .field("folder_path", &self.folder_path)
            .finish_non_exhaustive()
    }
}

impl FolderModel {
    /// 创建新的文件夹模型
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        folder_id: FolderId,
        folder_path: PathBuf,
        filesystem: Arc<dyn FileSystem>,
        block_store: Arc<dyn BlockStore>,
        local_device: DeviceId,
        pull_queue_tx: mpsc::Sender<PullTask>,
    ) -> Result<Self> {
        let index_manager = Arc::new(IndexManager::new(
            folder_id.clone(),
            local_device,
            block_store.clone(),
        ));

        Ok(Self {
            folder_id,
            folder_path,
            filesystem,
            block_store,
            local_device,
            index_manager,
            conflict_resolver: ConflictResolver::new(local_device),
            conflict_manager: Arc::new(Mutex::new(ConflictManager::new())),
            remote_devices: RwLock::new(HashMap::new()),
            state: RwLock::new(FolderState::Idle),
            pull_queue_tx,
        })
    }

    /// 获取文件夹 ID
    pub fn folder_id(&self) -> &FolderId {
        &self.folder_id
    }

    /// 获取文件夹路径
    pub fn folder_path(&self) -> &PathBuf {
        &self.folder_path
    }

    /// 获取文件系统
    pub fn filesystem(&self) -> Arc<dyn FileSystem> {
        self.filesystem.clone()
    }

    /// 获取块存储
    pub fn block_store(&self) -> Arc<dyn BlockStore> {
        self.block_store.clone()
    }

    /// 获取当前状态
    pub async fn state(&self) -> FolderState {
        self.state.read().await.clone()
    }

    /// 设置状态
    pub async fn set_state(&self, state: FolderState) {
        let mut current = self.state.write().await;
        *current = state;
    }

    /// 更新同步状态
    pub async fn update_syncing_state(&self, syncing: bool) {
        let mut state = self.state.write().await;
        *state = if syncing {
            FolderState::Syncing { total: 0, done: 0 }
        } else {
            FolderState::Idle
        };
    }

    /// 加载本地索引
    pub async fn load_local_index(&self) -> Result<()> {
        self.index_manager.load_local_index().await
    }

    /// 扫描本地文件夹
    pub async fn scan_local(&self) -> Result<Vec<FileInfo>> {
        self.set_state(FolderState::Scanning).await;

        let files = self.filesystem.scan_directory(&self.folder_path).await?;

        // 更新本地索引
        self.index_manager.replace_local_index(files.clone()).await?;

        self.set_state(FolderState::Idle).await;

        info!("扫描完成: {} 个文件", files.len());
        Ok(files)
    }

    /// 注册远程设备
    pub async fn register_device(&self, device: DeviceId) {
        let mut devices = self.remote_devices.write().await;
        devices.insert(device, RemoteDeviceState::new());
        info!("注册远程设备: device={}", device.short_id());
    }

    /// 注销远程设备
    pub async fn unregister_device(&self, device: &DeviceId) {
        let mut devices = self.remote_devices.write().await;
        if devices.remove(device).is_some() {
            info!("注销远程设备: device={}", device.short_id());
        }
        // 清理索引
        self.index_manager.cleanup_device(device).await;
    }

    /// 处理远程索引
    ///
    /// 比较远程索引与本地索引，确定需要拉取的文件
    async fn handle_remote_index(
        &self,
        device: DeviceId,
        remote_files: Vec<FileInfo>,
    ) -> Result<()> {
        debug!(
            "处理远程索引: folder={}, device={}, files={}",
            self.folder_id,
            device.short_id(),
            remote_files.len()
        );

        // 更新远程设备状态
        {
            let mut devices = self.remote_devices.write().await;
            let state = devices.entry(device).or_insert_with(RemoteDeviceState::new);
            state.update_files(remote_files.clone());
        }

        // 通知索引管理器
        self.index_manager
            .receive_full_index(device, remote_files.clone())
            .await?;

        // 对比索引，找出需要拉取的文件
        for remote_file in remote_files {
            self.process_remote_file(device, remote_file).await?;
        }

        Ok(())
    }

    /// 处理远程文件
    async fn process_remote_file(&self, device: DeviceId, remote_file: FileInfo) -> Result<()> {
        let local_file = self.index_manager.get_local_file(&remote_file.name).await;

        match local_file {
            None => {
                // 本地没有，需要拉取
                trace!("本地没有文件，需要拉取: {}", remote_file.name);
                self.queue_for_pull(device, remote_file).await?;
            }
            Some(local) => {
                // 对比版本向量
                use std::cmp::Ordering;
                match local.version.compare(&remote_file.version) {
                    Some(Ordering::Less) => {
                        // 远程更新，需要拉取
                        trace!("远程版本更新，需要拉取: {}", remote_file.name);
                        self.queue_for_pull(device, remote_file).await?;
                    }
                    None => {
                        // 冲突！
                        warn!("检测到冲突: {}", remote_file.name);
                        self.handle_conflict(local, remote_file, device).await?;
                    }
                    _ => {
                        // 本地更新或相同，忽略
                        trace!("本地版本更新或相同，忽略: {}", remote_file.name);
                    }
                }
            }
        }

        Ok(())
    }

    /// 将文件加入拉取队列
    async fn queue_for_pull(&self, device: DeviceId, file: FileInfo) -> Result<()> {
        let task = PullTask::new(file, device, 0);

        self.pull_queue_tx
            .send(task)
            .await
            .map_err(|e| syncthing_core::SyncthingError::Internal(format!("队列已满: {}", e)))?;

        Ok(())
    }

    /// 处理冲突
    async fn handle_conflict(
        &self,
        local: FileInfo,
        remote: FileInfo,
        remote_device: DeviceId,
    ) -> Result<()> {
        let resolution = self
            .conflict_resolver
            .resolve(Some(&local), &remote);

        match resolution {
            Some(crate::conflict::ConflictResolution::RemoteWins) => {
                // 远程胜出，拉取远程版本
                info!("冲突解决: 远程胜出 {}", remote.name);
                self.queue_for_pull(remote_device, remote).await?;
            }
            Some(crate::conflict::ConflictResolution::LocalWins) => {
                // 本地胜出，无需操作
                info!("冲突解决: 本地胜出 {}", local.name);
            }
            Some(crate::conflict::ConflictResolution::Conflict { conflict_copy_path }) => {
                // 需要创建冲突副本
                warn!("检测到冲突: {} -> {}", remote.name, conflict_copy_path);

                // 记录冲突
                let conflict_info = crate::conflict::ConflictInfo {
                    folder: self.folder_id.clone(),
                    original_name: local.name.clone(),
                    conflict_name: conflict_copy_path,
                    local_version: local.version.clone(),
                    remote_version: remote.version.clone(),
                    conflict_time: std::time::SystemTime::now(),
                };

                let mut manager = self.conflict_manager.lock().await;
                manager.record_conflict(conflict_info);

                // 使用 last writer wins 策略
                if self.conflict_resolver.last_writer_wins(&local, &remote, remote_device) {
                    info!("Last writer wins: 本地胜出 {}", local.name);
                } else {
                    info!("Last writer wins: 远程胜出 {}", remote.name);
                    self.queue_for_pull(remote_device, remote).await?;
                }
            }
            None => {
                // 无需同步
            }
        }

        Ok(())
    }

    /// 从文件系统读取块
    async fn read_block_from_filesystem(
        &self,
        hash: BlockHash,
    ) -> Result<Vec<u8>> {
        // 获取所有本地文件
        let local_files = self.index_manager.get_all_local_files().await;

        // 查找包含此块的文件
        for file_info in local_files {
            for block in &file_info.blocks {
                if block.hash == hash {
                    let file_path = self.folder_path.join(&file_info.name);
                    trace!("在文件 {} 中找到块", file_info.name);

                    // 从文件读取块数据
                    return self
                        .filesystem
                        .read_block(&file_path, block.offset, block.size)
                        .await;
                }
            }
        }

        // 块未找到
        Err(syncthing_core::SyncthingError::BlockNotFound(hash))
    }

    /// 获取索引管理器
    pub fn index_manager(&self) -> Arc<IndexManager> {
        self.index_manager.clone()
    }

    /// 获取所有远程设备
    pub async fn remote_devices(&self) -> Vec<DeviceId> {
        let devices = self.remote_devices.read().await;
        devices.keys().cloned().collect()
    }

    /// 获取远程设备状态
    pub async fn get_remote_device_state(&self, device: &DeviceId) -> Option<RemoteDeviceState> {
        let devices = self.remote_devices.read().await;
        devices.get(device).cloned()
    }

    /// 获取远程文件信息
    pub async fn get_remote_file(
        &self,
        device: &DeviceId,
        name: &str,
    ) -> Option<FileInfo> {
        let devices = self.remote_devices.read().await;
        devices
            .get(device)
            .and_then(|state| state.get_file(name).cloned())
    }

    /// 更新下载进度
    pub async fn update_download_progress(&self, device: DeviceId, progress: &DownloadProgress) {
        let mut devices = self.remote_devices.write().await;
        if let Some(state) = devices.get_mut(&device) {
            state.update_progress(progress.clone());
        }
    }
}

#[async_trait]
impl Model for FolderModel {
    async fn index(&self, device: DeviceId, folder: &str, files: Vec<FileInfo>) -> Result<()> {
        // 验证文件夹 ID
        if folder != self.folder_id.as_str() {
            return Err(syncthing_core::SyncthingError::FolderNotFound(
                FolderId::new(folder),
            ));
        }

        info!(
            "收到完整索引: folder={}, device={}, files={}",
            folder,
            device.short_id(),
            files.len()
        );

        // 确保设备已注册
        self.register_device(device).await;

        // 处理远程索引
        self.handle_remote_index(device, files).await
    }

    async fn index_update(&self, device: DeviceId, folder: &str, files: Vec<FileInfo>) -> Result<()> {
        // 验证文件夹 ID
        if folder != self.folder_id.as_str() {
            return Err(syncthing_core::SyncthingError::FolderNotFound(
                FolderId::new(folder),
            ));
        }

        debug!(
            "收到索引更新: folder={}, device={}, files={}",
            folder,
            device.short_id(),
            files.len()
        );

        // 更新索引管理器
        self.index_manager
            .receive_index_update(device, files.clone())
            .await?;

        // 更新远程设备状态
        {
            let mut devices = self.remote_devices.write().await;
            let state = devices.entry(device).or_insert_with(RemoteDeviceState::new);
            state.update_files(files.clone());
        }

        // 处理每个更新的文件
        for file in files {
            self.process_remote_file(device, file).await?;
        }

        Ok(())
    }

    async fn request(
        &self,
        folder: &str,
        name: &str,
        offset: i64,
        size: i32,
        hash: &[u8],
    ) -> Result<Vec<u8>> {
        // 验证文件夹 ID
        if folder != self.folder_id.as_str() {
            return Err(syncthing_core::SyncthingError::FolderNotFound(
                FolderId::new(folder),
            ));
        }

        const MAX_BLOCK_SIZE: usize = 16 * 1024 * 1024;
        if size < 0 || size as usize > MAX_BLOCK_SIZE {
            return Err(syncthing_core::SyncthingError::Protocol(format!(
                "Invalid block size: {}, max allowed: {}",
                size, MAX_BLOCK_SIZE
            )));
        }

        trace!(
            "收到块请求: folder={}, name={}, offset={}, size={}",
            folder, name, offset, size
        );

        // 转换 hash
        let hash_bytes: [u8; 32] = hash.try_into()
            .map_err(|_| syncthing_core::SyncthingError::Protocol("Invalid hash length".to_string()))?;
        let block_hash = BlockHash::from_bytes(hash_bytes);

        // 首先尝试从块存储获取
        match self.block_store.get(block_hash).await? {
            Some(data) => {
                trace!("从块存储找到块");
                Ok(data)
            }
            None => {
                // 从文件系统读取
                trace!("从文件系统读取块");
                self.read_block_from_filesystem(block_hash).await
            }
        }
    }

    async fn cluster_config(&self, device: DeviceId, config: &ClusterConfig) -> Result<()> {
        info!(
            "收到集群配置: device={}, devices={}, folders={}",
            device.short_id(),
            config.devices.len(),
            config.folders.len()
        );

        // 注册设备
        self.register_device(device).await;

        // 更新远程设备信息
        {
            let mut devices = self.remote_devices.write().await;
            for dev_info in &config.devices {
                if !devices.contains_key(&dev_info.id) {
                    devices.insert(dev_info.id, RemoteDeviceState::new());
                }
            }
        }

        debug!("集群配置处理完成");
        Ok(())
    }

    async fn closed(&self, device: DeviceId, err: Option<&syncthing_core::SyncthingError>) {
        if let Some(e) = err {
            warn!("连接关闭: device={}, error={}", device.short_id(), e);
        } else {
            info!("连接正常关闭: device={}", device.short_id());
        }

        // 清理设备
        self.unregister_device(&device).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncthing_core::types::DeviceId;

    #[allow(dead_code)]
    fn create_test_device(id: u8) -> DeviceId {
        let mut bytes = [0u8; 32];
        bytes[0] = id;
        DeviceId::from_bytes(bytes)
    }

    // 注意：这些测试需要 mock 对象，完整测试需要设置 FileSystem 和 BlockStore
    #[test]
    fn test_folder_model_creation() {
        // 测试结构创建
        // 实际使用需要真实的 filesystem 和 block_store
    }

    #[tokio::test]
    async fn test_folder_state_management() {
        // 测试状态管理
        let (_tx, _rx): (mpsc::Sender<crate::puller::PullTask>, _) = mpsc::channel(100);
        
        // 需要 mock 对象来创建 FolderModel
        // 这里只是演示测试结构
    }
}
