//! 同步事件系统
//! 
//! 提供事件发布和订阅机制，用于通知状态变更

use syncthing_core::types::{FileInfo, Folder, Vector};
use tokio::sync::broadcast;

/// 同步事件
#[derive(Debug, Clone)]
pub enum SyncEvent {
    /// 本地索引更新
    LocalIndexUpdated {
        folder: String,
        files: Vec<FileInfo>,
    },
    
    /// 远程索引接收
    RemoteIndexReceived {
        folder: String,
        device: syncthing_core::DeviceId,
        files: Vec<FileInfo>,
    },
    
    /// 文件夹扫描完成
    FolderScanCompleted {
        folder: String,
        files_changed: usize,
    },
    
    /// 文件夹扫描失败
    FolderScanFailed {
        folder: String,
        error: String,
    },
    
    /// 项目开始处理
    ItemStarted {
        folder: String,
        item: String,
        action: ItemAction,
    },
    
    /// 项目完成处理
    ItemFinished {
        folder: String,
        item: String,
        action: ItemAction,
        error: Option<String>,
    },
    
    /// 冲突检测
    ConflictDetected {
        folder: String,
        item: String,
        local_version: Vector,
        remote_version: Vector,
    },
    
    /// 冲突解决
    ConflictResolved {
        folder: String,
        item: String,
        resolution: ConflictResolution,
    },
    
    /// 下载进度
    DownloadProgress {
        folder: String,
        file: String,
        bytes_done: u64,
        bytes_total: u64,
    },
    
    /// 上传进度
    UploadProgress {
        folder: String,
        file: String,
        bytes_done: u64,
        bytes_total: u64,
    },
    
    /// 文件夹状态变更
    FolderStateChanged {
        folder: String,
        from: syncthing_core::types::FolderStatus,
        to: syncthing_core::types::FolderStatus,
    },
    
    /// 文件夹配置更新
    FolderConfigUpdated {
        folder: Folder,
    },
    
    /// 连接设备
    DeviceConnected {
        device: syncthing_core::DeviceId,
    },
    
    /// 断开设备
    DeviceDisconnected {
        device: syncthing_core::DeviceId,
        reason: String,
    },
    
    /// 同步完成
    SyncComplete {
        folder: String,
        stats: SyncStats,
    },
}

/// 项目动作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemAction {
    Add,
    Modify,
    Delete,
    Conflict,
}

/// 冲突解决方式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictResolution {
    UseLocal,
    UseRemote,
    Merge,
    RenameBoth,
}

/// 同步统计
#[derive(Debug, Clone, Default)]
pub struct SyncStats {
    pub files_added: usize,
    pub files_modified: usize,
    pub files_deleted: usize,
    pub bytes_transferred: u64,
    pub conflicts_detected: usize,
    pub conflicts_resolved: usize,
}

/// 事件发布者
#[derive(Debug, Clone)]
pub struct EventPublisher {
    sender: broadcast::Sender<SyncEvent>,
}

impl EventPublisher {
    /// 创建新的事件发布者
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// 发布事件
    pub fn publish(&self, event: SyncEvent) {
        let _ = self.sender.send(event);
    }

    /// 获取订阅者
    pub fn subscribe(&self) -> EventSubscriber {
        EventSubscriber {
            receiver: self.sender.subscribe(),
        }
    }
}

impl Default for EventPublisher {
    fn default() -> Self {
        Self::new(1000)
    }
}

/// 事件订阅者
#[derive(Debug)]
pub struct EventSubscriber {
    receiver: broadcast::Receiver<SyncEvent>,
}

impl EventSubscriber {
    /// 接收事件
    pub async fn recv(&mut self) -> Option<SyncEvent> {
        self.receiver.recv().await.ok()
    }

    /// 尝试接收事件（非阻塞）
    pub fn try_recv(&mut self) -> Result<SyncEvent, broadcast::error::TryRecvError> {
        self.receiver.try_recv()
    }
}

impl Clone for EventSubscriber {
    fn clone(&self) -> Self {
        Self {
            receiver: self.receiver.resubscribe(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_event_publishing() {
        let publisher = EventPublisher::new(10);
        let mut subscriber = publisher.subscribe();

        publisher.publish(SyncEvent::FolderScanCompleted {
            folder: "test".to_string(),
            files_changed: 5,
        });

        let event = subscriber.recv().await;
        assert!(matches!(event, Some(SyncEvent::FolderScanCompleted { .. })));
    }
}
