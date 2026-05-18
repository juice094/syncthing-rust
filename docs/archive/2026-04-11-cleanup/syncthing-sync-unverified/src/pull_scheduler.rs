//! Module: syncthing-sync
//! Worker: Agent-Sync
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! Pull Scheduler - 拉取调度器
//!
//! 该模块实现 PullScheduler，负责协调文件拉取任务，管理下载队列和优先级。

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;


use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::JoinSet;
use tracing::{debug, error, info, trace, warn};

use syncthing_core::traits::{BlockStore, FileSystem};
use syncthing_core::types::{BlockHash, DeviceId, FileInfo, FolderId};
use syncthing_core::Result;

use crate::connection_manager::ConnectionManagerHandle;
use crate::folder_model::FolderModel;


/// 拉取任务
#[derive(Debug, Clone)]
pub struct PullTask {
    /// 文件信息
    pub file: FileInfo,
    /// 来源设备
    pub source_device: DeviceId,
    /// 优先级（数值越大优先级越高）
    pub priority: i32,
}

impl PullTask {
    /// 创建新的拉取任务
    pub fn new(file: FileInfo, source_device: DeviceId, priority: i32) -> Self {
        Self {
            file,
            source_device,
            priority,
        }
    }
}

impl PartialEq for PullTask {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl Eq for PullTask {}

impl PartialOrd for PullTask {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PullTask {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap 是最大堆，所以高优先级应该先弹出
        self.priority.cmp(&other.priority)
    }
}

/// 拉取调度器
///
/// 管理文件拉取任务的队列和调度
#[derive(Debug)]
pub struct PullScheduler {
    /// 文件夹模型
    model: Arc<FolderModel>,
    /// 待处理任务队列
    pending_pulls: Arc<RwLock<BinaryHeap<PullTask>>>,
    /// 任务发送通道
    task_tx: mpsc::Sender<PullTask>,
    /// 任务接收通道
    task_rx: Arc<Mutex<mpsc::Receiver<PullTask>>>,
    /// 运行状态
    running: Arc<RwLock<bool>>,
    /// 并发限制
    max_concurrent: usize,
    /// 活动下载计数
    active_downloads: Arc<RwLock<usize>>,
}

impl PullScheduler {
    /// 创建新的拉取调度器
    ///
    /// # 参数
    /// * `model` - 文件夹模型
    /// * `max_concurrent` - 最大并发下载数
    pub fn new(model: Arc<FolderModel>, max_concurrent: usize) -> Self {
        let (task_tx, task_rx) = mpsc::channel(1000);
        
        Self {
            model,
            pending_pulls: Arc::new(RwLock::new(BinaryHeap::new())),
            task_tx,
            task_rx: Arc::new(Mutex::new(task_rx)),
            running: Arc::new(RwLock::new(false)),
            max_concurrent,
            active_downloads: Arc::new(RwLock::new(0)),
        }
    }

    /// 添加需要拉取的文件到队列
    ///
    /// # 参数
    /// * `file` - 文件信息
    /// * `device` - 来源设备
    /// * `priority` - 优先级（可选，默认为 0）
    pub async fn queue_pull(&self, file: FileInfo, device: DeviceId, priority: Option<i32>) {
        let task = PullTask::new(file, device, priority.unwrap_or(0));
        let file_name = task.file.name.clone();
        
        // 添加到内部队列
        {
            let mut pending = self.pending_pulls.write().await;
            pending.push(task.clone());
        }
        
        // 发送到通道
        if let Err(e) = self.task_tx.send(task).await {
            warn!("发送拉取任务失败: {}", e);
        } else {
            debug!("添加拉取任务: file={}, device={}", file_name, device.short_id());
        }
    }

    /// 获取队列长度
    pub async fn queue_len(&self) -> usize {
        let pending = self.pending_pulls.read().await;
        pending.len()
    }

    /// 获取活动下载数
    pub async fn active_downloads(&self) -> usize {
        *self.active_downloads.read().await
    }

    /// 启动调度器
    ///
    /// 开始处理拉取任务队列
    pub async fn start(&self) {
        let mut running = self.running.write().await;
        *running = true;
        info!("拉取调度器已启动");
    }

    /// 停止调度器
    pub async fn stop(&self) {
        let mut running = self.running.write().await;
        *running = false;
        info!("拉取调度器已停止");
    }

