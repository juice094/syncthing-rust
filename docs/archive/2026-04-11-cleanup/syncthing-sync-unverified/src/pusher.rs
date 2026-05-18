//! Module: syncthing-sync
//! Worker: Agent-C
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! 推送逻辑模块
//!
//! 该模块实现响应其他设备块请求的推送逻辑，包括：
//! - 接收并处理块请求消息
//! - 从本地文件系统或块存储读取块数据
//! - 发送块响应
//! - 处理索引请求

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;


use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, info, trace, warn};

use syncthing_core::traits::{BlockStore, BepConnection, BepMessage, FileSystem};
use syncthing_core::types::{BlockHash, DeviceId, FileInfo, FolderId};
use syncthing_core::Result;

use crate::index::IndexManager;

/// 设备到 BEP 连接的映射
type DeviceConnectionMap = HashMap<DeviceId, Arc<Mutex<Box<dyn BepConnection>>>>;

/// 推送器
///
/// 负责响应远程设备的块请求和索引请求
pub struct Pusher {
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
    /// 活动连接映射
    connections: Arc<RwLock<DeviceConnectionMap>>,
}

impl Pusher {
    /// 创建新的推送器
    pub fn new(
        folder_id: FolderId,
        folder_path: PathBuf,
        local_device: DeviceId,
        index_manager: Arc<IndexManager>,
        block_store: Arc<dyn BlockStore>,
        file_system: Arc<dyn FileSystem>,
    ) -> Self {
        Self {
            folder_id,
            folder_path,
            local_device,
            index_manager,
            block_store,
            file_system,
            connections: Arc::new(RwLock::new(HashMap::new())),
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
        info!("推送器注册设备连接: {}", device.short_id());
    }

    /// 注销设备连接
    pub async fn unregister_connection(&self, device: &DeviceId) {
        let mut connections = self.connections.write().await;
        if connections.remove(device).is_some() {
            info!("推送器注销设备连接: {}", device.short_id());
        }
    }

    /// 处理传入的 BEP 消息
    ///
    /// 根据消息类型分发到相应的处理函数
    pub async fn handle_message(
        &self,
        device: DeviceId,
        message: BepMessage,
    ) -> Result<Option<BepMessage>> {
        match message {
            BepMessage::Request {
                folder,
                hash,
                offset,
                size,
            } => {
                if folder != self.folder_id {
                    warn!(
                        "收到请求但文件夹不匹配: expected={}, got={}",
                        self.folder_id, folder
                    );
                    return Ok(None);
                }
                self.handle_block_request(device, hash, offset, size).await
            }
            BepMessage::Index { folder, files } => {
                if folder != self.folder_id {
                    return Ok(None);
                }
                self.handle_index(device, files).await?;
                Ok(None)
            }
            BepMessage::IndexUpdate { folder, files } => {
                if folder != self.folder_id {
                    return Ok(None);
                }
                self.handle_index_update(device, files).await?;
                Ok(None)
            }
            _ => {
                trace!("推送器忽略消息类型: {:?}", message);
                Ok(None)
            }
        }
    }

    /// 处理块请求
    ///
    /// 从本地读取块数据并返回响应
    async fn handle_block_request(
        &self,
        device: DeviceId,
        hash: BlockHash,
        offset: u64,
        size: usize,
    ) -> Result<Option<BepMessage>> {
        const MAX_BLOCK_SIZE: usize = 16 * 1024 * 1024;
        if size > MAX_BLOCK_SIZE {
            return Err(syncthing_core::SyncthingError::Protocol(format!(
                "Block size {} exceeds maximum {}",
                size, MAX_BLOCK_SIZE
            )));
        }

        trace!(
            "处理块请求: device={}, hash={:?}, offset={}, size={}",
            device.short_id(),
            hash,
            offset,
            size
        );

        // 首先尝试从块存储获取
        let data = match self.block_store.get(hash).await? {
            Some(data) => {
                trace!("从块存储找到块: hash={:?}", hash);
                data
            }
            None => {
                // 从文件系统读取
                trace!("从文件系统读取块: hash={:?}", hash);
                self.read_block_from_filesystem(hash, offset, size).await?
            }
        };

        // 验证数据大小
        if data.len() != size {
            warn!(
                "块大小不匹配: expected={}, got={}",
                size,
                data.len()
            );
        }

        trace!(
            "发送块响应: device={}, hash={:?}, size={}",
            device.short_id(),
            hash,
            data.len()
        );

        Ok(Some(BepMessage::Response { hash, data }))
    }

    /// 从文件系统读取块
    ///
    /// 根据块哈希查找对应的文件并读取数据
    async fn read_block_from_filesystem(
        &self,
        hash: BlockHash,
        _offset: u64,
        _size: usize,
    ) -> Result<Vec<u8>> {
        // 获取所有本地文件
        let local_files = self.index_manager.get_all_local_files().await;

        // 查找包含此块的文件
        for file_info in local_files {
            for block in &file_info.blocks {
                if block.hash == hash {
                    let file_path = self.folder_path.join(&file_info.name);
                    trace!(
                        "在文件 {} 中找到块: hash={:?}",
                        file_info.name,
                        hash
                    );

                    // 从文件读取块数据
                    let data = self
                        .file_system
                        .read_block(&file_path, block.offset, block.size)
                        .await?;

                    return Ok(data);
                }
            }
        }

        // 块未找到
        error!("请求的块未找到: hash={:?}", hash);
        Err(syncthing_core::SyncthingError::BlockNotFound(hash))
    }

    /// 处理完整索引
    ///
    /// 接收远程设备的完整索引
    async fn handle_index(
        &self,
        device: DeviceId,
        files: Vec<FileInfo>,
    ) -> Result<()> {
        info!(
            "接收完整索引: device={}, files={}",
            device.short_id(),
            files.len()
        );

        self.index_manager
            .receive_full_index(device, files)
            .await?;

        Ok(())
    }

    /// 处理索引更新
    ///
    /// 接收远程设备的索引更新（增量）
    async fn handle_index_update(
        &self,
        device: DeviceId,
        files: Vec<FileInfo>,
    ) -> Result<()> {
        debug!(
            "接收索引更新: device={}, files={}",
            device.short_id(),
            files.len()
        );

        self.index_manager
            .receive_index_update(device, files)
            .await?;

        Ok(())
    }

    /// 发送本地索引到指定设备
    ///
    /// 通常在连接建立后调用
    pub async fn send_index(&self, device: DeviceId) -> Result<()> {
        info!("发送本地索引到设备: {}", device.short_id());

        let files = self.index_manager.get_all_local_files().await;

        let connections = self.connections.read().await;
        if let Some(conn) = connections.get(&device) {
            let mut conn = conn.lock().await;
            conn.send_index(&self.folder_id, files.clone()).await?;
            debug!("已发送 {} 个文件的索引", files.len());
        } else {
            warn!("设备 {} 未连接，无法发送索引", device.short_id());
        }

        Ok(())
    }

    /// 发送索引更新到所有连接的设备
    ///
    /// 当本地文件发生变化时调用
    pub async fn broadcast_index_update(&self, files: Vec<FileInfo>) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }

