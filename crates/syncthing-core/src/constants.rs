//! Centralized constants for syncthing-rust.
//!
//! All hardcoded defaults must live here. No magic numbers in CLI parsing,
//! config defaults, or test assertions.

/// Default BEP listen port.
///
/// Historical note: Go Syncthing uses 22000. Rust implementation uses 22001
/// to avoid port collision when running side-by-side for interoperability testing.
pub const DEFAULT_BEP_PORT: u16 = 22001;

/// Default REST API port.
pub const DEFAULT_API_PORT: u16 = 8385;

/// Default listen address for BEP (all interfaces).
pub const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:22001";

/// Default API listen address (localhost only for security).
pub const DEFAULT_API_ADDR: &str = "127.0.0.1:8385";

/// Maximum concurrent dial attempts per device.
pub const MAX_PARALLEL_DIALS: usize = 3;

/// Default device name.
pub const DEFAULT_DEVICE_NAME: &str = "syncthing-rust";

/// Default folder scan interval (seconds).
pub const DEFAULT_SCAN_INTERVAL_SECS: u64 = 3600;

/// Default folder rescan interval for wizard/test configs (seconds).
pub const DEFAULT_WIZARD_SCAN_INTERVAL_SECS: u64 = 10;
