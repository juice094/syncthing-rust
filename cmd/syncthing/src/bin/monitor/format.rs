use std::path::PathBuf;

use serde_json::json;
use tokio::io::AsyncWriteExt;

use crate::sample::Sample;
use crate::util::fmt_iso8601;

pub(crate) fn build_csv_header(proc_count: usize, log_count: usize, sync_count: usize) -> String {
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
    cols.extend_from_slice(&[
        "connected".to_string(),
        "connection_type".to_string(),
        "need_files".to_string(),
        "need_bytes".to_string(),
        "scan_count".to_string(),
        "pull_count".to_string(),
        "pull_failed".to_string(),
        "invalid_file_count".to_string(),
        "scan_duration_ms".to_string(),
        "pull_duration_ms".to_string(),
        "scan_count_total".to_string(),
        "pull_count_total".to_string(),
        "pull_failed_total".to_string(),
        "invalid_file_total".to_string(),
    ]);
    cols.join(",") + "\n"
}

fn opt_u64_csv(v: Option<u64>) -> String {
    match v {
        Some(n) => n.to_string(),
        None => String::new(),
    }
}

fn opt_bool_csv(v: Option<bool>) -> String {
    match v {
        Some(true) => "true".to_string(),
        Some(false) => "false".to_string(),
        None => String::new(),
    }
}

fn opt_duration_csv(v: Option<u64>) -> String {
    match v {
        Some(n) => n.to_string(),
        None => "0".to_string(),
    }
}

pub(crate) fn sample_to_csv(sample: &Sample) -> String {
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
    parts.push(opt_bool_csv(sample.api.connected));
    parts.push(sample.api.connection_type.clone());
    parts.push(opt_u64_csv(sample.api.need_files));
    parts.push(opt_u64_csv(sample.api.need_bytes));
    parts.push(sample.log_metrics.scan_count.to_string());
    parts.push(sample.log_metrics.pull_count.to_string());
    parts.push(sample.log_metrics.pull_failed.to_string());
    parts.push(sample.log_metrics.invalid_file_count.to_string());
    parts.push(opt_duration_csv(sample.log_metrics.scan_duration_ms));
    parts.push(opt_duration_csv(sample.log_metrics.pull_duration_ms));
    parts.push(sample.log_metrics.scan_count_total.to_string());
    parts.push(sample.log_metrics.pull_count_total.to_string());
    parts.push(sample.log_metrics.pull_failed_total.to_string());
    parts.push(sample.log_metrics.invalid_file_total.to_string());
    parts.join(",") + "\n"
}

pub(crate) fn sample_to_json(sample: &Sample) -> anyhow::Result<String> {
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
        "connected": sample.api.connected,
        "connection_type": sample.api.connection_type,
        "need_files": sample.api.need_files,
        "need_bytes": sample.api.need_bytes,
        "scan_count": sample.log_metrics.scan_count,
        "pull_count": sample.log_metrics.pull_count,
        "pull_failed": sample.log_metrics.pull_failed,
        "invalid_file_count": sample.log_metrics.invalid_file_count,
        "scan_duration_ms": sample.log_metrics.scan_duration_ms,
        "pull_duration_ms": sample.log_metrics.pull_duration_ms,
        "scan_count_total": sample.log_metrics.scan_count_total,
        "pull_count_total": sample.log_metrics.pull_count_total,
        "pull_failed_total": sample.log_metrics.pull_failed_total,
        "invalid_file_total": sample.log_metrics.invalid_file_total,
    });
    Ok(format!("{}\n", serde_json::to_string(&obj)?))
}

pub(crate) async fn append_text(path: &PathBuf, text: String) -> anyhow::Result<()> {
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(text.as_bytes()).await?;
    file.flush().await?;
    Ok(())
}

/// True if the file does not exist or has zero length.
pub(crate) async fn is_empty_or_missing(path: &PathBuf) -> bool {
    match tokio::fs::metadata(path).await {
        Ok(m) => m.len() == 0,
        Err(_) => true,
    }
}
