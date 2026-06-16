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
//!
//! Telemetry collector mode:
//!   syncthing-monitor --proc syncthing --log syncthing.log \
//!       --api-key `<KEY>` --folder-id default --folder-id share \
//!       --output metrics.csv --json metrics.jsonl

mod alerts;
mod api;
mod args;
mod format;
mod log_parser;
mod sample;
mod util;

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant, SystemTime};

use clap::Parser;
use sysinfo::{ProcessRefreshKind, RefreshKind, System};
use tracing::{info, warn};

use crate::alerts::check_alerts;
use crate::api::poll_api;
use crate::args::{parse_duration, Args, ProcId};
use crate::format::{
    append_text, build_csv_header, is_empty_or_missing, sample_to_csv, sample_to_json,
};
use crate::log_parser::{aggregate_log_metrics, LogParser};
use crate::sample::{
    sample_file_counts, sample_log_sizes, sample_processes, ApiSample, LogMtimeTracker, Sample,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt::init();

    let interval = parse_duration(&args.interval)?;
    let duration = args.duration.as_deref().map(parse_duration).transpose()?;
    let proc_ids: Vec<ProcId> = args.processes.iter().map(|s| ProcId::parse(s)).collect();

    info!(
        "Monitor starting: {} process(es), {} log file(s), interval={:?}, duration={:?}",
        proc_ids.len(),
        args.logs.len(),
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

    // Only write CSV header if the output file is missing or empty.
    if is_empty_or_missing(&args.output).await {
        let header = build_csv_header(proc_ids.len(), args.logs.len(), args.sync_dirs.len());
        append_text(&args.output, header).await?;
    }

    // Initialize log parsers and mtime trackers for silence detection.
    let mut log_parsers: Vec<LogParser> = args
        .logs
        .iter()
        .map(|p| LogParser::new(p.clone()))
        .collect();
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

    // Optional REST API client.
    let api_client = args.api_key.as_ref().map(|_| reqwest::Client::new());

    // Rolling RSS windows per process, used for predictive alerts.
    let mut rss_windows: Vec<VecDeque<u64>> =
        (0..proc_ids.len()).map(|_| VecDeque::new()).collect();

    // Previous per-folder needFiles for rising-backlog detection.
    let mut prev_need_files: HashMap<String, u64> = HashMap::new();

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

        // Parse log files for new performance signals.
        for parser in &mut log_parsers {
            parser.update().await;
        }
        let log_metrics = aggregate_log_metrics(&mut log_parsers);

        // Poll REST API if configured.
        let api = if let (Some(client), Some(api_key)) = (&api_client, args.api_key.as_ref()) {
            poll_api(client, &args.api_addr, api_key, &args.folder_id).await
        } else {
            ApiSample::default()
        };

        let sample = Sample {
            ts: SystemTime::now(),
            elapsed_secs,
            procs,
            log_sizes_mb,
            file_counts,
            api,
            log_metrics,
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

        // Alerts
        check_alerts(
            &sample,
            &args,
            &mut rss_windows,
            &mut prev_need_files,
            &mut log_trackers,
            interval,
        )
        .await;

        // Check shutdown signal (non-blocking)
        if shutdown_rx.try_recv().is_ok() {
            info!("Shutdown signal received");
            break;
        }
    }

    info!("Monitor stopped after {}s", start.elapsed().as_secs());
    Ok(())
}
