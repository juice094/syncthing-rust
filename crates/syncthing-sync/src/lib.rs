//! Sync state machine.
//!
//! Implements the full file-synchronization logic:
//! - scan loop (`scanner`): periodic detection of local file changes
//! - pull loop (`puller`): synchronizing files from remote devices
//! - index handling (`index_handler`): processing received index updates
//! - conflict resolution (`conflict_resolver`, `merge`)

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
pub mod orchestrator;
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
pub use orchestrator::{FolderOrchestrator, OrchestratorConfig, OrchestratorLoad};
pub use puller::{
    concurrency::{ConcurrencyPolicy, RttTracker},
    BlockSource,
};
pub use service::SyncService;
