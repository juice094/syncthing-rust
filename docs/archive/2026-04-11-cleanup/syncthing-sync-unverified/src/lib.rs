//! Module: syncthing-sync
//! Worker: Agent-Sync
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! # Syncthing 同步引擎
//!
//! 这是 Syncthing Rust 实现的同步引擎模块，负责：
//!
//! - **索引管理** ([`index`] 模块): 维护本地和远程文件索引，处理索引的接收、存储和合并
//! - **拉取逻辑** ([`puller`] 模块): 从远程设备下载文件和块
//! - **推送逻辑** ([`pusher`] 模块): 响应远程设备的块请求
//! - **冲突解决** ([`conflict`] 模块): 使用版本向量检测和解决文件冲突
//! - **同步模型** ([`model`] 模块): 实现 [`SyncModel`] trait，协调各组件
//! - **模型 trait** ([`model_trait`] 模块): 定义 BEP 协议消息处理接口
//! - **连接管理** ([`connection_manager`] 模块): 管理 BEP 连接
//! - **拉取调度** ([`pull_scheduler`] 模块): 协调文件拉取任务
//! - **文件夹模型** ([`folder_model`] 模块): 文件夹级别的模型实现
//! - **主服务** ([`service`] 模块): SyncService 主服务

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

// 核心模块
pub mod accept_loop;
pub mod conflict;
pub mod connection_manager;
pub mod folder_model;
pub mod index;
pub mod model;
pub mod model_trait;
pub mod pull_scheduler;
pub mod puller;
pub mod pusher;
pub mod scan_scheduler;
pub mod service;

// 重新导出主要类型
pub use accept_loop::accept_loop;
pub use conflict::{ConflictInfo, ConflictManager, ConflictResolution, ConflictResolver};
pub use connection_manager::{ConnectionManager, ConnectionManagerHandle, ConnectionEvent};
pub use folder_model::FolderModel;
pub use index::{IndexDiff, IndexDiffer, IndexManager, IndexStats, NeededFile};
pub use model::{FolderContext, SyncConfig, SyncEngine, SyncState};
pub use model_trait::{
    ClusterConfig, DeviceInfo, DownloadProgress, FolderInfo, FolderState, Index, IndexUpdate,
    Model, ModelExt, RemoteDeviceState, Request,
};
pub use pull_scheduler::{FilePuller, PullScheduler, PullStats, PullTask, BlockDownloadTask};
pub use puller::{BlockDownloader, BlockRequest, PullResult, Puller, PullQueue};
pub use pusher::{BlockProvider, PushHandler, PushStats, Pusher};
pub use scan_scheduler::ScanScheduler;
pub use service::SyncService;

/// 模块版本信息
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 获取模块信息
pub fn module_info() -> &'static str {
    concat!(
        "syncthing-sync v",
        env!("CARGO_PKG_VERSION"),
        " (Agent-Sync implementation)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_info() {
        let info = module_info();
        assert!(info.contains("syncthing-sync"));
        assert!(info.contains("Agent-Sync"));
    }

    #[test]
    fn test_version() {
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn test_folder_state_variants() {
        // 测试 FolderState 枚举的所有变体
        assert_eq!(FolderState::Idle.to_string(), "idle");
        assert_eq!(FolderState::Scanning.to_string(), "scanning");
        assert!(FolderState::Syncing { total: 10, done: 5 }
            .to_string()
            .contains("syncing"));
        assert!(FolderState::Error { message: "test".to_string() }
            .to_string()
            .contains("error"));
        assert_eq!(FolderState::Paused.to_string(), "paused");
    }
}
