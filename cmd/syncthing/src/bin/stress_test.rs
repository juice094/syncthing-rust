//! 72h Stress Test Binary
//!
//! Usage:
//!   syncthing.exe stress-test --duration 72h --report report.csv
//!   syncthing.exe stress-test --duration 5m  --report quick.csv

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use clap::Parser;
use tokio::io::AsyncWriteExt;
use tokio::time::{interval, interval_at};
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(name = "stress-test")]
struct Args {
    /// Test duration, e.g. 72h, 5m, 30s
    #[arg(long, default_value = "72h")]
    duration: String,
    /// CSV report path
    #[arg(long, default_value = "stress-test-report.csv")]
    report: PathBuf,
    /// Data directory for persistent node state
    #[arg(long, default_value = "stress-test-data")]
    data_dir: PathBuf,
    /// File injection interval, e.g. 5m
    #[arg(long, default_value = "5m")]
    inject_interval: String,
    /// Network fault injection interval, e.g. 30m
    #[arg(long, default_value = "30m")]
    fault_interval: String,
}

fn parse_duration(s: &str) -> anyhow::Result<Duration> {
    if s.len() < 2 {
        anyhow::bail!("duration too short: {}", s);
    }
    let num: u64 = s[..s.len() - 1].parse()?;
    match &s[s.len() - 1..] {
        "s" => Ok(Duration::from_secs(num)),
        "m" => Ok(Duration::from_secs(num * 60)),
        "h" => Ok(Duration::from_secs(num * 3600)),
        "d" => Ok(Duration::from_secs(num * 86400)),
        u => anyhow::bail!("invalid duration unit: {}", u),
    }
}

fn fmt_duration(d: Duration) -> String {
    let secs = d.as_secs();
    format!("{:02}h{:02}m{:02}s", secs / 3600, (secs % 3600) / 60, secs % 60)
}

