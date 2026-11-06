//! Central constants for all tunable parameters and popup dimensions.

use std::time::Duration;

/// Frame loop period in milliseconds.
pub const TICK_MS: u64 = 250;
/// Frame loop rate.
pub const TICK_RATE: Duration = Duration::from_millis(TICK_MS);

/// Lines loaded from the daemon log at TUI startup.
pub const LOG_TAIL_LINES: usize = 50;
/// In-memory log buffer capacity.
pub const LOG_BUFFER_CAP: usize = 100;

/// TUI event channel capacity.
pub const EVENT_CHANNEL_CAP: usize = 256;

/// Config hot-reload debounce window.
pub const CONFIG_WATCH_DEBOUNCE_MS: u64 = 500;
/// How many relays get the deep TLS health check.
pub const RELAY_HEALTH_CHECK_CAP: usize = 10;

/// Global Discovery re-announce interval.
pub const GLOBAL_DISCOVERY_INTERVAL_SECS: u64 = 300;
/// Relay listener reconnect backoff ceiling.
pub const RELAY_MAX_BACKOFF_SECS: u64 = 300;
/// Run length that resets the relay backoff to its initial value.
pub const RELAY_BACKOFF_RESET_SECS: u64 = 60;

/// Help popup width.
pub const HELP_POPUP_W: u16 = 70;
/// Help popup height.
pub const HELP_POPUP_H: u16 = 22;
/// Error popup width.
pub const ERROR_POPUP_W: u16 = 50;
/// Error popup height.
pub const ERROR_POPUP_H: u16 = 12;
/// Confirm dialog width.
pub const CONFIRM_POPUP_W: u16 = 52;
/// Confirm dialog height.
pub const CONFIRM_POPUP_H: u16 = 12;