        debug!("广播索引更新: {} 个文件", files.len());

        let connections = self.connections.read().await;
        for (device, conn) in connections.iter() {
            let mut conn = conn.lock().await;
            if let Err(e) = conn.send_index_update(&self.folder_id, files.clone()).await {
                warn!("向设备 {} 发送索引更新失败: {}", device.short_id(), e);
            } else {
                trace!("已向设备 {} 发送索引更新", device.short_id());
            }
        }

        Ok(())
    }

    /// 处理块响应（作为请求的发送方）
    ///
    /// 存储接收到的块数据
    pub async fn handle_block_response(
        &self,
        hash: BlockHash,
        data: Vec<u8>,
    ) -> Result<()> {
        trace!("处理块响应: hash={:?}, size={}", hash, data.len());

        // 验证哈希
        let computed_hash = BlockHash::from_data(&data);
        if computed_hash != hash {
            return Err(syncthing_core::SyncthingError::Protocol(format!(
                "Invalid block: expected {}, got {}",
                hash, computed_hash
            )));
        }

        // 存储块
        self.block_store.put(hash, &data).await?;

        Ok(())
    }
}

/// 推送处理器
///
/// 处理单个连接的推送逻辑
pub struct PushHandler {
    /// 设备 ID
    device: DeviceId,
    /// 连接
    connection: Arc<Mutex<Box<dyn BepConnection>>>,
    /// 文件夹推送器映射
    pushers: Arc<RwLock<HashMap<FolderId, Arc<Pusher>>>>,
}

