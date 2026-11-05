//! Syncthing core library.
//!
//! Provides the core error types, device ID definitions, and the shared
//! traits and types used by all other crates.

pub mod constants;
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
