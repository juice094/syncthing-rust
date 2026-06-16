use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use sysinfo::{ProcessRefreshKind, System};

use crate::args::ProcId;

/// Sample for one process.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProcSample {
    pub(crate) rss_mb: u64,
    pub(crate) cpu_percent: f32,
    pub(crate) found: bool,
}

/// Last-known mtime per log file, for silence detection.
#[derive(Debug)]
pub(crate) struct LogMtimeTracker {
    pub(crate) path: PathBuf,
    pub(crate) last_mtime: std::time::SystemTime,
    pub(crate) last_size: u64,
}

/// Polling result from the syncthing REST API.
#[derive(Debug, Default, Clone)]
pub(crate) struct ApiSample {
    pub(crate) connected: Option<bool>,
    pub(crate) connection_type: String,
    pub(crate) need_files: Option<u64>,
    pub(crate) need_bytes: Option<u64>,
    /// Per-folder needFiles, used for rising-backlog alerts.
    pub(crate) per_folder_need_files: HashMap<String, u64>,
}

/// Aggregated log-derived metrics for one sample window.
#[derive(Debug, Default, Clone)]
pub(crate) struct LogMetrics {
    pub(crate) scan_count: u64,
    pub(crate) pull_count: u64,
    pub(crate) pull_failed: u64,
    pub(crate) invalid_file_count: u64,
    pub(crate) scan_count_total: u64,
    pub(crate) pull_count_total: u64,
    pub(crate) pull_failed_total: u64,
    pub(crate) invalid_file_total: u64,
    pub(crate) scan_duration_ms: Option<u64>,
    pub(crate) pull_duration_ms: Option<u64>,
}

/// One row of the output timeseries.
#[derive(Debug)]
pub(crate) struct Sample {
    pub(crate) ts: SystemTime,
    pub(crate) elapsed_secs: u64,
    pub(crate) procs: Vec<ProcSample>,
    pub(crate) log_sizes_mb: Vec<u64>,
    pub(crate) file_counts: Vec<usize>,
    pub(crate) api: ApiSample,
    pub(crate) log_metrics: LogMetrics,
}

pub(crate) async fn sample_processes(
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

pub(crate) async fn sample_log_sizes(paths: &[PathBuf]) -> Vec<u64> {
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

pub(crate) async fn sample_file_counts(paths: &[PathBuf]) -> Vec<usize> {
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
