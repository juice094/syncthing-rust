//! Cross-platform process monitor for long-running syncthing stress tests.
//!
//! Replaces `scripts/72h_monitor.sh` (Linux-only `/proc/$PID` parsing) with a
//! portable Rust binary using the `sysinfo` crate.
//!
//! Usage:
//!   syncthing-monitor --proc 1234 --proc 5678 --log node_a.log --log node_b.log \
//!       --sync-dir sync/a --sync-dir sync/b --output metrics.csv
//!
//!   syncthing-monitor --proc syncthing --proc syncthing --interval 10s --duration 5m

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{bail, Context};
use clap::Parser;
use sysinfo::{ProcessRefreshKind, RefreshKind, System};
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(name = "syncthing-monitor")]
struct Args {
    /// Process identifiers to monitor (PID as number, or executable name).
    /// Repeat for each process. Names are matched exactly against the
    /// executable basename (no path, no extension).
    #[arg(long = "proc", required = true)]
    processes: Vec<String>,

    /// Log files to track size of (bytes → MB in output).
    #[arg(long = "log")]
    logs: Vec<PathBuf>,

    /// Sync directories to count files in.
    #[arg(long = "sync-dir")]
    sync_dirs: Vec<PathBuf>,

    /// Output CSV path.
    #[arg(long, default_value = "monitor.csv")]
    output: PathBuf,

    /// Optional JSON-lines output path (one JSON object per sample).
    #[arg(long)]
    json: Option<PathBuf>,

    /// Sampling interval, e.g. 10s, 1m, 60.
    #[arg(long, default_value = "60s")]
    interval: String,

    /// Total duration to run, e.g. 72h, 5m. Omit for infinite.
    #[arg(long)]
    duration: Option<String>,

    /// RSS alert threshold in MiB per process.
    #[arg(long, default_value = "512")]
    rss_alert_mb: u64,

    /// Log size alert threshold in MiB per log file.
    #[arg(long, default_value = "1024")]
    log_alert_mb: u64,

    /// Alert output path. Appends one line per threshold breach.
    #[arg(long, default_value = "alerts.log")]
    alerts: PathBuf,
    /// Log silence threshold: if a log file has not grown in this many seconds,
    /// emit an alert (indicates daemon may be hung).
    #[arg(long, default_value = "120")]
    log_silent_secs: u64,
}

/// Parsed representation of a process identifier.
enum ProcId {
    Pid(u32),
    Name(String),
}

impl ProcId {
    fn parse(s: &str) -> Self {
        if let Ok(pid) = s.parse::<u32>() {
            ProcId::Pid(pid)
        } else {
            ProcId::Name(s.to_owned())
        }
    }
}

fn parse_duration(s: &str) -> anyhow::Result<Duration> {
    let s = s.trim();
    if s.is_empty() {
        bail!("duration string is empty");
    }
    // Pure digits → seconds
    if s.chars().all(|c| c.is_ascii_digit()) {
        return Ok(Duration::from_secs(s.parse()?));
    }
    let num: u64 = s[..s.len() - 1]
        .parse()
        .with_context(|| format!("invalid duration number in {}", s))?;
    match &s[s.len() - 1..] {
        "s" => Ok(Duration::from_secs(num)),
        "m" => Ok(Duration::from_secs(num * 60)),
        "h" => Ok(Duration::from_secs(num * 3600)),
        "d" => Ok(Duration::from_secs(num * 86400)),
        u => bail!("invalid duration unit: {}", u),
    }
}

