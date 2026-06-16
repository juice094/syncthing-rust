//! 内部健康看门狗
//!
//! 目标：在 daemon 进入“API 无响应”等半死状态时自动重启进程，实现自愈。
//! 触发条件：连续两次对 REST API `/rest/system/status` 的健康检查失败或超时。
//!
//! 重启方式：使用当前可执行文件路径和命令行参数重新 spawn 一个新进程，随后
//! 触发当前进程的优雅关闭。若优雅关闭卡住，当前进程将在短暂延迟后强制退出。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::{load_config, CONFIG_FILE_NAME};

/// 健康检查间隔
const CHECK_INTERVAL: Duration = Duration::from_secs(30);
/// 单次健康检查超时
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
/// 连续失败次数阈值，达到后触发重启
const MAX_FAILURES: u32 = 2;
/// 发送关闭信号后，等待优雅关闭的最长时间
const GRACEFUL_SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

/// 启动看门狗任务。
///
/// `api_addr` 是 REST API 实际绑定的地址；`config_dir` 用于读取 API key；
/// `shutdown_tx` 用于触发当前 daemon 的优雅关闭。
pub fn spawn(
    api_addr: SocketAddr,
    config_dir: PathBuf,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(e) = run(api_addr, config_dir, shutdown_tx).await {
            warn!("Internal watchdog exited unexpectedly: {}", e);
        }
    })
}

async fn run(
    api_addr: SocketAddr,
    config_dir: PathBuf,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
) -> anyhow::Result<()> {
    let api_key = load_api_key(&config_dir).await.unwrap_or_default();
    if api_key.is_empty() {
        warn!("No API key available, internal watchdog disabled");
        return Ok(());
    }

    let client = reqwest::Client::builder()
        .timeout(CHECK_TIMEOUT)
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build watchdog HTTP client: {}", e))?;

    let url = format!("http://127.0.0.1:{}/rest/system/status", api_addr.port());
    let mut failures = 0u32;
    let mut interval = tokio::time::interval(CHECK_INTERVAL);

    info!(
        "Internal watchdog started, checking {} every {:?}",
        url, CHECK_INTERVAL
    );

    loop {
        interval.tick().await;

        match client.get(&url).header("X-API-Key", &api_key).send().await {
            Ok(resp) if resp.status().is_success() => {
                if failures > 0 {
                    info!("Watchdog health check recovered");
                }
                failures = 0;
            }
            Ok(resp) => {
                warn!(
                    "Watchdog health check returned non-success status: {}",
                    resp.status()
                );
                failures += 1;
            }
            Err(e) => {
                warn!("Watchdog health check failed: {}", e);
                failures += 1;
            }
        }

        if failures >= MAX_FAILURES {
            error!(
                "Watchdog detected daemon unresponsive ({} consecutive failures); restarting process",
                failures
            );
            restart_process(shutdown_tx);
            break;
        }
    }

    Ok(())
}

async fn load_api_key(config_dir: &std::path::Path) -> anyhow::Result<String> {
    let config_path = config_dir.join(CONFIG_FILE_NAME);
    let config = load_config(&config_path)?;
    Ok(config.gui.api_key)
}

fn restart_process(shutdown_tx: tokio::sync::watch::Sender<bool>) {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            error!("Watchdog failed to get current executable: {}", e);
            return;
        }
    };
    let args: Vec<String> = std::env::args().skip(1).collect();

    match std::process::Command::new(&exe).args(&args).spawn() {
        Ok(child) => {
            info!(
                "Watchdog spawned replacement process (pid={:?}) from {:?}",
                child.id(),
                exe
            );
        }
        Err(e) => {
            error!("Watchdog failed to spawn replacement process: {}", e);
            return;
        }
    }

    // 触发当前进程的优雅关闭；如果卡住，在短暂等待后强制退出。
    let _ = shutdown_tx.send(true);
    tokio::spawn(async move {
        tokio::time::sleep(GRACEFUL_SHUTDOWN_WAIT).await;
        error!("Watchdog forcing process exit after replacement spawn");
        std::process::exit(1);
    });
}
