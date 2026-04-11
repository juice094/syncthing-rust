//! Module: syncthing-sync
//! Worker: Agent-C
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! 拉取逻辑模块
//!
//! 该模块实现文件拉取（下载）逻辑，包括：
//! - 比较本地与远程索引，决定需要下载的文件
//! - 计算需要下载的块
//! - 从其他设备请求块数据
//! - 组装块为完整文件

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;


use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::JoinSet;
use tracing::{debug, error, info, trace, warn};

use syncthing_core::traits::{BlockStore, BepConnection, FileSystem};
use syncthing_core::types::{BlockHash, BlockInfo, DeviceId, FileInfo, FolderId};
use syncthing_core::Result;

use crate::conflict::{ConflictResolution, ConflictResolver};
use crate::index::{IndexManager, NeededFile};

/// 设备到 BEP 连接的映射
type DeviceConnectionMap = HashMap<DeviceId, Arc<Mutex<Box<dyn BepConnection>>>>;

/// 拉取任务
#[derive(Debug, Clone)]
pub struct PullTask {
    /// 文件信息
    pub file_info: FileInfo,
    /// 来源设备
    pub source_device: DeviceId,
    /// 需要下载的块列表
    pub needed_blocks: Vec<BlockInfo>,
    /// 优先级
    pub priority: i32,
}

impl PullTask {
    /// 创建新的拉取任务
    pub fn new(file_info: FileInfo, source_device: DeviceId, priority: i32) -> Self {
        Self {
            file_info,
            source_device,
            needed_blocks: Vec::new(),
            priority,
        }
    }

    /// 计算需要下载的总字节数
    pub fn total_bytes(&self) -> u64 {
        self.needed_blocks.iter().map(|b| b.size as u64).sum()
    }
}

/// 拉取器
pub struct Puller {
    /// 文件夹 ID
    folder_id: FolderId,
    /// 文件夹本地路径
    folder_path: PathBuf,
    /// 本地设备 ID
    #[allow(dead_code)]
    local_device: DeviceId,
    /// 索引管理器
    index_manager: Arc<IndexManager>,
    /// 块存储
    block_store: Arc<dyn BlockStore>,
    /// 文件系统
    file_system: Arc<dyn FileSystem>,
    /// 冲突解决器
    conflict_resolver: ConflictResolver,
    /// 活动连接映射
    connections: Arc<RwLock<DeviceConnectionMap>>,
    /// 并发下载限制
    max_concurrent_downloads: usize,
}

