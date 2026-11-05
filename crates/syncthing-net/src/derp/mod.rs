//! DERP (Detoured Encrypted Routing Protocol) relay protocol.
//!
//! Minimal P2P relay protocol forwarding packets when NAT traversal fails.
//!
//! ## Core design
//! - Frame format: `[4 bytes len] [1 byte type] [payload]`
//! - After client registration the server forwards packets by device_id
//! - Supports TCP and WebSocket transports (WebSocket can masquerade as HTTPS)
//! - Supports relay chaining (Client -> Relay A -> Relay B -> Peer)
//!
//! ## Module layout
//! - `protocol`: frame format definition and codec
//! - `client`: DERP client (connects to relay, sends/receives packets)
//! - `server`: DERP server (accepts connections, forwards by device_id)

pub mod client;
pub mod pipe;
pub mod protocol;
pub mod server;
pub mod transport;

pub use client::{DerpClient, DerpClientConfig, DerpClientState};
pub use pipe::DerpPipe;
pub use protocol::{Frame, FrameType, MAX_FRAME_SIZE, PROTOCOL_VERSION};
pub use server::{DerpServer, DerpServerConfig};
pub use transport::DerpTransport;
