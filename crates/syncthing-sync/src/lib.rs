//! Syncthing 同步状态机
//! 
//! 实现完整的文件同步逻辑，包括：
//! - 扫描循环 (scan_loop): 定期检测本地文件变更
//! - 拉取循环 (pull_loop): 从远程设备同步文件
//! - 索引处理 (index_handler): 处理接收到的索引更新
//! - 冲突解决: 处理版本冲突

pub mod error;
pub mod model;
pub mod folder_model;
pub mod service;
pub mod scanner;
pub mod puller;
pub mod index;
pub mod index_handler;
pub mod conflict_resolver;
pub mod events;
pub mod database;
pub mod supervisor;
pub mod sync_task;
pub mod block_server;
pub mod watcher;
pub mod ignore;

pub use supervisor::{Supervisor, RestartConfig, RestartPolicy};

pub use error::{SyncError, Result};
pub use model::{SyncManager, FolderState};
pub use service::SyncService;
pub use events::{SyncEvent, EventPublisher, EventSubscriber};
pub use puller::BlockSource;
