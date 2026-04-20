//! Syncthing Core Library
//! 
//! 提供核心的错误类型、设备ID定义和基础工具

pub mod error;
pub mod device_id;
pub mod types;
pub mod traits;
pub mod validation;
pub mod identity;

pub use error::{SyncthingError, Result};
pub use device_id::DeviceId;
pub use types::*;
pub use identity::{Identity, DeviceIdentity};