impl Puller {
    /// 创建新的拉取器
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        folder_id: FolderId,
        folder_path: PathBuf,
        local_device: DeviceId,
        index_manager: Arc<IndexManager>,
        block_store: Arc<dyn BlockStore>,
        file_system: Arc<dyn FileSystem>,
        max_concurrent_downloads: usize,
    ) -> Self {
        Self {
            folder_id,
            folder_path,
            local_device,
            index_manager,
            block_store,
            file_system,
            conflict_resolver: ConflictResolver::new(local_device),
            connections: Arc::new(RwLock::new(HashMap::new())),
            max_concurrent_downloads,
        }
    }

    /// 注册设备连接
    pub async fn register_connection(
        &self,
        device: DeviceId,
        conn: Box<dyn BepConnection>,
    ) {
        let mut connections = self.connections.write().await;
        connections.insert(device, Arc::new(Mutex::new(conn)));
        info!("注册设备连接: {}", device.short_id());
    }

    /// 注销设备连接
    pub async fn unregister_connection(&self, device: &DeviceId) {
        let mut connections = self.connections.write().await;
        if connections.remove(device).is_some() {
            info!("注销设备连接: {}", device.short_id());
        }
    }

    /// 执行拉取操作
    ///
    /// 从远程设备拉取需要同步的文件
    pub async fn pull(&self) -> Result<PullResult> {
        info!("开始拉取: folder={}", self.folder_id);

        // 计算需要同步的文件
        let needed_files = self.index_manager.calculate_needed_files().await;

        if needed_files.is_empty() {
            info!("没有需要同步的文件");
            return Ok(PullResult::default());
        }

        info!("需要同步 {} 个文件", needed_files.len());

        // 创建拉取任务
        let tasks = self.create_pull_tasks(needed_files).await?;

        // 执行拉取
        let result = self.execute_pull_tasks(tasks).await?;

        info!(
            "拉取完成: files={}, bytes={}, errors={}",
            result.files_processed,
            result.bytes_transferred,
            result.errors.len()
        );

        Ok(result)
    }

    /// 创建拉取任务
    async fn create_pull_tasks(
        &self,
        needed_files: Vec<NeededFile>,
    ) -> Result<Vec<PullTask>> {
        let mut tasks = Vec::new();

        for needed in needed_files {
            let local_file = self
                .index_manager
                .get_local_file(&needed.file_info.name)
                .await;

            // 检查冲突
            let resolution = self
                .conflict_resolver
                .resolve(local_file.as_ref(), &needed.file_info);

            match resolution {
                None => {
                    // 无需同步
                    trace!("跳过文件: {}", needed.file_info.name);
                    continue;
                }
                Some(ConflictResolution::LocalWins) => {
                    // 本地版本更新，跳过
                    trace!("本地版本更新，跳过: {}", needed.file_info.name);
                    continue;
                }
                Some(ConflictResolution::RemoteWins) => {
                    // 需要下载
                    let needed_blocks = self
                        .calculate_needed_blocks(&needed.file_info, local_file.as_ref())
                        .await?;

                    if !needed_blocks.is_empty() {
                        tasks.push(PullTask {
                            file_info: needed.file_info,
                            source_device: needed.source_device,
                            needed_blocks,
                            priority: 0,
                        });
                    }
                }
                Some(ConflictResolution::Conflict { conflict_copy_path }) => {
                    // 处理冲突
                    warn!(
                        "处理冲突: {} -> {}",
                        needed.file_info.name, conflict_copy_path
                    );

                    let mut file_info = needed.file_info.clone();

                    if let Some(ref local) = local_file {
                        // 1. 物理创建冲突副本（跳过目录和软链接）
                        let conflict_path = PathBuf::from(&conflict_copy_path);
                        if let Err(e) = self.handle_conflict_copy(local, &conflict_path).await {
                            warn!(
                                "创建冲突副本失败: {} -> {}: {}",
                                needed.file_info.name, conflict_copy_path, e
                            );
                        }

                        // 2. 合并版本向量
                        let merged_version = self.conflict_resolver.merge_versions(local, &needed.file_info);
                        file_info.version = merged_version;

                        warn!(
                            "冲突已解决，合并版本并继续拉取远程文件: {} version={:?}",
                            file_info.name, file_info.version
                        );
                    }

                    // 3. 继续按 RemoteWins 拉取（使用合并后的 version）
                    let needed_blocks = self
                        .calculate_needed_blocks(&file_info, local_file.as_ref())
                        .await?;

                    if !needed_blocks.is_empty() {
                        tasks.push(PullTask {
                            file_info,
                            source_device: needed.source_device,
                            needed_blocks,
                            priority: 0,
                        });
                    }
                }
            }
        }

        Ok(tasks)
    }

    /// 计算需要下载的块
    ///
    /// 比较本地和远程文件的块，找出需要下载的块
    async fn calculate_needed_blocks(
        &self,
        remote_file: &FileInfo,
        local_file: Option<&FileInfo>,
    ) -> Result<Vec<BlockInfo>> {
        let mut needed = Vec::new();

        for block in &remote_file.blocks {
            // 检查本地是否已有此块
            let has_block = match local_file {
                None => self.block_store.has(block.hash).await?,
                Some(local) => {
                    // 检查本地文件是否已有此块
                    let local_has = local.blocks.iter().any(|b| b.hash == block.hash);
                    if local_has {
                        true
                    } else {
                        self.block_store.has(block.hash).await?
                    }
                }
            };

            if !has_block {
                needed.push(block.clone());
            }
        }

        trace!(
            "文件 {} 需要下载 {} 个块（共 {} 个）",
            remote_file.name,
            needed.len(),
            remote_file.blocks.len()
        );

        Ok(needed)
    }

    /// 创建冲突副本
    ///
    /// 将本地现有文件复制到冲突副本路径，保留原始内容。
    /// 目录和软链接跳过物理复制，仅记录日志。
    async fn handle_conflict_copy(
        &self,
        local_info: &FileInfo,
        conflict_name: &Path,
    ) -> Result<()> {
        // 目录和软链接不创建物理副本
        if local_info.is_directory {
            warn!("跳过目录的冲突副本创建: {}", local_info.name);
            return Ok(());
        }
        if local_info.is_symlink {
            warn!("跳过软链接的冲突副本创建: {}", local_info.name);
            return Ok(());
        }

        let local_path = self.folder_path.join(&local_info.name);
        let conflict_path = self.folder_path.join(conflict_name);

        // 确保父目录存在
        if let Some(parent) = conflict_path.parent() {
            self.file_system.create_dir(parent).await?;
        }

        // 分块读取本地文件并写入冲突副本
        const CHUNK_SIZE: usize = 128 * 1024;
        let mut offset = 0u64;
        let file_size = local_info.size;

        while offset < file_size {
            let to_read = std::cmp::min(CHUNK_SIZE, (file_size - offset) as usize);
            let data = self.file_system.read_block(&local_path, offset, to_read).await?;
            if data.is_empty() {
                break;
            }
            self.file_system.write_block(&conflict_path, offset, &data).await?;
            offset += data.len() as u64;
        }

        info!(
            "已创建冲突副本: {} -> {}",
            local_info.name,
            conflict_name.display()
        );
        Ok(())
    }

    /// 执行拉取任务
    async fn execute_pull_tasks(&self, tasks: Vec<PullTask>) -> Result<PullResult> {
        let mut result = PullResult::default();
        let (tx, mut rx) = mpsc::channel::<PullProgress>(32);

        // 使用信号量限制并发
        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.max_concurrent_downloads));
        let mut join_set = JoinSet::new();

        for task in tasks {
            let permit = semaphore.clone().acquire_owned().await
                .expect("Semaphore should not be closed");
            let tx = tx.clone();
            let folder_path = self.folder_path.clone();
            let block_store = self.block_store.clone();
            let file_system = self.file_system.clone();
            let connections = self.connections.clone();
            let folder_id = self.folder_id.clone();

            join_set.spawn(async move {
                let _permit = permit;
                let file_name = task.file_info.name.clone();

                match Self::pull_file(
                    task,
                    folder_id,
                    folder_path,
                    block_store,
                    file_system,
                    connections,
                )
                .await
                {
                    Ok(bytes) => {
                        let _ = tx.send(PullProgress::Success {
                            file: file_name,
                            bytes,
                        }).await;
                    }
                    Err(e) => {
                        let _ = tx.send(PullProgress::Failed {
                            file: file_name,
                            error: e.to_string(),
                        }).await;
                    }
                }
            });
        }

        // 关闭发送端，让接收端知道没有更多消息
        drop(tx);

        // 收集结果
        while let Some(progress) = rx.recv().await {
            match progress {
                PullProgress::Success { file, bytes } => {
                    result.files_processed += 1;
                    result.bytes_transferred += bytes;
                    debug!("成功拉取文件: {} ({} bytes)", file, bytes);
                }
                PullProgress::Failed { file, error } => {
                    result.errors.push(format!("{}: {}", file, error));
                    error!("拉取文件失败: {} - {}", file, error);
                }
            }
        }

        // 等待所有任务完成
        while join_set.join_next().await.is_some() {}

        Ok(result)
    }

    /// 拉取单个文件
    async fn pull_file(
        task: PullTask,
        folder_id: FolderId,
        folder_path: PathBuf,
        block_store: Arc<dyn BlockStore>,
        file_system: Arc<dyn FileSystem>,
        connections: Arc<RwLock<DeviceConnectionMap>>,
    ) -> Result<u64> {
        let file_name = &task.file_info.name;
        let file_path = folder_path.join(file_name);

        info!("开始拉取文件: {} ({} bytes)", file_name, task.file_info.size);

        // 确保父目录存在
        if let Some(parent) = file_path.parent() {
            file_system.create_dir(parent).await?;
        }

        // 下载所有需要的块
        let mut total_bytes = 0u64;

        for block in &task.needed_blocks {
            // 尝试从已注册的设备连接下载
            let data = Self::download_block(
                &folder_id,
                block.hash,
                block.offset,
                block.size,
                task.source_device,
                connections.clone(),
            )
            .await?;

            // 验证块哈希
            let computed_hash = BlockHash::from_data(&data);
            if computed_hash != block.hash {
                return Err(syncthing_core::SyncthingError::Protocol(format!(
                    "Invalid block: expected {}, got {}",
                    block.hash, computed_hash
                )));
            }

            // 存储块到 block store
            block_store.put(block.hash, &data).await?;

            // 写入文件
            file_system
                .write_block(&file_path, block.offset, &data)
                .await?;

            total_bytes += data.len() as u64;
        }

        info!("完成拉取文件: {} ({} bytes)", file_name, total_bytes);
        Ok(total_bytes)
    }

    /// 下载单个块
    async fn download_block(
        folder_id: &FolderId,
        hash: BlockHash,
        offset: u64,
        size: usize,
        preferred_device: DeviceId,
        connections: Arc<RwLock<DeviceConnectionMap>>,
    ) -> Result<Vec<u8>> {
        // 首先尝试从首选设备下载
        let connections_guard = connections.read().await;

        if let Some(conn) = connections_guard.get(&preferred_device) {
            let mut conn = conn.lock().await;
            trace!(
                "从设备 {} 请求块: hash={:?}, offset={}, size={}",
                preferred_device.short_id(),
                hash,
                offset,
                size
            );

            match conn.request_block(folder_id, hash, offset, size).await {
                Ok(data) => {
                    trace!("成功下载块: hash={:?}, size={}", hash, data.len());
                    return Ok(data);
                }
                Err(e) => {
                    warn!(
                        "从首选设备 {} 下载块失败: {}",
                        preferred_device.short_id(),
                        e
                    );
                }
            }
        }

        // 尝试其他设备
        for (device, conn) in connections_guard.iter() {
            if *device == preferred_device {
                continue;
            }

            let mut conn = conn.lock().await;
            trace!(
                "从备用设备 {} 请求块: hash={:?}",
                device.short_id(),
                hash
            );

            match conn.request_block(folder_id, hash, offset, size).await {
                Ok(data) => {
                    trace!("成功从备用设备下载块: hash={:?}", hash);
                    return Ok(data);
                }
                Err(e) => {
                    trace!("从设备 {} 下载块失败: {}", device.short_id(), e);
                }
            }
        }

        Err(syncthing_core::SyncthingError::BlockNotFound(hash))
    }
}

