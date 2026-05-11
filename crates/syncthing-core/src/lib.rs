//! Syncthing Core Library
//!
//! 提供核心的错误类型、设备ID定义和基础工具

pub mod device_id;
pub mod error;
pub mod identity;
pub mod paths;
pub mod traits;
pub mod types;
pub mod validation;

pub use device_id::DeviceId;
pub use error::{Result, SyncthingError};
pub use identity::{DeviceIdentity, Identity};
pub use traits::{AggregateConnectionStats, ConnectionInfo, ConnectionManager, FolderDatabase};
pub use traits::{
    BoxedPipe, PathQuality, ReliablePipe, Transport, TransportListener, TransportType,
};
pub use types::*;