    /// 主循环：处理拉取任务
    ///
    /// # 参数
    /// * `connection_manager` - 连接管理器句柄
    pub async fn run(&self, connection_manager: ConnectionManagerHandle) {
        info!("拉取调度器主循环启动");
        
        self.start().await;
        
        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.max_concurrent));
        let mut join_set = JoinSet::new();
        
        // 创建文件拉取器
        let puller = FilePuller::new(
            self.model.folder_id().clone(),
            self.model.folder_path().clone(),
            self.model.block_store(),
            self.model.filesystem(),
        );
        
        loop {
            // 检查是否停止
            if !*self.running.read().await {
                info!("拉取调度器停止信号收到，退出主循环");
                break;
            }
            
            // 接收任务
            let task = {
                let mut rx = self.task_rx.lock().await;
                tokio::time::timeout(tokio::time::Duration::from_secs(1), rx.recv()).await
            };
            
            match task {
                Ok(Some(task)) => {
                    // 从队列中移除
                    {
                        let mut pending = self.pending_pulls.write().await;
                        pending.pop(); // 移除堆顶元素
                    }
                    
                    // 获取信号量许可
                    let permit = match semaphore.clone().acquire_owned().await {
                        Ok(p) => p,
                        Err(e) => {
                            error!("获取信号量失败: {}", e);
                            continue;
                        }
                    };
                    
                    let file_name = task.file.name.clone();
                    let downloads = self.active_downloads.clone();
                    let cm = connection_manager.clone();
                    let puller = puller.clone();
                    
                    // 更新活动下载计数
                    {
                        let mut count = downloads.write().await;
                        *count += 1;
                    }
                    
                    // 更新文件夹状态
                    self.model.update_syncing_state(true).await;
                    
                    // 在后台任务中处理拉取
                    join_set.spawn(async move {
                        let _permit = permit;
                        
                        debug!("开始处理拉取任务: file={}", file_name);
                        
                        match puller.pull_file(&task, &cm).await {
                            Ok(bytes) => {
                                debug!("完成拉取任务: file={}, bytes={}", file_name, bytes);
                            }
                            Err(e) => {
                                error!("拉取任务失败: file={}, error={}", file_name, e);
                            }
                        }
                        
                        // 减少活动下载计数
                        let mut count = downloads.write().await;
                        *count = count.saturating_sub(1);
                    });
                }
                Ok(None) => {
                    // 通道关闭
                    info!("任务通道已关闭，退出主循环");
                    break;
                }
                Err(_) => {
                    // 超时，继续检查 running 状态
                    continue;
                }
            }
        }
        
        // 等待所有任务完成
        while join_set.join_next().await.is_some() {}
        
        // 更新文件夹状态为空闲
        self.model.update_syncing_state(false).await;
        
        info!("拉取调度器主循环结束");
    }

    /// 清空队列
    pub async fn clear_queue(&self) {
        let mut pending = self.pending_pulls.write().await;
        pending.clear();
        info!("拉取队列已清空");
    }

    /// 检查是否有待处理的任务
    pub async fn has_pending(&self) -> bool {
        let pending = self.pending_pulls.read().await;
        !pending.is_empty()
    }
}

/// 块下载任务
#[derive(Debug, Clone)]
pub struct BlockDownloadTask {
    /// 文件信息
    pub file_info: FileInfo,
    /// 块信息
    pub block_info: syncthing_core::types::BlockInfo,
    /// 来源设备
    pub source_device: DeviceId,
    /// 文件夹路径
    pub folder_path: PathBuf,
}

/// 文件拉取器
///
/// 负责实际执行文件拉取操作
pub struct FilePuller {
    /// 文件夹 ID
    folder_id: FolderId,
    /// 文件夹路径
    folder_path: PathBuf,
    /// 块存储
    block_store: Arc<dyn BlockStore>,
    /// 文件系统
    file_system: Arc<dyn FileSystem>,
}

impl Clone for FilePuller {
    fn clone(&self) -> Self {
        Self {
            folder_id: self.folder_id.clone(),
            folder_path: self.folder_path.clone(),
            block_store: self.block_store.clone(),
            file_system: self.file_system.clone(),
        }
    }
}