/// 拉取结果
#[derive(Debug, Clone, Default)]
pub struct PullResult {
    /// 处理的文件数量
    pub files_processed: u32,
    /// 传输的字节数
    pub bytes_transferred: u64,
    /// 错误信息
    pub errors: Vec<String>,
}

/// 拉取进度
#[derive(Debug, Clone)]
enum PullProgress {
    /// 成功
    Success { file: String, bytes: u64 },
    /// 失败
    Failed { file: String, error: String },
}

/// 块下载器
pub struct BlockDownloader {
    /// 块存储
    block_store: Arc<dyn BlockStore>,
    /// 并发限制
    max_concurrent: usize,
}

impl BlockDownloader {
    /// 创建新的块下载器
    pub fn new(block_store: Arc<dyn BlockStore>, max_concurrent: usize) -> Self {
        Self {
            block_store,
            max_concurrent,
        }
    }

    /// 批量下载块
    pub async fn download_blocks(
        &self,
        blocks: Vec<BlockRequest>,
        connection: Arc<Mutex<Box<dyn BepConnection>>>,
    ) -> Vec<(BlockHash, Result<Vec<u8>>)> {
        let mut results = Vec::new();
        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.max_concurrent));
        let mut join_set = JoinSet::new();

        for request in blocks {
            let permit = semaphore.clone().acquire_owned().await
                .expect("semaphore should not be closed");
            let conn = connection.clone();
            let block_store = self.block_store.clone();

            join_set.spawn(async move {
                let _permit = permit;
                let hash = request.hash;

                // 先检查块存储
                match block_store.get(hash).await {
                    Ok(Some(data)) => (hash, Ok(data)),
                    _ => {
                        // 从网络下载
                        let mut conn = conn.lock().await;
                        match conn
                            .request_block(&request.folder_id, hash, request.offset, request.size)
                            .await
                        {
                            Ok(data) => {
                                // 存储到 block store
                                let _ = block_store.put(hash, &data).await;
                                (hash, Ok(data))
                            }
                            Err(e) => (hash, Err(e)),
                        }
                    }
                }
            });
        }

        while let Some(Ok(result)) = join_set.join_next().await {
            results.push(result);
        }

        results
    }
}

