//! BEP (Block Exchange Protocol) Protocol Implementation
//!
//! 实现Syncthing BEP协议的Rust版本
//! 参考: syncthing/lib/protocol/*.go

pub mod handshake;
pub mod messages;
pub mod connection;

pub use handshake::{send_hello, recv_hello, exchange_hello, HELLO_MAGIC, MAX_HELLO_SIZE};
pub use messages::{
    Hello,
    WireVector, WireCounter, WireBlockInfo, WireFileInfo,
    Request, Response, Index, IndexUpdate, ClusterConfig, WireFolder,
    encode_message, decode_message,
};
pub use connection::BepConnection;

use syncthing_core::{Result, SyncthingError};

/// Re-export error types
pub use syncthing_core::{Result as BepResult, SyncthingError as BepError};
