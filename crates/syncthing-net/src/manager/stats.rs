use dashmap::DashMap;
use tokio::task::JoinHandle;
use tokio::time::sleep;

use syncthing_core::{DeviceId, RetryConfig};

/// 管理器统计信息
#[derive(Debug, Clone)]
pub struct ManagerStats {
    pub active_connections: usize,
    pub connected_devices: usize,
    pub pending_connections: usize,
    pub total_bytes_sent: u64,
    pub total_bytes_received: u64,
}

/// 重连调度器
pub struct ReconnectScheduler {
    config: RetryConfig,
    pending: DashMap<DeviceId, JoinHandle<()>>,
}

impl ReconnectScheduler {
    pub fn new(config: RetryConfig) -> Self {
        Self {
            config,
            pending: DashMap::new(),
        }
    }
    
    pub fn schedule<F>(&self, device_id: DeviceId, attempt: u32, task: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        // 取消现有的重连任务
        if let Some((_, handle)) = self.pending.remove(&device_id) {
            handle.abort();
        }
        
        let backoff = self.config.backoff_duration(attempt);
        let handle = tokio::spawn(async move {
            sleep(backoff).await;
            task.await;
        });
        
        self.pending.insert(device_id, handle);
    }
    
    pub fn cancel(&self, device_id: &DeviceId) {
        if let Some((_, handle)) = self.pending.remove(device_id) {
            handle.abort();
        }
    }
}