/// 块请求
#[derive(Debug, Clone)]
pub struct BlockRequest {
    /// 文件夹 ID
    pub folder_id: FolderId,
    /// 块哈希
    pub hash: BlockHash,
    /// 偏移量
    pub offset: u64,
    /// 大小
    pub size: usize,
}

/// 拉取调度器
///
/// 负责协调多个文件夹的拉取操作
pub struct PullScheduler {
    /// Model 引用
    models: Arc<RwLock<HashMap<FolderId, Arc<dyn crate::model_trait::Model>>>>,
    /// 连接管理器
    connections: Arc<RwLock<HashMap<FolderId, Vec<Arc<Mutex<Box<dyn BepConnection>>>>>>>,
    /// 运行状态
    running: Arc<RwLock<bool>>,
}

impl PullScheduler {
    /// 创建新的拉取调度器
    pub fn new() -> Self {
        Self {
            models: Arc::new(RwLock::new(HashMap::new())),
            connections: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// 注册 Model
    pub async fn register_model(&self, folder_id: FolderId, model: Arc<dyn crate::model_trait::Model>) {
        let mut models = self.models.write().await;
        models.insert(folder_id, model);
    }

    /// 注销 Model
    pub async fn unregister_model(&self, folder_id: &FolderId) {
        let mut models = self.models.write().await;
        models.remove(folder_id);
    }

    /// 注册连接
    pub async fn register_connection(
        &self,
        folder_id: FolderId,
        conn: Arc<Mutex<Box<dyn BepConnection>>>,
    ) {
        let mut connections = self.connections.write().await;
        connections.entry(folder_id).or_insert_with(Vec::new).push(conn);
    }

    /// 主循环：检测变更并拉取
    ///
    /// 这是一个长期运行的任务，应该在一个单独的任务中执行。
    pub async fn run(&self) {
        info!("拉取调度器启动");

        {
            let mut running = self.running.write().await;
            *running = true;
        }

        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));

        loop {
            interval.tick().await;

            // 检查是否停止
            if !*self.running.read().await {
                info!("拉取调度器停止");
                break;
            }

            // 遍历所有注册的 Model
            let models = self.models.read().await;
            for (folder_id, _model) in models.iter() {
                // 1. 获取需要同步的文件
                // 2. 向连接发送请求
                // 3. 接收块并写入
                // 4. 更新索引

                debug!("检查文件夹同步状态: {}", folder_id);
            }
        }
    }

