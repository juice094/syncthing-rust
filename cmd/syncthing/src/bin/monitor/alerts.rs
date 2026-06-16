use std::collections::HashMap;
use std::collections::VecDeque;
use std::time::{Duration, SystemTime};

use tracing::warn;

use crate::args::Args;
use crate::format::append_text;
use crate::sample::{LogMtimeTracker, Sample};
use crate::util::fmt_iso8601;

/// Compute slope and intercept for simple linear regression.
pub(crate) fn linear_regression(xs: &[f64], ys: &[f64]) -> Option<(f64, f64)> {
    let n = xs.len();
    if n < 2 || n != ys.len() {
        return None;
    }
    let n_f = n as f64;
    let mean_x = xs.iter().sum::<f64>() / n_f;
    let mean_y = ys.iter().sum::<f64>() / n_f;
    let mut num = 0.0;
    let mut den = 0.0;
    for (x, y) in xs.iter().zip(ys.iter()) {
        let dx = x - mean_x;
        num += dx * (y - mean_y);
        den += dx * dx;
    }
    if den == 0.0 {
        return None;
    }
    let slope = num / den;
    let intercept = mean_y - slope * mean_x;
    Some((slope, intercept))
}

pub(crate) async fn check_alerts(
    sample: &Sample,
    args: &Args,
    rss_windows: &mut [VecDeque<u64>],
    prev_need_files: &mut HashMap<String, u64>,
    log_trackers: &mut [LogMtimeTracker],
    interval: Duration,
) {
    // 1. Process crash / disappearance / RSS thresholds / predictive RSS.
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
        } else {
            // Immediate RSS alert.
            if p.rss_mb > args.rss_alert_mb {
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

            // Predictive RSS via linear regression over last 30 samples.
            let window = &mut rss_windows[idx];
            window.push_back(p.rss_mb);
            if window.len() > 60 {
                window.pop_front();
            }
            if window.len() >= 30 {
                let ys: Vec<f64> = window.iter().copied().map(|v| v as f64).collect();
                let xs: Vec<f64> = (0..ys.len()).map(|i| i as f64).collect();
                if let Some((slope, intercept)) = linear_regression(&xs, &ys) {
                    if slope > 0.0 {
                        let current_x = xs.last().copied().unwrap_or(0.0);
                        let threshold = args.rss_alert_mb as f64;
                        let samples_to_threshold = ((threshold - intercept) / slope) - current_x;
                        let samples_in_6h = (6.0 * 3600.0) / interval.as_secs_f64();
                        if samples_to_threshold > 0.0 && samples_to_threshold <= samples_in_6h {
                            let hours = samples_to_threshold * interval.as_secs_f64() / 3600.0;
                            let msg = format!(
                                "[PREDICT] {} proc{} RSS projected to exceed {}MiB within {:.1}h\n",
                                fmt_iso8601(sample.ts),
                                idx,
                                args.rss_alert_mb,
                                hours
                            );
                            warn!("{}", msg.trim());
                            if let Err(e) = append_text(&args.alerts, msg).await {
                                warn!("Failed to write alert: {}", e);
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. Sync backlog growing alert.
    for (folder_id, need) in &sample.api.per_folder_need_files {
        if *need > 0 {
            if let Some(prev) = prev_need_files.get(folder_id) {
                if *need > *prev {
                    let msg = format!(
                        "[PREDICT] {} Sync backlog growing: folder={} need_files={}\n",
                        fmt_iso8601(sample.ts),
                        folder_id,
                        need
                    );
                    warn!("{}", msg.trim());
                    if let Err(e) = append_text(&args.alerts, msg).await {
                        warn!("Failed to write alert: {}", e);
                    }
                }
            }
        }
        prev_need_files.insert(folder_id.clone(), *need);
    }

    // 3. Log size threshold
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

    // 4. Log silence detection: if mtime hasn't changed AND size hasn't grown
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
}
