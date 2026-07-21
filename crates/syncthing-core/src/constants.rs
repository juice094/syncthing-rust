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

/// Default GUI (REST API) listen address.
///
/// 本地优先：默认仅绑定 127.0.0.1，避免未授权远程访问。
/// 如需远程访问，用户应显式配置或启动时传入 `--allow-remote`。
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

/// 单次 pull 删除配额下限：一次 pull 循环允许应用的远程删除数至少放行此值，
/// 避免小文件夹（如共 10 个文件）因比例配额误伤正常删除。
pub const PULL_DELETE_QUOTA_MIN: usize = 20;

/// 单次 pull 删除配额比例：删除数超过 `max(PULL_DELETE_QUOTA_MIN, 本地文件数 * 比例)`
/// 时判定为异常批量删除并拒绝整个删除批次（下载/修改动作不受影响）。
/// 防止陈旧/损坏的对端索引静默清空本地（2026-07-20 事故，114 文件被删）。
pub const PULL_DELETE_QUOTA_RATIO: f64 = 0.25;

/// 陈旧索引告警阈值：单次 pull 中 BEP error code 2（NO_SUCH_FILE）响应数超过此值时，
/// 判定对端索引可能陈旧/损坏并输出健康告警（2026-07-20 事故前兆）。
pub const STALE_INDEX_WARN_THRESHOLD: usize = 10;