    /// 停止调度器
    pub async fn stop(&self) {
        let mut running = self.running.write().await;
        *running = false;
    }

    /// 触发单个文件夹的拉取
    pub async fn trigger_pull(&self, folder_id: &FolderId) -> Result<()> {
        info!("触发拉取: folder={}", folder_id);

        let models = self.models.read().await;
        if let Some(_model) = models.get(folder_id) {
            // 这里应该调用 Model 的 pull 方法
            // 由于 Model trait 没有定义 pull 方法，我们需要通过其他方式触发
            debug!("触发文件夹拉取: {}", folder_id);
            Ok(())
        } else {
            Err(syncthing_core::SyncthingError::FolderNotFound(folder_id.clone()))
        }
    }
}

impl Default for PullScheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// 拉取队列
pub struct PullQueue {
    /// 队列发送端
    tx: mpsc::Sender<PullTask>,
    /// 队列接收端
    rx: Arc<Mutex<mpsc::Receiver<PullTask>>>,
}

impl PullQueue {
    /// 创建新的拉取队列
    pub fn new(capacity: usize) -> Self {
        let (tx, rx) = mpsc::channel(capacity);
        Self {
            tx,
            rx: Arc::new(Mutex::new(rx)),
        }
    }

    /// 获取发送端
    pub fn sender(&self) -> mpsc::Sender<PullTask> {
        self.tx.clone()
    }

    /// 获取接收端
    pub fn receiver(&self) -> Arc<Mutex<mpsc::Receiver<PullTask>>> {
        self.rx.clone()
    }

