use std::sync::Arc;

use tracing::{debug, info, warn};

use syncthing_core::{ConnectionType, DeviceId, SyncthingError};

use crate::connection::{BepConnection, ConnectionEvent};

use super::{ConnectionEntry, ConnectionManager};

impl ConnectionManager {
    /// 注册新连接
    pub(crate) async fn register_connection(&self, device_id: DeviceId, conn: Arc<BepConnection>) -> syncthing_core::Result<()> {
        debug!("Registering connection for device {}", device_id);
        
        let conn_id = conn.id();
        let new_conn_type = conn.connection_type();
        
        // 连接竞争解决（Connection Race Resolution）
        // 当双方同时建立连接时，各自会有 incoming + outgoing 两条连接。
        // Syncthing 规则：device ID 较小的设备保留 incoming，关闭 outgoing；
        //                 device ID 较大的设备保留 outgoing，关闭 incoming。
        // 这样双方保留的是同一个物理连接。
        if let Some(nested) = self.connections.get_mut(&device_id) {
            if let Some(existing) = nested.iter().next() {
                let old_conn_id = *existing.key();
                let old_conn_type = existing.value().conn.connection_type();
                
                let should_replace = if old_conn_type == new_conn_type {
                    // 同类型连接：保留旧的，避免频繁切换
                    false
                } else {
                    // 不同类型：根据 device ID 竞争解决
                    let local_smaller = self.local_device_id.0 < device_id.0;
                    match (old_conn_type, new_conn_type) {
                        (ConnectionType::Outgoing, ConnectionType::Incoming) => {
                            // 旧 outgoing，新 incoming
                            // local_smaller → 保留 incoming（新）
                            local_smaller
                        }
                        (ConnectionType::Incoming, ConnectionType::Outgoing) => {
                            // 旧 incoming，新 outgoing
                            // local_larger → 保留 outgoing（新）
                            !local_smaller
                        }
                        _ => unreachable!(),
                    }
                };
                
                if should_replace {
                    info!("Closing existing connection {} for device {} (new {} via race resolution)", 
                          old_conn_id, device_id, conn_id);
                    existing.value().conn.close().await.ok();
                    nested.clear();
                    nested.insert(conn_id, ConnectionEntry::new(Arc::clone(&conn)));
                } else {
                    info!("Closing new connection {} for device {} (keeping existing {} via race resolution)", 
                          conn_id, device_id, old_conn_id);
                    conn.close().await.ok();
                    return Ok(());
                }
            } else {
                nested.insert(conn_id, ConnectionEntry::new(Arc::clone(&conn)));
            }
        } else {
            let nested = dashmap::DashMap::new();
            nested.insert(conn_id, ConnectionEntry::new(Arc::clone(&conn)));
            self.connections.insert(device_id, nested);
        }
        
        self.conn_id_index.insert(conn_id, device_id);
        
        // 清除 pending 状态并重置重试计数（连接成功）
        {
            let mut pending = self.pending_connections.write().await;
            if pending.remove(&device_id).is_some() {
                debug!("Cleared pending state for {} (connection established)", device_id);
            }
        }
        
        // 设置连接的设备ID
        conn.set_device_id(device_id);
        
        // 从待连接列表中移除
        self.pending_connections.write().await.remove(&device_id);
        
        info!("Connection registered for device {} (conn_id: {}, type: {:?})", device_id, conn_id, new_conn_type);
        
        // 触发回调
        if let Some(callback) = self.on_connected.read().as_ref() {
            callback(device_id);
        }
        
        // 发送事件
        let _ = self.event_tx.send(ConnectionEvent::Connected {
            device_id,
        });
        
        Ok(())
    }
    
    /// 注册传入连接（设备ID已知时直接注册）
    pub(crate) async fn register_incoming(&self, conn: Arc<BepConnection>) -> syncthing_core::Result<()> {
        debug!("Registering incoming connection {}", conn.id());
        
        if let Some(device_id) = conn.device_id() {
            self.register_connection(device_id, conn).await
        } else {
            warn!("Incoming connection {} has no device_id, skipping registration", conn.id());
            Err(SyncthingError::connection("incoming connection missing device ID"))
        }
    }
    
    /// 断开与设备的连接
    pub(crate) async fn disconnect(&self, device_id: &DeviceId, reason: &str) -> syncthing_core::Result<()> {
        info!("Disconnecting device {}: {}", device_id, reason);

        if let Some((_, nested)) = self.connections.remove(device_id) {
            for entry in nested {
                let (conn_id, e) = entry;
                e.conn.close().await.ok();
                self.conn_id_index.remove(&conn_id);
            }

            // 触发回调
            if let Some(callback) = self.on_disconnected.read().as_ref() {
                callback(*device_id, reason.to_string());
            }
        }

        // 触发重连（如果适用）
        if self.should_reconnect(device_id, reason) {
            self.schedule_reconnect(*device_id).await;
        }

        Ok(())
    }

    /// 断开指定连接
    pub(crate) async fn disconnect_connection(&self, conn_id: &uuid::Uuid, reason: &str) -> syncthing_core::Result<()> {
        let Some((_, device_id)) = self.conn_id_index.remove(conn_id) else {
            return Ok(());
        };

        let device_has_other_conns = if let Some(nested) = self.connections.get_mut(&device_id) {
            nested.remove(conn_id);
            !nested.is_empty()
        } else {
            false
        };

        if !device_has_other_conns {
            self.connections.remove(&device_id);
            // 触发重连（如果适用）
            if self.should_reconnect(&device_id, reason) {
                self.schedule_reconnect(device_id).await;
            }
            // 触发回调
            if let Some(callback) = self.on_disconnected.read().as_ref() {
                callback(device_id, reason.to_string());
            }
        }

        Ok(())
    }
    
    /// 断开所有连接
    pub(crate) async fn disconnect_all(&self, reason: &str) {
        let devices: Vec<DeviceId> = self.connections.iter().map(|e| *e.key()).collect();
        
        for device_id in devices {
            if let Err(e) = self.disconnect(&device_id, reason).await {
                warn!("Error disconnecting {}: {}", device_id, e);
            }
        }
    }
}
