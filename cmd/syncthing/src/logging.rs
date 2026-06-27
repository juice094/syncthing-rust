//! 日志初始化模块
//!
//! 提供 daemon、TUI、tray 三种运行模式的日志配置。
//! 支持 text（默认）和 json 两种输出格式。JSON 格式适合 ELK/Splunk/Loki 聚合。

use std::path::Path;

use tracing::Level;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Layer as _;

/// 日志输出格式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// 人类可读文本格式（默认）
    Text,
    /// 结构化 JSON 格式（适合日志聚合系统）
    Json,
}

impl LogFormat {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "json" => Self::Json,
            _ => Self::Text,
        }
    }
}

/// 构建 daemon 日志 layer（按小时轮转，保留 7 天）
fn build_daemon_writer(
    config_dir: &Path,
) -> (tracing_appender::non_blocking::NonBlocking, LevelFilter) {
    let logs_dir = config_dir.join("logs");
    if let Err(e) = std::fs::create_dir_all(&logs_dir) {
        eprintln!("Warning: cannot create logs dir: {}", e);
    }
    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::HOURLY)
        .max_log_files(168)
        .filename_prefix("daemon")
        .filename_suffix("log")
        .build(&logs_dir)
        .unwrap_or_else(|e| {
            eprintln!(
                "Warning: cannot create rolling file appender: {}. Falling back to TMP.",
                e
            );
            tracing_appender::rolling::hourly(std::env::temp_dir(), "syncthing-fallback")
        });
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    // SAFETY: daemon 生命周期内 _guard 持续有效
    std::mem::forget(_guard);
    (non_blocking, LevelFilter::INFO)
}

/// 初始化 daemon 模式日志（按小时轮转，保留 7 天）
pub fn init_daemon_logging(
    config_dir: &Path,
    log_level: Level,
    format: LogFormat,
) -> anyhow::Result<()> {
    let (non_blocking, _default_filter) = build_daemon_writer(config_dir);
    let level_filter = LevelFilter::from_level(log_level);

    let layer = match format {
        LogFormat::Json => tracing_subscriber::fmt::layer()
            .json()
            .with_writer(non_blocking)
            .with_filter(level_filter)
            .boxed(),
        LogFormat::Text => tracing_subscriber::fmt::layer()
            .with_writer(non_blocking)
            .with_filter(level_filter)
            .boxed(),
    };

    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|e| anyhow::anyhow!("Failed to set subscriber: {}", e))
}

/// 初始化 TUI 模式日志（按天轮转，保留 7 天）
pub fn init_tui_file_writer(
    config_dir: &Path,
) -> (
    tracing_appender::non_blocking::NonBlocking,
    tracing_appender::non_blocking::WorkerGuard,
) {
    let logs_dir = config_dir.join("logs");
    if let Err(e) = std::fs::create_dir_all(&logs_dir) {
        eprintln!("Cannot create TUI logs dir: {}", e);
    }

    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .max_log_files(7)
        .filename_prefix("tui")
        .filename_suffix("log")
        .build(&logs_dir)
        .unwrap_or_else(|e| {
            eprintln!("Failed to create TUI log appender: {}. Using temp dir.", e);
            tracing_appender::rolling::daily(std::env::temp_dir(), "syncthing-tui-fallback")
        });

    tracing_appender::non_blocking(file_appender)
}

/// 初始化 Windows tray 模式日志（按天轮转，forget guard）
#[cfg(all(windows, feature = "tray"))]
pub fn init_tray_logging(config_dir: &Path, format: LogFormat) {
    let logs_dir = config_dir.join("logs");
    if let Err(e) = std::fs::create_dir_all(&logs_dir) {
        eprintln!("Cannot create logs dir: {}", e);
        return;
    }

    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .max_log_files(7)
        .filename_prefix("tray")
        .filename_suffix(if format == LogFormat::Json {
            "json"
        } else {
            "log"
        })
        .build(&logs_dir)
        .unwrap_or_else(|e| {
            eprintln!("Failed to create tray log appender: {}. Using temp dir.", e);
            tracing_appender::rolling::daily(std::env::temp_dir(), "syncthing-tray-fallback")
        });

    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    std::mem::forget(_guard);

    let layer = match format {
        LogFormat::Json => tracing_subscriber::fmt::layer()
            .json()
            .with_writer(non_blocking)
            .boxed(),
        LogFormat::Text => tracing_subscriber::fmt::layer()
            .with_writer(non_blocking)
            .boxed(),
    };

    let subscriber = tracing_subscriber::registry()
        .with(layer.with_filter(tracing_subscriber::filter::LevelFilter::INFO));
    if let Err(e) = tracing::subscriber::set_global_default(subscriber) {
        eprintln!("Failed to set tray logger: {}", e);
    }
}
