//! Platform-specific path utilities for Syncthing.

use std::path::PathBuf;

/// Returns the default configuration directory for Syncthing.
///
/// - Windows: `%LOCALAPPDATA%\syncthing-rust`
/// - Linux/macOS: `~/.local/share/syncthing-rust`
/// - Fallback: `./.syncthing-rust`
pub fn default_config_dir() -> PathBuf {
    dirs::data_local_dir()
        .map(|d| d.join("syncthing-rust"))
        .unwrap_or_else(|| PathBuf::from(".syncthing-rust"))
}
