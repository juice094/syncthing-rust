//! Internal health watchdog.
//!
//! Restarts the process when the daemon gets half-dead (REST API
//! unresponsive): two consecutive failed or timed-out checks of
//! `/rest/system/status` trigger a restart.
//!
//! Restart = spawn a copy of the current executable with the same arguments,
//! then trigger a graceful shutdown of the current process. If the graceful
//! shutdown stalls, the process force-exits after a short delay.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::{load_config, CONFIG_FILE_NAME};

/// Health-check interval.
const CHECK_INTERVAL: Duration = Duration::from_secs(30);
/// Per-check timeout.
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
/// Consecutive failures that trigger a restart.
const MAX_FAILURES: u32 = 2;
/// Max wait for the graceful shutdown after signalling it.
const GRACEFUL_SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

/// Starts the watchdog task.
///
/// `api_addr` is the actual REST API bind address; `config_dir` supplies the
/// API key; `shutdown_tx` triggers the current daemon's graceful shutdown.
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

    let _ = shutdown_tx.send(true);
    tokio::spawn(async move {
        tokio::time::sleep(GRACEFUL_SHUTDOWN_WAIT).await;
        error!("Watchdog forcing process exit after replacement spawn");
        std::process::exit(1);
    });
}
