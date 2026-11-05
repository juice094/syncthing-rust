//! BEP (Block Exchange Protocol) implementation.
//!
//! Rust implementation of the Syncthing BEP protocol; reference:
//! syncthing/lib/protocol/*.go.

pub mod connection;
pub mod handshake;
pub mod messages;

pub use connection::BepRawConnection;
pub use handshake::{exchange_hello, recv_hello, send_hello, HELLO_MAGIC, MAX_HELLO_SIZE};
pub use messages::{
    decode_message, encode_message, ClusterConfig, Hello, Index, IndexUpdate, Request, Response,
    WireBlockInfo, WireCounter, WireFileInfo, WireFolder, WireVector,
};

use syncthing_core::{Result, SyncthingError};

/// Re-exported error types.
pub use syncthing_core::{Result as BepResult, SyncthingError as BepError};
