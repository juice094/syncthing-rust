//! Module: syncthing-sync
//! Worker: Agent-Sync
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! Connection Manager - 连接管理器
//!
//! 该模块实现 ConnectionManager，负责管理所有活动的 BEP 连接。

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use syncthing_core::traits::BepConnection;
use syncthing_core::types::DeviceId;
use syncthing_core::Result;

/// 连接管理器
///
/// 管理所有活动的 BEP 连接，提供连接查询和生命周期管理
pub struct ConnectionManager {
    /// 设备 ID 到连接的映射
    connections: RwLock<HashMap<DeviceId, Arc<RwLock<Box<dyn BepConnection>>>>>,
}

impl fmt::Debug for ConnectionManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionManager")
            .field("connections", &"<...>")
            .finish()
    }
}

impl ConnectionManager {
    /// 创建新的连接管理器
    pub fn new() -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
        }
    }

    /// 添加连接
    ///
    /// # 参数
    /// * `device` - 设备 ID
    /// * `conn` - BEP 连接
    pub async fn add_connection(&self, device: DeviceId, conn: Box<dyn BepConnection>) {
        let mut connections = self.connections.write().await;
        connections.insert(device, Arc::new(RwLock::new(conn)));
        info!("添加连接: device={}", device.short_id());
    }

    /// 移除连接
    ///
    /// # 参数
    /// * `device` - 设备 ID
    pub async fn remove_connection(&self, device: &DeviceId) {
        let mut connections = self.connections.write().await;
        if connections.remove(device).is_some() {
            info!("移除连接: device={}", device.short_id());
        }
    }

    /// 获取连接
    ///
    /// # 参数
    /// * `device` - 设备 ID
    ///
    /// # 返回
    /// 连接的可选引用
    pub async fn get_connection(
        &self,
        device: DeviceId,
    ) -> Option<Arc<RwLock<Box<dyn BepConnection>>>> {
        let connections = self.connections.read().await;
        connections.get(&device).cloned()
    }

    /// 检查是否有到指定设备的连接
    ///
    /// # 参数
    /// * `device` - 设备 ID
    ///
    /// # 返回
    /// 如果连接存在且活跃则返回 true
    pub async fn has_connection(&self, device: &DeviceId) -> bool {
        let connections = self.connections.read().await;
        if let Some(conn) = connections.get(device) {
            let conn = conn.read().await;
            conn.is_alive()
        } else {
            false
        }
    }

    /// 获取所有已连接的设备
    ///
    /// # 返回
    /// 已连接设备 ID 列表
    pub async fn connected_devices(&self) -> Vec<DeviceId> {
        let connections = self.connections.read().await;
        connections.keys().cloned().collect()
    }

    /// 获取连接数量
    pub async fn connection_count(&self) -> usize {
        let connections = self.connections.read().await;
        connections.len()
    }

    /// 关闭所有连接
    /// 
    /// 注意：由于 BepConnection::close 消耗 self，
    /// 此方法仅从映射中移除连接而不调用 close。
    /// 调用者应在移除前手动关闭连接。
    pub async fn close_all(&self) -> Result<()> {
        let mut connections = self.connections.write().await;
        let count = connections.len();
        connections.clear();
        info!("已清理 {} 个连接", count);
        Ok(())
    }

    /// 广播索引到所有连接
    ///
    /// # 参数
    /// * `folder` - 文件夹 ID
    /// * `files` - 文件列表
    pub async fn broadcast_index(
        &self,
        folder: &syncthing_core::types::FolderId,
        files: Vec<syncthing_core::types::FileInfo>,
    ) -> Result<()> {
        let connections = self.connections.read().await;
        
        for (device, conn) in connections.iter() {
            let mut conn = conn.write().await;
            match conn.send_index(folder, files.clone()).await {
                Ok(_) => {
                    debug!("发送索引到设备 {} 成功", device.short_id());
                }
                Err(e) => {
                    warn!("发送索引到设备 {} 失败: {}", device.short_id(), e);
                }
            }
        }
        
        Ok(())
    }

    /// 广播索引更新到所有连接
    ///
    /// # 参数
    /// * `folder` - 文件夹 ID
    /// * `files` - 更新的文件列表
    pub async fn broadcast_index_update(
        &self,
        folder: &syncthing_core::types::FolderId,
        files: Vec<syncthing_core::types::FileInfo>,
    ) -> Result<()> {
        let connections = self.connections.read().await;
        
        for (device, conn) in connections.iter() {
            let mut conn = conn.write().await;
            match conn.send_index_update(folder, files.clone()).await {
                Ok(_) => {
                    debug!("发送索引更新到设备 {} 成功", device.short_id());
                }
                Err(e) => {
                    warn!("发送索引更新到设备 {} 失败: {}", device.short_id(), e);
                }
            }
        }
        
        Ok(())
    }

    /// 请求块数据
    ///
    /// # 参数
    /// * `device` - 目标设备
    /// * `folder` - 文件夹 ID
    /// * `hash` - 块哈希
    /// * `offset` - 偏移量
    /// * `size` - 大小
    ///
    /// # 返回
    /// 块数据
    pub async fn request_block(
        &self,
        device: DeviceId,
        folder: &syncthing_core::types::FolderId,
        hash: syncthing_core::types::BlockHash,
        offset: u64,
        size: usize,
    ) -> Result<Vec<u8>> {
        let conn = self.get_connection(device).await
            .ok_or_else(|| syncthing_core::SyncthingError::DeviceNotFound(device))?;
        
        let mut conn = conn.write().await;
        conn.request_block(folder, hash, offset, size).await
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 连接事件
#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    /// 设备已连接
    Connected {
        /// 设备 ID
        device: DeviceId,
        /// 连接地址
        addr: String,
    },
    /// 设备已断开
    Disconnected {
        /// 设备 ID
        device: DeviceId,
        /// 断开原因
        reason: Option<String>,
    },
    /// 收到消息
    MessageReceived {
        /// 设备 ID
        device: DeviceId,
        /// 消息类型
        message_type: String,
    },
}

