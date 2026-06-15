//! 集中常量定义 — 所有可调参数和尺寸常量

use std::time::Duration;

// ── 帧循环 ──
pub const TICK_MS: u64 = 250;
pub const TICK_RATE: Duration = Duration::from_millis(TICK_MS);

// ── 日志缓冲 ──
pub const LOG_TAIL_LINES: usize = 50;
pub const LOG_BUFFER_CAP: usize = 100;

// ── 事件通道 ──
pub const EVENT_CHANNEL_CAP: usize = 256;

// ── Daemon 启动 ──
pub const CONFIG_WATCH_DEBOUNCE_MS: u64 = 500;
pub const RELAY_HEALTH_CHECK_CAP: usize = 10;

// ── 发现 / 中继 ──
pub const GLOBAL_DISCOVERY_INTERVAL_SECS: u64 = 300;
pub const RELAY_MAX_BACKOFF_SECS: u64 = 300;
pub const RELAY_BACKOFF_RESET_SECS: u64 = 60;

// ── 弹窗尺寸 ──
pub const HELP_POPUP_W: u16 = 70;
pub const HELP_POPUP_H: u16 = 22;
pub const ERROR_POPUP_W: u16 = 50;
pub const ERROR_POPUP_H: u16 = 12;
pub const CONFIRM_POPUP_W: u16 = 52;
pub const CONFIRM_POPUP_H: u16 = 12;
