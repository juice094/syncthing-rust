//! Client for the official Syncthing Relay Protocol v1.
//!
//! Interoperates with Go Syncthing relay servers as the fallback path when
//! NAT-to-NAT direct connections are impossible.
//!
//! ## Protocol essentials
//! - Protocol Mode: TLS with `bep-relay` ALPN, used for registration and signaling
//! - Session Mode: plain TCP, used for end-to-end data passthrough (BEP TLS runs on top)
//!
//! ## Module layout
//! - `protocol`: XDR message definitions, codec, and async I/O helpers
//! - `client`: `RelayProtocolClient` (TLS signaling connection) + `join_session` (plain session connection)
//! - `types`: error types and Result alias

pub mod client;
pub mod dial;
pub mod pool;
pub mod protocol;
pub mod server;
pub mod types;

pub use client::{join_session, RelayProtocolClient};
pub use dial::connect_bep_via_relay;
pub use pool::{
    fetch_default_relay, fetch_relay_pool, filter_healthy_relays, filter_healthy_relays_tls,
    DEFAULT_RELAY_POOL_URL,
};
pub use protocol::{
    ConnectRequest, Header, JoinRelayRequest, JoinSessionRequest, Message, MessageType, Ping, Pong,
    RelayFull, Response, SessionInvitation, MAGIC,
};
pub use server::RelayServer;
pub use types::{RelayError, Result as RelayResult};