/// 连接管理器句柄
///
/// 用于在不同任务间共享连接管理器
#[derive(Debug, Clone)]
pub struct ConnectionManagerHandle {
    inner: Arc<ConnectionManager>,
}

impl ConnectionManagerHandle {
    /// 创建新的句柄
    pub fn new(manager: Arc<ConnectionManager>) -> Self {
        Self { inner: manager }
    }

    /// 获取连接
    pub async fn get_connection(
        &self,
        device: DeviceId,
    ) -> Option<Arc<RwLock<Box<dyn BepConnection>>>> {
        self.inner.get_connection(device).await
    }

    /// 检查连接是否存在
    pub async fn has_connection(&self, device: &DeviceId) -> bool {
        self.inner.has_connection(device).await
    }

    /// 请求块数据
    pub async fn request_block(
        &self,
        device: DeviceId,
        folder: &syncthing_core::types::FolderId,
        hash: syncthing_core::types::BlockHash,
        offset: u64,
        size: usize,
    ) -> Result<Vec<u8>> {
        self.inner.request_block(device, folder, hash, offset, size).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_device(id: u8) -> DeviceId {
        let mut bytes = [0u8; 32];
        bytes[0] = id;
        DeviceId::from_bytes(bytes)
    }

    #[test]
    fn test_connection_manager_new() {
        let manager = ConnectionManager::new();
        // 新创建的连接管理器应该为空
        assert_eq!(tokio::runtime::Runtime::new().unwrap().block_on(async {
            manager.connection_count().await
        }), 0);
    }

    #[test]
    fn test_connection_events() {
        let device = create_test_device(1);
        
        let connected = ConnectionEvent::Connected {
            device,
            addr: "127.0.0.1:22000".to_string(),
        };
        
        match &connected {
            ConnectionEvent::Connected { device: d, addr } => {
                assert_eq!(d, &device);
                assert_eq!(addr, "127.0.0.1:22000");
            }
            _ => panic!("Expected Connected event"),
        }

        let disconnected = ConnectionEvent::Disconnected {
            device,
            reason: Some("timeout".to_string()),
        };
        
        match &disconnected {
            ConnectionEvent::Disconnected { device: d, reason } => {
                assert_eq!(d, &device);
                assert_eq!(reason.as_ref().unwrap(), "timeout");
            }
            _ => panic!("Expected Disconnected event"),
        }
    }

    #[tokio::test]
    async fn test_connection_manager_handle() {
        let manager = Arc::new(ConnectionManager::new());
        let handle = ConnectionManagerHandle::new(manager.clone());
        
        // 测试初始状态
        assert!(!handle.has_connection(&create_test_device(1)).await);
        assert!(handle.get_connection(create_test_device(1)).await.is_none());
    }
}