impl fmt::Debug for FilePuller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilePuller")
            .field("folder_id", &self.folder_id)
            .field("folder_path", &self.folder_path)
            .field("block_store", &"<...>")
            .field("file_system", &"<...>")
            .finish()
    }
}

impl FilePuller {
    /// 创建新的文件拉取器
    pub fn new(
        folder_id: FolderId,
        folder_path: PathBuf,
        block_store: Arc<dyn BlockStore>,
        file_system: Arc<dyn FileSystem>,
    ) -> Self {
        Self {
            folder_id,
            folder_path,
            block_store,
            file_system,
        }
    }

    /// 拉取单个文件
    ///
    /// # 参数
    /// * `task` - 拉取任务
    /// * `connection_manager` - 连接管理器
    ///
    /// # 返回
    /// 拉取的字节数
    pub async fn pull_file(
        &self,
        task: &PullTask,
        connection_manager: &ConnectionManagerHandle,
    ) -> Result<u64> {
        let file_path = self.folder_path.join(&task.file.name);
        
        info!("开始拉取文件: {} ({} bytes)", task.file.name, task.file.size);
        
        // 确保父目录存在
        if let Some(parent) = file_path.parent() {
            self.file_system.create_dir(parent).await?;
        }
        
        let mut total_bytes = 0u64;
        
        // 下载每个块
        for block in &task.file.blocks {
            // 检查是否已有此块
            if self.block_store.has(block.hash).await? {
                trace!("块已存在: hash={:?}", block.hash);
                continue;
            }
            
            // 从远程设备请求块
            let data = connection_manager
                .request_block(
                    task.source_device,
                    &self.folder_id,
                    block.hash,
                    block.offset,
                    block.size,
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
            
            // 存储块
            self.block_store.put(block.hash, &data).await?;
            
            // 写入文件
            self.file_system
                .write_block(&file_path, block.offset, &data)
                .await?;
            
            total_bytes += data.len() as u64;
        }
        
        info!("完成拉取文件: {} ({} bytes)", task.file.name, total_bytes);
        Ok(total_bytes)
    }
}

/// 拉取统计
#[derive(Debug, Clone, Default)]
pub struct PullStats {
    /// 处理的文件数量
    pub files_processed: usize,
    /// 传输的字节数
    pub bytes_transferred: u64,
    /// 错误数量
    pub errors: usize,
    /// 队列中的任务数
    pub queued_tasks: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncthing_core::types::DeviceId;

    fn create_test_device(id: u8) -> DeviceId {
        let mut bytes = [0u8; 32];
        bytes[0] = id;
        DeviceId::from_bytes(bytes)
    }

    #[test]
    fn test_pull_task_priority() {
        let device = create_test_device(1);
        
        let task1 = PullTask::new(FileInfo::new("low.txt"), device, 1);
        let task2 = PullTask::new(FileInfo::new("high.txt"), device, 10);
        let task3 = PullTask::new(FileInfo::new("medium.txt"), device, 5);
        
        // 验证优先级排序（高优先级应该在堆顶）
        let mut heap = BinaryHeap::new();
        heap.push(task1);
        heap.push(task2);
        heap.push(task3);
        
        // 最高优先级应该先弹出
        let first = heap.pop().unwrap();
        assert_eq!(first.file.name, "high.txt");
        assert_eq!(first.priority, 10);
        
        let second = heap.pop().unwrap();
        assert_eq!(second.file.name, "medium.txt");
        assert_eq!(second.priority, 5);
        
        let third = heap.pop().unwrap();
        assert_eq!(third.file.name, "low.txt");
        assert_eq!(third.priority, 1);
    }

    #[test]
    fn test_pull_task_equality() {
        let device = create_test_device(1);
        let task1 = PullTask::new(FileInfo::new("file.txt"), device, 5);
        let task2 = PullTask::new(FileInfo::new("file.txt"), device, 5);
        
        assert_eq!(task1, task2);
    }

    #[test]
    fn test_pull_stats_default() {
        let stats = PullStats::default();
        assert_eq!(stats.files_processed, 0);
        assert_eq!(stats.bytes_transferred, 0);
        assert_eq!(stats.errors, 0);
        assert_eq!(stats.queued_tasks, 0);
    }

    #[tokio::test]
    async fn test_pull_scheduler_lifecycle() {
        // 这个测试需要 mock 对象，这里只是演示生命周期方法
        // 实际测试需要更完整的设置
    }
}
