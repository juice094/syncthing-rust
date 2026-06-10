//! 72h Stress Test Binary
//!
//! Usage:
//!   syncthing.exe stress-test --duration 72h --report report.csv
//!   syncthing.exe stress-test --duration 5m  --report quick.csv

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use clap::Parser;
use sha2::{Digest, Sha256};
use sysinfo::{ProcessRefreshKind, RefreshKind, System};
use tokio::io::AsyncWriteExt;
use tokio::time::{interval, interval_at};
use tracing::{info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// PID 文件管理：写入当前进程号，返回路径用于后续清理
async fn write_pid_file(path: &PathBuf) -> anyhow::Result<()> {
    let pid = std::process::id();
    tokio::fs::write(path, pid.to_string()).await?;
    Ok(())
}

async fn remove_pid_file(path: &PathBuf) {
    let _ = tokio::fs::remove_file(path).await;
}

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
    /// Resume from existing data directory (do not clean)
    #[arg(long)]
    resume: bool,
    /// PID file path for process management
    #[arg(long, default_value = "stress-test.pid")]
    pid_file: PathBuf,
    /// T2.2 — Directory for rotating log files (daily rotation, keep 7 days)
    #[arg(long, default_value = "stress-logs")]
    log_dir: PathBuf,
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
    format!(
        "{:02}h{:02}m{:02}s",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

fn fmt_system_time(t: SystemTime) -> String {
    // T2.2: Use chrono for proper ISO 8601 (RFC 3339) timestamp.
    // Previous impl emitted "days_since_epoch + HH:MM:SS" which broke CSV consumers.
    let dt: DateTime<Utc> = t.into();
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// T2.2: Initialize logging with rotating file appender + stdout.
///
/// - File: `<log_dir>/stress.log.YYYY-MM-DD` rotated daily, max 7 retained.
/// - Stdout: same content, useful for live tailing under nohup.
/// - Filter: respects `RUST_LOG` env var, defaults to INFO.
///
/// Returns the non-blocking worker guard which **must be held** for the duration of
/// `main()`; dropping it flushes and closes the appender.
fn init_logging(log_dir: &PathBuf) -> anyhow::Result<tracing_appender::non_blocking::WorkerGuard> {
    if let Err(e) = std::fs::create_dir_all(log_dir) {
        eprintln!(
            "Warning: cannot create log dir {:?}: {}. Logs will fall back to TMP.",
            log_dir, e
        );
    }
    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .max_log_files(7)
        .filename_prefix("stress")
        .filename_suffix("log")
        .build(log_dir)
        .unwrap_or_else(|e| {
            eprintln!(
                "Warning: cannot create rolling file appender: {}. Falling back to TMP/stress-fallback.",
                e
            );
            tracing_appender::rolling::daily(std::env::temp_dir(), "stress-fallback")
        });
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false);
    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stdout)
        .with_ansi(true);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer)
        .with(stdout_layer)
        .try_init()
        .map_err(|e| anyhow::anyhow!("Failed to set tracing subscriber: {}", e))?;

    Ok(guard)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // T2.2: replace `tracing_subscriber::fmt::init()` with rotating file appender
    // (daily rotation, keep 7 days) + stdout layer. The non-blocking worker guard
    // must outlive main() to flush buffered logs on exit.
    let _log_guard = init_logging(&args.log_dir)?;

    // T-F1 ENHANCEMENT: Panic hook to capture all unhandled panics
    let crash_log = std::env::current_dir()
        .unwrap_or_default()
        .join("stress-crash.log");
    std::panic::set_hook(Box::new(move |info| {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let msg = format!(
            "[ts={}] PANIC: {}\nbacktrace:\n{:?}\n\n",
            now,
            info,
            std::backtrace::Backtrace::force_capture()
        );
        eprintln!("{}", msg);
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&crash_log)
        {
            use std::io::Write;
            let _ = file.write_all(msg.as_bytes());
        }
    }));

    let duration = parse_duration(&args.duration)?;
    let inject_interval = parse_duration(&args.inject_interval)?;
    let fault_interval = parse_duration(&args.fault_interval)?;

    info!(
        "Stress test starting: duration={}, inject={}, fault={}, resume={}",
        fmt_duration(duration),
        fmt_duration(inject_interval),
        fmt_duration(fault_interval),
        args.resume
    );

    // PID file
    write_pid_file(&args.pid_file).await?;
    let pid_file = args.pid_file.clone();

    // Ctrl+C / graceful shutdown
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        if let Err(e) = tokio::signal::ctrl_c().await {
            warn!("Failed to listen for Ctrl+C: {}", e);
            return;
        }
        info!("Received Ctrl+C, requesting graceful shutdown...");
        let _ = shutdown_tx.send(());
    });

    // T-F1 ENHANCEMENT: Main task heartbeat — writes timestamp to file every 30s
    // If this file stops being updated while process appears dead, we know the main
    // tokio::select was alive but process was killed externally. If file is updated
    // recently but process is dead, that's a strong signal of TerminateProcess.
    let heartbeat_path = std::env::current_dir()
        .unwrap_or_default()
        .join("stress-heartbeat.log");
    let heartbeat_handle = tokio::spawn({
        let path = heartbeat_path.clone();
        async move {
            let mut counter: u64 = 0;
            let mut hb_ticker = interval(Duration::from_secs(30));
            loop {
                hb_ticker.tick().await;
                counter += 1;
                let now = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let line = format!("hb#{} ts={} pid={}\n", counter, now, std::process::id());
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                {
                    use std::io::Write;
                    let _ = file.write_all(line.as_bytes());
                    let _ = file.flush();
                }
            }
        }
    });

    if !args.resume {
        // Clean old data (fresh start)
        let _ = tokio::fs::remove_dir_all(&args.data_dir).await;
    } else {
        info!(
            "Resume mode: keeping existing data_dir {}",
            args.data_dir.display()
        );
    }

    let node_a_dir = args.data_dir.join("node-a");
    let node_b_dir = args.data_dir.join("node-b");

    let node_a: syncthing_test_utils::harness::TestNode =
        syncthing_test_utils::harness::TestNode::new_with_dir("a", node_a_dir.clone()).await?;
    let node_b: syncthing_test_utils::harness::TestNode =
        syncthing_test_utils::harness::TestNode::new_with_dir("b", node_b_dir.clone()).await?;

    let folder_id = "stress-folder";
    let folder_path_a = node_a_dir.join("sync");
    let folder_path_b = node_b_dir.join("sync");
    tokio::fs::create_dir_all(&folder_path_a).await?;
    tokio::fs::create_dir_all(&folder_path_b).await?;

    // Configure shared folder on both nodes
    if !args.resume {
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
    } else {
        info!("Resume mode: skipping folder reconfiguration");
    }

    // Connect peers
    node_a.connect_to(&node_b).await?;
    node_b.connect_to(&node_a).await?;
    node_a
        .wait_for_connection(node_b.device_id, Duration::from_secs(30))
        .await?;
    info!("Nodes connected, stress test active");

    if !args.resume {
        // Clean old reports
        let _ = tokio::fs::remove_file(&args.report).await;
        let _ = tokio::fs::remove_file(args.report.with_extension("metrics.csv")).await;
    } else {
        info!("Resume mode: appending to existing reports");
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
    let metrics_report = args.report.with_extension("metrics.csv");

    let monitor_task = tokio::spawn(async move {
        // T-F1 ENHANCEMENT: tick every 60s for long runs (was 600s) — needed for early-death visibility
        let tick_secs = if duration.as_secs() < 600 { 10 } else { 60 };
        let mut ticker = interval_at(
            tokio::time::Instant::now() + Duration::from_secs(5),
            Duration::from_secs(tick_secs),
        );
        // T-F1: memory sampling via spawn_blocking to avoid freezing tokio worker on Windows
        let sysinfo_task = move || {
            let mut sys = System::new_with_specifics(
                RefreshKind::new().with_processes(ProcessRefreshKind::new()),
            );
            sys.refresh_processes_specifics(ProcessRefreshKind::new());
            // T-F1 FIX: 二进制名是 stress_test（非 syncthing），之前 rss_mb 永远为 0
            sys.processes_by_exact_name("stress_test".as_ref())
                .map(|p| p.memory() / 1024 / 1024)
                .sum::<u64>()
        };
        // T-A3: metrics CSV header
        {
            let hdr = "timestamp,elapsed_secs,connected_a_b,connected_b_a,folder_state_a,folder_state_b,files_a,files_b,errors,rss_mb\n";
            if let Err(e) = append_to_file(&monitor_report, hdr.to_string()).await {
                warn!("Failed to write report header: {}", e);
            }
        }
        loop {
            ticker.tick().await;
            let elapsed = start.elapsed().as_secs();
            // T-F1 ENHANCEMENT: alive log per tick
            info!("monitor alive at T+{}s", elapsed);
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

            // Memory sampling (T-F1) - spawn_blocking for Windows stability
            let rss_mb = tokio::task::spawn_blocking(sysinfo_task).await.unwrap_or(0);

            let ts = fmt_system_time(SystemTime::now());
            let line = format!(
                "{},{},{},{},{},{},{},{},{},{}\n",
                ts,
                elapsed,
                connected_ab,
                connected_ba,
                state_a,
                state_b,
                files_a,
                files_b,
                errors,
                rss_mb
            );

            if let Err(e) = append_to_file(&monitor_report, line).await {
                warn!("Failed to write report: {}", e);
            }

            // T-A3: flush BEP metrics to CSV
            if let Err(e) = syncthing_net::metrics::global().flush_to_csv(&metrics_report) {
                warn!("Failed to flush metrics CSV: {}", e);
            }

            // Consistency check: every 5 ticks (~5 min) compare file hashes
            if elapsed.is_multiple_of(tick_secs * 5) {
                match compute_dir_hashes(&monitor_fa).await {
                    Ok(hashes_a) => match compute_dir_hashes(&monitor_fb).await {
                        Ok(hashes_b) => {
                            let (ok, mismatches) = compare_hashes("A", &hashes_a, "B", &hashes_b);
                            if ok {
                                info!(
                                    "consistency_check ok: {} files match between A and B",
                                    hashes_a.len()
                                );
                            } else {
                                let mismatch_count = mismatches.len();
                                warn!("consistency_check FAILED: {} mismatches", mismatch_count);
                                for m in &mismatches[..mismatch_count.min(5)] {
                                    warn!("  {}", m);
                                }
                                monitor_errors.fetch_add(mismatch_count as u64, Ordering::Relaxed);
                                let log_line = format!(
                                    "{},{},consistency_mismatch,{},{},{}\n",
                                    ts,
                                    elapsed,
                                    mismatch_count,
                                    hashes_a.len(),
                                    hashes_b.len()
                                );
                                let _ = append_to_file(
                                    &monitor_report.with_extension("consistency.csv"),
                                    log_line,
                                )
                                .await;
                            }
                        }
                        Err(e) => warn!("consistency_check: failed to hash B: {}", e),
                    },
                    Err(e) => warn!("consistency_check: failed to hash A: {}", e),
                }
            }
        }
    });

    // ── Load injection task ──
    let inject_path = folder_path_a.clone();
    let inject_errors = Arc::clone(&error_count);
    let inject_task = tokio::spawn(async move {
        let mut ticker = interval(inject_interval);
        let mut counter = 0u64;
        // T-F1: vary file sizes to stress block hashing and transfer
        let sizes: Vec<usize> = vec![1024, 64 * 1024, 1024 * 1024, 10 * 1024 * 1024];
        loop {
            ticker.tick().await;
            counter += 1;

            // Create with rotating size
            let size = sizes[(counter as usize) % sizes.len()];
            let file = inject_path.join(format!("file_{:04}.dat", counter));
            let data = vec![(counter % 256) as u8; size];
            if let Err(e) = tokio::fs::write(&file, &data).await {
                warn!("Inject create failed: {}", e);
                inject_errors.fetch_add(1, Ordering::Relaxed);
            }

            // Modify older file (same size class)
            if counter > 3 {
                let old = inject_path.join(format!("file_{:04}.dat", counter - 3));
                if old.exists() {
                    let size = sizes[((counter - 3) as usize) % sizes.len()];
                    let data = vec![((counter % 256) + 1) as u8; size];
                    if let Err(e) = tokio::fs::write(&old, &data).await {
                        warn!("Inject modify failed: {}", e);
                        inject_errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }

            // Delete oldest file
            if counter > 6 {
                let old = inject_path.join(format!("file_{:04}.dat", counter - 6));
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
            if let Err(e) = fault_handle
                .disconnect(&fault_peer, "stress fault injection")
                .await
            {
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
    tokio::select! {
        _ = tokio::time::sleep(duration) => {
            info!("Stress test duration reached");
        }
        _ = &mut shutdown_rx => {
            info!("Graceful shutdown requested");
        }
    }

    info!(
        "Stress test stopping after {}",
        fmt_duration(start.elapsed())
    );
    monitor_task.abort();
    inject_task.abort();
    fault_task.abort();
    heartbeat_handle.abort();

    // Final sync check: content hash consistency (not just file count)
    let final_a_count = count_files(&folder_path_a).await;
    let final_b_count = count_files(&folder_path_b).await;
    let total_errors = error_count.load(Ordering::Relaxed);

    let (consistent, mismatches) = match compute_dir_hashes(&folder_path_a).await {
        Ok(hashes_a) => match compute_dir_hashes(&folder_path_b).await {
            Ok(hashes_b) => {
                let (ok, mm) = compare_hashes("A", &hashes_a, "B", &hashes_b);
                info!(
                    "Final consistency: {} files in A, {} files in B, consistent={}",
                    hashes_a.len(),
                    hashes_b.len(),
                    ok
                );
                (ok, mm)
            }
            Err(e) => {
                warn!("Final consistency check failed for B: {}", e);
                (false, vec![format!("hash_B_error: {}", e)])
            }
        },
        Err(e) => {
            warn!("Final consistency check failed for A: {}", e);
            (false, vec![format!("hash_A_error: {}", e)])
        }
    };

    if !consistent {
        warn!(
            "FINAL CONSISTENCY CHECK FAILED — {} mismatches:",
            mismatches.len()
        );
        for m in &mismatches {
            warn!("  {}", m);
        }
    }

    info!(
        "Final state: files_a={}, files_b={}, errors={}, consistent={}",
        final_a_count, final_b_count, total_errors, consistent
    );

    node_a.shutdown().await;
    node_b.shutdown().await;

    // Cleanup PID file
    remove_pid_file(&pid_file).await;
    info!("Report: {}", args.report.display());
    Ok(())
}

async fn count_files(path: &PathBuf) -> usize {
    let mut count = 0;
    let Ok(mut entries) = tokio::fs::read_dir(path).await else {
        return 0;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry
            .file_type()
            .await
            .map(|t| t.is_file())
            .unwrap_or(false)
        {
            count += 1;
        }
    }
    count
}

/// 递归扫描目录，返回 (相对路径 → SHA-256 十六进制) 的映射。
/// 目录条目被跳过，仅计算普通文件。
async fn compute_dir_hashes(
    root: &std::path::Path,
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    let mut result = std::collections::HashMap::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(e) => {
                warn!("read_dir failed on {}: {}", dir.display(), e);
                continue;
            }
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let ft = match entry.file_type().await {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                let hash = match sha256_file(&path).await {
                    Ok(h) => h,
                    Err(e) => {
                        warn!("sha256 failed on {}: {}", path.display(), e);
                        continue;
                    }
                };
                result.insert(rel, hash);
            }
        }
    }
    Ok(result)
}

/// 计算单个文件的 SHA-256，在 spawn_blocking 中执行。
async fn sha256_file(path: &std::path::Path) -> anyhow::Result<String> {
    let path = path.to_path_buf();
    let bytes = tokio::fs::read(&path).await?;
    let hash = tokio::task::spawn_blocking(move || {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        hex::encode(hasher.finalize())
    })
    .await?;
    Ok(hash)
}

/// 比对两个目录的哈希映射，返回不一致信息。
fn compare_hashes(
    a_name: &str,
    a: &std::collections::HashMap<String, String>,
    b_name: &str,
    b: &std::collections::HashMap<String, String>,
) -> (bool, Vec<String>) {
    let mut mismatches = Vec::new();
    // A 中有但 B 中缺失或内容不同
    for (rel, hash_a) in a {
        match b.get(rel) {
            Some(hash_b) if hash_a == hash_b => {}
            Some(hash_b) => {
                mismatches.push(format!(
                    "content_mismatch: {} ({}={} {}={})",
                    rel,
                    a_name,
                    &hash_a[..8],
                    b_name,
                    &hash_b[..8]
                ));
            }
            None => {
                mismatches.push(format!("missing_in_{}: {}", b_name, rel));
            }
        }
    }
    // B 中有但 A 中缺失
    for rel in b.keys() {
        if !a.contains_key(rel) {
            mismatches.push(format!("missing_in_{}: {}", a_name, rel));
        }
    }
    let ok = mismatches.is_empty();
    (ok, mismatches)
}

async fn append_to_file(path: &PathBuf, line: String) -> anyhow::Result<()> {
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .write(true)
        .open(path)
        .await?;
    file.write_all(line.as_bytes()).await?;
    file.flush().await?;
    Ok(())
}
