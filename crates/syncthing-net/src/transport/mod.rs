//! Transport layer implementations.
//!
//! Concrete implementations of the `Transport` trait:
//! - `RawTcpTransport`: raw TCP (no TLS, no BEP Hello)
//! - Future: `WebSocketTransport`, `QuicTransport`, `ProxiedTransport`
//!
//! Design principle: each Transport only establishes the raw byte pipe;
//! TLS/BEP are handled uniformly by the upper layer.
//!
//! **Note**: the `Transport` trait itself is defined in
//! `syncthing_core::traits`; this module only provides concrete
//! implementations and the registry.

pub mod bep_adapter;
pub mod keepalive;
pub mod proxy;
pub mod tcp;

#[cfg(feature = "websocket")]
pub mod websocket;

use std::sync::Arc;
use syncthing_core::Transport;

/// Transport registry supporting coexistence of multiple transports and
/// best-path selection.
///
/// Future extension: dynamically select the best transport by `PathQuality`
/// metrics (RTT, packet loss, bandwidth estimate).
pub struct TransportRegistry {
    transports: Vec<Arc<dyn Transport>>,
}

impl TransportRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            transports: Vec::new(),
        }
    }

    /// Registers a transport implementation.
    pub fn register(&mut self, transport: Arc<dyn Transport>) {
        self.transports.push(transport);
    }

    /// Looks up a transport by scheme.
    pub fn get(&self, scheme: &str) -> Option<Arc<dyn Transport>> {
        self.transports
            .iter()
            .find(|t| t.scheme() == scheme)
            .cloned()
    }

    /// Returns all registered schemes.
    pub fn schemes(&self) -> Vec<&'static str> {
        self.transports.iter().map(|t| t.scheme()).collect()
    }

    /// Selects the default transport (currently the first registered one).
    pub fn default_transport(&self) -> Option<Arc<dyn Transport>> {
        self.transports.first().cloned()
    }
}

impl Default for TransportRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub use tcp::RawTcpTransport;
