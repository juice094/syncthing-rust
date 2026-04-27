use tokio::time::interval;
use tracing::{debug, info, warn};

use syncthing_core::DeviceId;

use crate::connection::ConnectionEvent;

use super::ConnectionManager;

impl ConnectionManager {
    /// 启动事件处理任务
    pub(crate) fn spawn_event_handler(&self) {
        let mut event_rx = self.event_rx.write().take()
            .expect("event receiver already taken");
        
        let weak = self.self_weak.read().clone().unwrap();
        
        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                if let Some(manager) = weak.upgrade() {
                    manager.handle_event(event).await;
                } else {
                    break;
                }
            }
        });
    }
    
    /// 处理连接事件
    async fn handle_event(&self, event: ConnectionEvent) {
        match event {
            ConnectionEvent::Connected { device_id } => {
                debug!("Device {} connected", device_id);
            }
            ConnectionEvent::Disconnected { reason } => {
                // 需要从连接中找到设备ID
                let _device_id: Option<DeviceId> = None; // 简化处理
                info!("Device disconnected: {:?} - {}", _device_id, reason);
                
                // 从活跃连接中移除
                // self.connections.remove(&device_id); 简化处理
                
                // 触发重连（如果适用）
                if let Some(ref d) = _device_id {
                    if self.should_reconnect(d, &reason) {
                        self.schedule_reconnect(*d).await;
                    }
                    
                    // 触发回调
                    if let Some(callback) = self.on_disconnected.read().as_ref() {
                        callback(*d, reason.clone());
                    }
                }
            }
            ConnectionEvent::Error { error } => {
                warn!("Connection error: {}", error);
            }
            _ => {}
        }
    }
    
    /// 启动维护任务
    pub(crate) fn spawn_maintenance_task(&self) {
        let interval_duration = self.config.heartbeat_interval;
        
        let weak = self.self_weak.read().clone().unwrap();
        
        let handle = tokio::spawn(async move {
            let mut ticker = interval(interval_duration);
            
            loop {
                ticker.tick().await;
                if let Some(manager) = weak.upgrade() {
                    manager.cleanup_stale_connections().await;
                } else {
                    break;
                }
            }
        });
        
        *self.maintenance_handle.write() = Some(handle);
    }
}