impl PushHandler {
    /// 创建新的推送处理器
    pub fn new(
        device: DeviceId,
        connection: Box<dyn BepConnection>,
        pushers: Arc<RwLock<HashMap<FolderId, Arc<Pusher>>>>,
    ) -> Self {
        Self {
            device,
            connection: Arc::new(Mutex::new(connection)),
            pushers,
        }
    }

    /// 运行推送处理器
    ///
    /// 循环接收消息并处理
    pub async fn run(&self) -> Result<()> {
        info!("启动推送处理器: device={}", self.device.short_id());

        loop {
            let message = {
                let mut conn = self.connection.lock().await;
                conn.recv_message().await?
            };

            match message {
                Some(msg) => {
                    self.handle_message(msg).await?;
                }
                None => {
                    info!("连接关闭: device={}", self.device.short_id());
                    break;
                }
            }
        }

        Ok(())
    }

    /// 处理单个消息
    async fn handle_message(&self, message: BepMessage) -> Result<()> {
        // 根据消息中的文件夹 ID 找到对应的推送器
        let folder_id = match &message {
            BepMessage::Index { folder, .. } => Some(folder.clone()),
            BepMessage::IndexUpdate { folder, .. } => Some(folder.clone()),
            BepMessage::Request { folder, .. } => Some(folder.clone()),
            _ => None,
        };

        if let Some(folder_id) = folder_id {
            let pushers = self.pushers.read().await;
            if let Some(pusher) = pushers.get(&folder_id) {
                if let Some(response) = pusher.handle_message(self.device, message).await? {
                    let _conn = self.connection.lock().await;
                    // 注意：这里需要根据 BepConnection 的具体实现来发送响应
                    // 可能需要添加 send_response 方法到 trait
                    let _ = response;
                    trace!("发送响应");
                }
            } else {
                warn!("未找到文件夹 {} 的推送器", folder_id);
            }
        }

        Ok(())
    }
}

/// 推送统计信息
#[derive(Debug, Clone, Default)]
pub struct PushStats {
    /// 处理的请求数量
    pub requests_handled: u64,
    /// 发送的块数量
    pub blocks_sent: u64,
    /// 发送的字节数
    pub bytes_sent: u64,
    /// 错误数量
    pub errors: u64,
}

/// 块提供者
///
/// 负责提供块数据给远程设备
pub struct BlockProvider {
    /// 块存储
    block_store: Arc<dyn BlockStore>,
    /// 文件系统
    file_system: Arc<dyn FileSystem>,
    /// 文件夹路径
    folder_path: PathBuf,
}

impl BlockProvider {
    /// 创建新的块提供者
    pub fn new(
        block_store: Arc<dyn BlockStore>,
        file_system: Arc<dyn FileSystem>,
        folder_path: PathBuf,
    ) -> Self {
        Self {
            block_store,
            file_system,
            folder_path,
        }
    }

    /// 获取块数据
    ///
    /// 按优先级尝试从以下位置获取：
    /// 1. 块存储
    /// 2. 本地文件系统
    pub async fn get_block(
        &self,
        hash: BlockHash,
        file_info: Option<&FileInfo>,
    ) -> Result<Vec<u8>> {
        // 首先尝试块存储
        if let Some(data) = self.block_store.get(hash).await? {
            trace!("从块存储获取块: hash={:?}", hash);
            return Ok(data);
        }

        // 从文件系统获取
        if let Some(file) = file_info {
            for block in &file.blocks {
                if block.hash == hash {
                    let file_path = self.folder_path.join(&file.name);
                    trace!(
                        "从文件系统获取块: file={}, offset={}, size={}",
                        file.name,
                        block.offset,
                        block.size
                    );
                    return self
                        .file_system
                        .read_block(&file_path, block.offset, block.size)
                        .await;
                }
            }
        }

        Err(syncthing_core::SyncthingError::BlockNotFound(hash))
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

    #[test]
    fn test_push_stats_default() {
        let stats = PushStats::default();
        assert_eq!(stats.requests_handled, 0);
        assert_eq!(stats.blocks_sent, 0);
        assert_eq!(stats.bytes_sent, 0);
        assert_eq!(stats.errors, 0);
    }
}