fn fmt_iso8601(t: SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Utc> = t.into();
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Sample for one process.
#[derive(Debug, Clone, Default)]
struct ProcSample {
    rss_mb: u64,
    cpu_percent: f32,
    found: bool,
}

/// Last-known mtime per log file, for silence detection.
#[derive(Debug)]
struct LogMtimeTracker {
    path: PathBuf,
    last_mtime: std::time::SystemTime,
    last_size: u64,
}

/// One row of the output timeseries.
#[derive(Debug)]
struct Sample {
    ts: SystemTime,
    elapsed_secs: u64,
    procs: Vec<ProcSample>,
    log_sizes_mb: Vec<u64>,
    file_counts: Vec<usize>,
}

async fn sample_processes(
    sys: &mut System,
    proc_ids: &[ProcId],
    cpu_settle: Duration,
) -> Vec<ProcSample> {
    sys.refresh_processes_specifics(ProcessRefreshKind::new().with_memory().with_cpu());
    // sysinfo needs a short delay between refreshes for accurate CPU %.
    tokio::time::sleep(cpu_settle).await;
    sys.refresh_processes_specifics(ProcessRefreshKind::new().with_memory().with_cpu());

    proc_ids
        .iter()
        .map(|id| match id {
            ProcId::Pid(pid) => sys
                .process(sysinfo::Pid::from_u32(*pid))
                .map(|p| ProcSample {
                    rss_mb: p.memory() / 1024 / 1024,
                    cpu_percent: p.cpu_usage(),
                    found: true,
                })
                .unwrap_or_default(),
            ProcId::Name(name) => {
                let mut sample = ProcSample::default();
                // Sum across all matching processes (there may be multiple).
                for p in sys.processes_by_exact_name(name.as_ref()) {
                    sample.rss_mb += p.memory() / 1024 / 1024;
                    sample.cpu_percent += p.cpu_usage();
                    sample.found = true;
                }
                sample
            }
        })
        .collect()
}

async fn sample_log_sizes(paths: &[PathBuf]) -> Vec<u64> {
    let mut sizes = Vec::with_capacity(paths.len());
    for path in paths {
        let mb = tokio::fs::metadata(path)
            .await
            .map(|m| m.len() / 1024 / 1024)
            .unwrap_or(0);
        sizes.push(mb);
    }
    sizes
}

async fn sample_file_counts(paths: &[PathBuf]) -> Vec<usize> {
    let mut counts = Vec::with_capacity(paths.len());
    for path in paths {
        let mut count = 0usize;
        if let Ok(mut entries) = tokio::fs::read_dir(path).await {
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
        }
        counts.push(count);
    }
    counts
}

fn build_csv_header(proc_count: usize, log_count: usize, sync_count: usize) -> String {
    let mut cols = vec!["timestamp".to_string(), "elapsed_secs".to_string()];
    for i in 0..proc_count {
        cols.push(format!("proc{}_rss_mb", i));
        cols.push(format!("proc{}_cpu", i));
    }
    for i in 0..log_count {
        cols.push(format!("log{}_mb", i));
    }
    for i in 0..sync_count {
        cols.push(format!("files{}_count", i));
    }
    cols.join(",") + "\n"
}

fn sample_to_csv(sample: &Sample) -> String {
    let mut parts = vec![fmt_iso8601(sample.ts), sample.elapsed_secs.to_string()];
    for p in &sample.procs {
        parts.push(if p.found {
            p.rss_mb.to_string()
        } else {
            "-1".to_string()
        });
        parts.push(if p.found {
            format!("{:.1}", p.cpu_percent)
        } else {
            "-1".to_string()
        });
    }
    for s in &sample.log_sizes_mb {
        parts.push(s.to_string());
    }
    for c in &sample.file_counts {
        parts.push(c.to_string());
    }
    parts.join(",") + "\n"
}

fn sample_to_json(sample: &Sample) -> anyhow::Result<String> {
    use serde_json::json;
    let procs: Vec<_> = sample
        .procs
        .iter()
        .map(|p| {
            json!({
                "rss_mb": if p.found { Some(p.rss_mb) } else { None },
                "cpu_percent": if p.found { Some(p.cpu_percent) } else { None },
                "found": p.found,
            })
        })
        .collect();
    let obj = json!({
        "timestamp": fmt_iso8601(sample.ts),
        "elapsed_secs": sample.elapsed_secs,
        "processes": procs,
        "log_sizes_mb": sample.log_sizes_mb,
        "file_counts": sample.file_counts,
    });
    Ok(format!("{}\n", serde_json::to_string(&obj)?))
}

async fn append_text(path: &PathBuf, text: String) -> anyhow::Result<()> {
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(text.as_bytes()).await?;
    file.flush().await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt::init();

    let interval = parse_duration(&args.interval)?;
    let duration = args.duration.as_deref().map(parse_duration).transpose()?;
    let proc_ids: Vec<ProcId> = args.processes.iter().map(|s| ProcId::parse(s)).collect();

    info!(
        "Monitor starting: {} process(es), interval={:?}, duration={:?}",
        proc_ids.len(),
        interval,
        duration
    );

    // Ctrl+C graceful shutdown
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = shutdown_tx.send(());
        }
    });

    let mut sys =
        System::new_with_specifics(RefreshKind::new().with_processes(ProcessRefreshKind::new()));

    let header = build_csv_header(proc_ids.len(), args.logs.len(), args.sync_dirs.len());
    append_text(&args.output, header).await?;

    // Initialize log mtime trackers for silence detection
    let mut log_trackers: Vec<LogMtimeTracker> = Vec::with_capacity(args.logs.len());
    for path in &args.logs {
        let (mtime, size) = tokio::fs::metadata(path)
            .await
            .map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()))
            .unwrap_or((SystemTime::UNIX_EPOCH, 0));
        log_trackers.push(LogMtimeTracker {
            path: path.clone(),
            last_mtime: mtime,
            last_size: size,
        });
    }

    let start = Instant::now();
    let mut ticker = tokio::time::interval(interval);

    loop {
        ticker.tick().await;

        // Check shutdown or duration
        if duration.map(|d| start.elapsed() >= d).unwrap_or(false) {
            info!("Duration reached, stopping monitor");
            break;
        }

        let elapsed_secs = start.elapsed().as_secs();
        let procs = sample_processes(&mut sys, &proc_ids, Duration::from_millis(500)).await;
        let log_sizes_mb = sample_log_sizes(&args.logs).await;
        let file_counts = sample_file_counts(&args.sync_dirs).await;

        let sample = Sample {
            ts: SystemTime::now(),
            elapsed_secs,
            procs,
            log_sizes_mb,
            file_counts,
        };

        // CSV
        if let Err(e) = append_text(&args.output, sample_to_csv(&sample)).await {
            warn!("Failed to append CSV: {}", e);
        }

        // JSON
        if let Some(ref json_path) = args.json {
            match sample_to_json(&sample) {
                Ok(line) => {
                    if let Err(e) = append_text(json_path, line).await {
                        warn!("Failed to append JSON: {}", e);
                    }
                }
                Err(e) => warn!("JSON serialization failed: {}", e),
            }
        }

        // ── Alerts ──
        // 1. Process crash / disappearance
        for (idx, p) in sample.procs.iter().enumerate() {
            if !p.found {
                let msg = format!(
                    "[ALERT] {} proc{} PROCESS NOT FOUND (crashed or exited)\n",
                    fmt_iso8601(sample.ts),
                    idx,
                );
                warn!("{}", msg.trim());
                if let Err(e) = append_text(&args.alerts, msg.clone()).await {
                    warn!("Failed to write alert: {}", e);
                }
            } else if p.rss_mb > args.rss_alert_mb {
                let msg = format!(
                    "[ALERT] {} proc{} RSS {}MiB > threshold {}MiB\n",
                    fmt_iso8601(sample.ts),
                    idx,
                    p.rss_mb,
                    args.rss_alert_mb
                );
                if let Err(e) = append_text(&args.alerts, msg).await {
                    warn!("Failed to write alert: {}", e);
                }
            }
        }

        // 2. Log size threshold
        for (idx, size_mb) in sample.log_sizes_mb.iter().enumerate() {
            if *size_mb > args.log_alert_mb {
                let msg = format!(
                    "[ALERT] {} log{} size {}MiB > threshold {}MiB\n",
                    fmt_iso8601(sample.ts),
                    idx,
                    size_mb,
                    args.log_alert_mb
                );
                if let Err(e) = append_text(&args.alerts, msg).await {
                    warn!("Failed to write alert: {}", e);
                }
            }
        }

        // 3. Log silence detection: if mtime hasn't changed AND size hasn't grown
        //    for log_silent_secs, the daemon may be hung.
        for (idx, tracker) in log_trackers.iter_mut().enumerate() {
            let current = tokio::fs::metadata(&tracker.path)
                .await
                .map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()))
                .unwrap_or((SystemTime::UNIX_EPOCH, 0));
            let (mtime, size) = current;
            if size > tracker.last_size || mtime > tracker.last_mtime {
                tracker.last_mtime = mtime;
                tracker.last_size = size;
            } else {
                let silent_secs = mtime
                    .duration_since(tracker.last_mtime)
                    .unwrap_or(Duration::ZERO)
                    .as_secs();
                if silent_secs >= args.log_silent_secs {
                    let msg = format!(
                        "[ALERT] {} log{} SILENT for {}s (no growth since {})\n",
                        fmt_iso8601(sample.ts),
                        idx,
                        silent_secs,
                        fmt_iso8601(tracker.last_mtime),
                    );
                    warn!("{}", msg.trim());
                    if let Err(e) = append_text(&args.alerts, msg.clone()).await {
                        warn!("Failed to write alert: {}", e);
                    }
                    // Reset tracker to avoid repeated alerts every tick
                    tracker.last_mtime = SystemTime::now();
                }
            }
        }

        // Check shutdown signal (non-blocking)
        if shutdown_rx.try_recv().is_ok() {
            info!("Shutdown signal received");
            break;
        }
    }

    info!("Monitor stopped after {}s", start.elapsed().as_secs());
    Ok(())
}