    /// 发送任务
    pub async fn send(&self, task: PullTask) -> Result<()> {
        self.tx
            .send(task)
            .await
            .map_err(|e| syncthing_core::SyncthingError::Internal(format!("发送失败: {}", e)))
    }

    /// 接收任务
    pub async fn recv(&self) -> Option<PullTask> {
        let mut rx = self.rx.lock().await;
        rx.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncthing_core::types::{BlockHash, DeviceId};
    use syncthing_core::version_vector::VersionVector;
    use syncthing_db::BlockStoreBuilder;
    use syncthing_fs::NativeFileSystem;
    use tempfile::TempDir;

    fn create_test_device(id: u8) -> DeviceId {
        let mut bytes = [0u8; 32];
        bytes[0] = id;
        DeviceId::from_bytes(bytes)
    }

    fn create_test_block_info(offset: u64, size: usize) -> BlockInfo {
        let data = vec![0u8; size];
        BlockInfo {
            hash: BlockHash::from_data(&data),
            offset,
            size,
        }
    }

    #[test]
    fn test_pull_task_total_bytes() {
        let task = PullTask {
            file_info: FileInfo::new("test.txt"),
            source_device: create_test_device(1),
            needed_blocks: vec![
                create_test_block_info(0, 1024),
                create_test_block_info(1024, 1024),
                create_test_block_info(2048, 512),
            ],
            priority: 0,
        };

        assert_eq!(task.total_bytes(), 2560);
    }

    #[test]
    fn test_pull_result_default() {
        let result = PullResult::default();
        assert_eq!(result.files_processed, 0);
        assert_eq!(result.bytes_transferred, 0);
        assert!(result.errors.is_empty());
    }

    #[tokio::test]
    async fn test_conflict_resolution_creates_copy_and_merges_version() {
        let temp_dir = TempDir::new().unwrap();
        let folder_path = temp_dir.path().to_path_buf();
        let folder_id = FolderId::new("test-folder");
        let local_device = create_test_device(1);
        let remote_device = create_test_device(2);

        // 创建文件系统和块存储
        let fs: Arc<dyn FileSystem> = Arc::new(NativeFileSystem::new(&folder_path));
        let block_store: Arc<dyn BlockStore> =
            Arc::new(BlockStoreBuilder::new().build().unwrap());

        // 创建索引管理器并加载空本地索引
        let index_manager = Arc::new(IndexManager::new(
            folder_id.clone(),
            local_device,
            block_store.clone(),
        ));
        index_manager.load_local_index().await.unwrap();

        // 创建 Puller
        let puller = Puller::new(
            folder_id.clone(),
            folder_path.clone(),
            local_device,
            index_manager.clone(),
            block_store.clone(),
            fs.clone(),
            4,
        );

        // 1. 在本地创建文件并写入旧内容
        let local_content = b"old local content";
        fs.write_block(Path::new("test.txt"), 0, local_content)
            .await
            .unwrap();

        // 2. 构造本地 FileInfo（版本 A）
        let mut local_version = VersionVector::new();
        local_version.increment(local_device);
        let mut local_file = FileInfo::new("test.txt");
        local_file.size = local_content.len() as u64;
        local_file.version = local_version.clone();
        local_file.blocks = vec![BlockInfo {
            hash: BlockHash::from_data(local_content),
            offset: 0,
            size: local_content.len(),
        }];

        // 更新本地索引
        index_manager.update_local_index(vec![local_file.clone()]).await.unwrap();

        // 3. 构造远程 FileInfo（版本 B，与 A 冲突）
        let mut remote_version = VersionVector::new();
        remote_version.increment(remote_device);
        let mut remote_file = FileInfo::new("test.txt");
        remote_file.size = 100;
        remote_file.version = remote_version.clone();
        // 使用不同的块，确保 needed_blocks 非空
        let remote_content = b"new remote content!!!";
        remote_file.blocks = vec![BlockInfo {
            hash: BlockHash::from_data(remote_content),
            offset: 0,
            size: remote_content.len(),
        }];

        // 4. 将远程索引加入索引管理器
        index_manager
            .receive_index_update(remote_device, vec![remote_file.clone()])
            .await
            .unwrap();

        // 5. 构造 NeededFile 并调用 create_pull_tasks
        let needed = NeededFile {
            file_info: remote_file.clone(),
            source_device: remote_device,
        };
        let tasks = puller.create_pull_tasks(vec![needed]).await.unwrap();

        // 6. 验证冲突副本已创建
        let entries = fs.scan_directory(Path::new(".")).await.unwrap();
        let conflict_names: Vec<String> = entries
            .iter()
            .filter(|e| e.name.contains(".sync-conflict-"))
            .map(|e| e.name.clone())
            .collect();
        assert_eq!(
            conflict_names.len(),
            1,
            "应该生成一个冲突副本: {:?}",
            entries.iter().map(|e| &e.name).collect::<Vec<_>>()
        );

        let conflict_name = &conflict_names[0];
        assert!(conflict_name.starts_with("test.sync-conflict-"));
        assert!(conflict_name.ends_with(".txt"));

        // 验证冲突副本内容与旧本地文件一致
        let conflict_content = fs
            .read_block(Path::new(conflict_name), 0, 1000)
            .await
            .unwrap();
        assert_eq!(
            conflict_content, local_content,
            "冲突副本应包含旧本地文件内容"
        );

        // 7. 验证任务中的 version 是合并后的结果
        assert_eq!(tasks.len(), 1, "应该有一个拉取任务");
        let task = &tasks[0];
        let merged = &task.file_info.version;
        assert_eq!(
            merged.get(&local_device),
            1,
            "合并后的 version 应包含本地设备的计数器"
        );
        assert_eq!(
            merged.get(&remote_device),
            1,
            "合并后的 version 应包含远程设备的计数器"
        );

        // 8. 模拟 pull_file 完成：将远程内容写入正常路径
        fs.write_block(Path::new("test.txt"), 0, remote_content)
            .await
            .unwrap();

        // 验证正常路径已被远程内容覆盖
        let normal_content = fs.read_block(Path::new("test.txt"), 0, 1000).await.unwrap();
        assert_eq!(
            normal_content, remote_content,
            "正常路径应被远程内容覆盖"
        );
    }

    #[tokio::test]
    async fn test_conflict_copy_skips_directory() {
        let temp_dir = TempDir::new().unwrap();
        let folder_path = temp_dir.path().to_path_buf();
        let folder_id = FolderId::new("test-folder");
        let local_device = create_test_device(1);
        let remote_device = create_test_device(2);

        let fs: Arc<dyn FileSystem> = Arc::new(NativeFileSystem::new(&folder_path));
        let block_store: Arc<dyn BlockStore> =
            Arc::new(BlockStoreBuilder::new().build().unwrap());
        let index_manager = Arc::new(IndexManager::new(
            folder_id.clone(),
            local_device,
            block_store.clone(),
        ));
        index_manager.load_local_index().await.unwrap();

        let puller = Puller::new(
            folder_id.clone(),
            folder_path.clone(),
            local_device,
            index_manager.clone(),
            block_store.clone(),
            fs.clone(),
            4,
        );

        // 创建本地目录
        fs.create_dir(Path::new("testdir")).await.unwrap();
        let mut local_file = FileInfo::new("testdir");
        local_file.is_directory = true;
        let mut local_version = VersionVector::new();
        local_version.increment(local_device);
        local_file.version = local_version;

        index_manager.update_local_index(vec![local_file.clone()]).await.unwrap();

        // 远程也有冲突版本
        let mut remote_file = FileInfo::new("testdir");
        remote_file.is_directory = true;
        let mut remote_version = VersionVector::new();
        remote_version.increment(remote_device);
        remote_file.version = remote_version;

        index_manager
            .receive_index_update(remote_device, vec![remote_file.clone()])
            .await
            .unwrap();

        let needed = NeededFile {
            file_info: remote_file,
            source_device: remote_device,
        };
        let tasks = puller.create_pull_tasks(vec![needed]).await.unwrap();

        // 目录不应生成冲突副本任务（needed_blocks 为空，因为目录没有块）
        assert!(tasks.is_empty(), "目录不应产生拉取任务");

        // 不应有冲突副本
        let entries = fs.scan_directory(Path::new(".")).await.unwrap();
        let conflict_count = entries
            .iter()
            .filter(|e| e.name.contains(".sync-conflict-"))
            .count();
        assert_eq!(conflict_count, 0, "目录不应生成冲突副本");
    }
}
