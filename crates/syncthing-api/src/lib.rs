//! Syncthing API Library
//!
//! This crate provides REST API and WebSocket event functionality for Syncthing.
//! It includes configuration management, HTTP endpoints, and real-time event streaming.
//!
//! # Features
//!
//! - **REST API**: HTTP endpoints for managing folders, devices, and sync status
//! - **WebSocket Events**: Real-time event streaming for UI updates
//! - **Configuration Management**: JSON-based configuration storage
//!
//! # Example
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use syncthing_api::config::JsonConfigStore;
//! use syncthing_api::events::EventBus;
//! use syncthing_api::rest::{RestApi, ApiState};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create configuration store
//!     let config_store = Arc::new(JsonConfigStore::new("/path/to/config.json"));
//!
//!     // Create event bus
//!     let event_bus = EventBus::new();
//!
//!     // Create API state
//!     let state = ApiState::new(config_store, event_bus, None);
//!
//!     // Create and start API server
//!     let mut api = RestApi::new(state);
//!     api.bind(syncthing_core::constants::DEFAULT_API_ADDR.parse()?).await?;
//!     api.run().await?;
//!
//!     Ok(())
//! }
//! ```

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

pub mod config;
pub mod events;
pub mod handlers;
pub mod rest;

// Re-export commonly used types
pub use config::{JsonConfigStore, MemoryConfigStore};
pub use events::{EventBus, FilteredSubscriber, InstrumentedEventBus};
pub use rest::{ApiState, RestApi};

use syncthing_core::Result;

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Initialize the API module
///
/// This function initializes logging and other module-level resources.
pub fn init() -> Result<()> {
    // Module initialization if needed
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn test_init() {
        assert!(init().is_ok());
    }
}
