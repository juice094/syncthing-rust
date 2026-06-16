use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "syncthing-monitor")]
pub(crate) struct Args {
    /// Process identifiers to monitor (PID as number, or executable name).
    /// Repeat for each process. Names are matched exactly against the
    /// executable basename (no path, no extension).
    #[arg(long = "proc", required = true)]
    pub(crate) processes: Vec<String>,

    /// Log files to track size of and parse for performance signals.
    #[arg(long = "log")]
    pub(crate) logs: Vec<PathBuf>,

    /// Sync directories to count files in.
    #[arg(long = "sync-dir")]
    pub(crate) sync_dirs: Vec<PathBuf>,

    /// Output CSV path.
    #[arg(long, default_value = "monitor.csv")]
    pub(crate) output: PathBuf,

    /// Optional JSON-lines output path (one JSON object per sample).
    #[arg(long)]
    pub(crate) json: Option<PathBuf>,

    /// Sampling interval, e.g. 10s, 1m, 60.
    #[arg(long, default_value = "60s")]
    pub(crate) interval: String,

    /// Total duration to run, e.g. 72h, 5m. Omit for infinite.
    #[arg(long)]
    pub(crate) duration: Option<String>,

    /// RSS alert threshold in MiB per process.
    #[arg(long, default_value = "512")]
    pub(crate) rss_alert_mb: u64,

    /// Log size alert threshold in MiB per log file.
    #[arg(long, default_value = "1024")]
    pub(crate) log_alert_mb: u64,

    /// Alert output path. Appends one line per threshold breach.
    #[arg(long, default_value = "alerts.log")]
    pub(crate) alerts: PathBuf,

    /// Log silence threshold: if a log file has not grown in this many seconds,
    /// emit an alert (indicates daemon may be hung).
    #[arg(long, default_value = "120")]
    pub(crate) log_silent_secs: u64,

    /// Optional REST API key. When provided, the monitor polls the syncthing
    /// REST API for connection and folder status.
    #[arg(long)]
    pub(crate) api_key: Option<String>,

    /// REST API base address (default: http://127.0.0.1:8385).
    #[arg(long, default_value = "http://127.0.0.1:8385")]
    pub(crate) api_addr: String,

    /// Folder IDs to poll via /rest/db/status. Repeatable.
    #[arg(long = "folder-id")]
    pub(crate) folder_id: Vec<String>,
}

/// Parsed representation of a process identifier.
pub(crate) enum ProcId {
    Pid(u32),
    Name(String),
}

impl ProcId {
    pub(crate) fn parse(s: &str) -> Self {
        if let Ok(pid) = s.parse::<u32>() {
            ProcId::Pid(pid)
        } else {
            ProcId::Name(s.to_owned())
        }
    }
}

pub(crate) fn parse_duration(s: &str) -> anyhow::Result<Duration> {
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