fn fmt_system_time(t: SystemTime) -> String {
    let dur = t.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs();
    let days = secs / 86400;
    let rem = secs % 86400;
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    format!("{}T{:02}:{:02}:{:02}Z", days, hh, mm, ss)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    let duration = parse_duration(&args.duration)?;
    let inject_interval = parse_duration(&args.inject_interval)?;
    let fault_interval = parse_duration(&args.fault_interval)?;

    info!(
        "Stress test starting: duration={}, inject={}, fault={}",
        fmt_duration(duration),
        fmt_duration(inject_interval),
        fmt_duration(fault_interval)
    );

    // Clean old data
    let _ = tokio::fs::remove_dir_all(&args.data_dir).await;

    let node_a_dir = args.data_dir.join("node-a");
    let node_b_dir = args.data_dir.join("node-b");

    let node_a: syncthing::test_harness::TestNode = syncthing::test_harness::TestNode::new_with_dir("a", node_a_dir.clone()).await?;
    let node_b: syncthing::test_harness::TestNode = syncthing::test_harness::TestNode::new_with_dir("b", node_b_dir.clone()).await?;

    let folder_id = "stress-folder";
    let folder_path_a = node_a_dir.join("sync");
    let folder_path_b = node_b_dir.join("sync");
    tokio::fs::create_dir_all(&folder_path_a).await?;
    tokio::fs::create_dir_all(&folder_path_b).await?;

    // Configure shared folder on both nodes
    node_a
        .add_folder(syncthing_core::types::Folder::new(
            folder_id,
            folder_path_a.to_string_lossy(),
        ))
        .await?;
    node_b
        .add_folder(syncthing_core::types::Folder::new(
            folder_id,
            folder_path_b.to_string_lossy(),
        ))
        .await?;

    // Connect peers
    node_a.connect_to(&node_b).await?;
    node_b.connect_to(&node_a).await?;
    node_a
        .wait_for_connection(node_b.device_id, Duration::from_secs(30))
        .await?;
    info!("Nodes connected, stress test active");

    // CSV header
    {
        let mut report = tokio::fs::File::create(&args.report).await?;
        report
            .write_all(b"timestamp,elapsed_secs,connected_a_b,connected_b_a,folder_state_a,folder_state_b,files_a,files_b,errors\n")
            .await?;
    }

    let start = Instant::now();
    let error_count = Arc::new(AtomicU64::new(0));

    // ── Monitor task ──
    let monitor_handle_a = node_a.connection_handle.clone();
    let monitor_handle_b = node_b.connection_handle.clone();
    let monitor_service_a = node_a.sync_service.clone();
    let monitor_service_b = node_b.sync_service.clone();
    let monitor_peer_b = node_b.device_id;
    let monitor_peer_a = node_a.device_id;
    let monitor_report = args.report.clone();
    let monitor_errors = Arc::clone(&error_count);
    let monitor_fa = folder_path_a.clone();
    let monitor_fb = folder_path_b.clone();

    let monitor_task = tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(600));
        loop {
            ticker.tick().await;
            let elapsed = start.elapsed().as_secs();
            let connected_ab = monitor_handle_a.get_connection(&monitor_peer_b).is_some();
            let connected_ba = monitor_handle_b.get_connection(&monitor_peer_a).is_some();

            let state_a = if monitor_service_a.get_folder(folder_id).is_some() {
                "present"
            } else {
                "missing"
            };
            let state_b = if monitor_service_b.get_folder(folder_id).is_some() {
                "present"
            } else {
                "missing"
            };

            let files_a = count_files(&monitor_fa).await;
            let files_b = count_files(&monitor_fb).await;
            let errors = monitor_errors.load(Ordering::Relaxed);

            let ts = fmt_system_time(SystemTime::now());
            let line = format!(
                "{},{},{},{},{},{},{},{},{}\n",
                ts, elapsed, connected_ab, connected_ba, state_a, state_b, files_a, files_b, errors
            );

            if let Err(e) = append_to_file(&monitor_report, line).await {
                warn!("Failed to write report: {}", e);
            }
        }
    });

    // ── Load injection task ──
    let inject_path = folder_path_a.clone();
    let inject_errors = Arc::clone(&error_count);
    let inject_task = tokio::spawn(async move {
        let mut ticker = interval(inject_interval);
        let mut counter = 0u64;
        loop {
            ticker.tick().await;
            counter += 1;

            // Create
            let file = inject_path.join(format!("file_{:04}.txt", counter));
            if let Err(e) = tokio::fs::write(&file, format!("content {}", counter)).await {
                warn!("Inject create failed: {}", e);
                inject_errors.fetch_add(1, Ordering::Relaxed);
            }

            // Modify older file
            if counter > 3 {
                let old = inject_path.join(format!("file_{:04}.txt", counter - 3));
                if old.exists() {
                    if let Err(e) = tokio::fs::write(&old, format!("modified {}", counter)).await {
                        warn!("Inject modify failed: {}", e);
                        inject_errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }

            // Delete oldest file
            if counter > 6 {
                let old = inject_path.join(format!("file_{:04}.txt", counter - 6));
                if old.exists() {
                    if let Err(e) = tokio::fs::remove_file(&old).await {
                        warn!("Inject delete failed: {}", e);
                        inject_errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    });

    // ── Fault injection task ──
    let fault_handle = node_a.connection_handle.clone();
    let fault_peer = node_b.device_id;
    let fault_addr = node_b.bep_addr;
    let fault_errors = Arc::clone(&error_count);
    let fault_task = tokio::spawn(async move {
        let mut ticker = interval_at(tokio::time::Instant::now() + fault_interval, fault_interval);
        loop {
            ticker.tick().await;
            info!("Fault injection: disconnecting");
            if let Err(e) = fault_handle.disconnect(&fault_peer, "stress fault injection").await {
                warn!("Fault disconnect failed: {}", e);
                fault_errors.fetch_add(1, Ordering::Relaxed);
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
            info!("Fault injection: reconnecting");
            if let Err(e) = fault_handle.connect_to(fault_peer, vec![fault_addr]).await {
                warn!("Fault reconnect failed: {}", e);
                fault_errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    });

    // ── Main timer ──
    tokio::time::sleep(duration).await;

    info!("Stress test completed after {}", fmt_duration(start.elapsed()));
    monitor_task.abort();
    inject_task.abort();
    fault_task.abort();

    // Final sync check
    let final_a = count_files(&folder_path_a).await;
    let final_b = count_files(&folder_path_b).await;
    let total_errors = error_count.load(Ordering::Relaxed);
    info!(
        "Final state: files_a={}, files_b={}, errors={}",
        final_a, final_b, total_errors
    );

    info!(
        "File counts: node_a={}, node_b={} (note: TestNode does not run full sync daemon)",
        final_a, final_b
    );

    node_a.shutdown().await;
    node_b.shutdown().await;
    info!("Report: {}", args.report.display());
    Ok(())
}

async fn count_files(path: &PathBuf) -> usize {
    let mut count = 0;
    let Ok(mut entries) = tokio::fs::read_dir(path).await else {
        return 0;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.file_type().await.map(|t| t.is_file()).unwrap_or(false) {
            count += 1;
        }
    }
    count
}

async fn append_to_file(path: &PathBuf, line: String) -> anyhow::Result<()> {
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .await?;
    file.write_all(line.as_bytes()).await?;
    Ok(())
}
