//! Syncthing 同步状态机
//!
//! 实现完整的文件同步逻辑，包括：
//! - 扫描循环 (scan_loop): 定期检测本地文件变更
//! - 拉取循环 (pull_loop): 从远程设备同步文件
//! - 索引处理 (index_handler): 处理接收到的索引更新
//! - 冲突解决: 处理版本冲突

pub mod block_server;
pub mod conflict_resolver;
pub mod database;
pub mod error;
pub mod events;
pub mod folder_model;
pub mod ignore;
pub mod index;
pub mod index_handler;
pub mod merge;
pub mod model;
pub mod puller;
pub mod scanner;
pub mod service;
pub mod supervisor;
pub mod sync_task;
pub mod watcher;

pub use supervisor::{RestartConfig, RestartPolicy, Supervisor};

pub use error::{Result, SyncError};
pub use events::{EventPublisher, EventSubscriber, SyncEvent};
pub use model::{FolderState, SyncManager};
pub use puller::BlockSource;
pub use service::SyncService;
