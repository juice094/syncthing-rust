//! Centralized constants for syncthing-rust.
//!
//! All hardcoded defaults must live here. No magic numbers in CLI parsing,
//! config defaults, or test assertions.
//!
//! See "docs/design/ADR-20261105-pull-delete-quota.md".

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

/// Default GUI (REST API) listen address.
///
/// Local-first: binds 127.0.0.1 only to prevent unauthorized remote access.
/// For remote access the user must configure it explicitly or pass
/// `--allow-remote` at startup.
pub const DEFAULT_GUI_ADDR: &str = "127.0.0.1:8385";

/// Maximum concurrent dial attempts per device.
pub const MAX_PARALLEL_DIALS: usize = 3;

/// Default device name.
pub const DEFAULT_DEVICE_NAME: &str = "syncthing-rust";

/// Default folder scan interval (seconds).
pub const DEFAULT_SCAN_INTERVAL_SECS: u64 = 3600;

/// Default folder rescan interval for wizard/test configs (seconds).
pub const DEFAULT_WIZARD_SCAN_INTERVAL_SECS: u64 = 10;

/// Default file block size for hashing and transfer.
pub const DEFAULT_BLOCK_SIZE: i32 = 128 * 1024;

/// Floor for the per-pull delete quota: at least this many remote deletions
/// are always permitted, so small folders are not falsely flagged by the
/// ratio quota.
pub const PULL_DELETE_QUOTA_MIN: usize = 20;

/// Ratio leg of the per-pull delete quota. A pull whose remote deletion count
/// exceeds `max(PULL_DELETE_QUOTA_MIN, local_file_count * ratio)` is rejected
/// as an anomalous mass deletion (downloads/updates are unaffected).
pub const PULL_DELETE_QUOTA_RATIO: f64 = 0.25;

/// Health-warning threshold for a stale peer index: more BEP error code 2
/// (NO_SUCH_FILE) responses than this in one pull triggers a health warning.
pub const STALE_INDEX_WARN_THRESHOLD: usize = 10;

/// First retry backoff after a pull failure (milliseconds). Transient
/// failures such as block request timeouts usually recover sub-second, so
/// there is no need to wait for the next periodic tick (see
/// `docs/KNOWN_ISSUES.md` §23).
pub const PULL_RETRY_BACKOFF_INIT_MS: u64 = 500;

/// Upper bound of the pull retry backoff (milliseconds). Aligned with the
/// periodic tick so steady-state retries never exceed one attempt per
/// `PULL_INTERVAL_SECS`.
pub const PULL_RETRY_BACKOFF_MAX_MS: u64 = PULL_INTERVAL_SECS * 1000;

/// Folder pull loop period (seconds): fallback trigger interval when no
/// remote index notification arrives.
pub const PULL_INTERVAL_SECS: u64 = 10;
