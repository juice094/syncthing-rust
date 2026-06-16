use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tracing::warn;

use crate::sample::LogMetrics;

/// Parser state for a single log file.
#[derive(Debug)]
pub(crate) struct LogParser {
    pub(crate) path: PathBuf,
    /// Byte offset of the next unread data.
    pub(crate) offset: u64,
    /// Leftover bytes from a previous read that did not end with a newline.
    pub(crate) leftover: Vec<u8>,
    /// Active scan starts: folder_id → Instant.
    pub(crate) scan_starts: HashMap<String, Instant>,
    /// Active pull starts: folder_id → Instant.
    pub(crate) pull_starts: HashMap<String, Instant>,

    // Window counters (reset after each sample).
    pub(crate) scan_count: u64,
    pub(crate) pull_count: u64,
    pub(crate) pull_failed: u64,
    pub(crate) invalid_file_count: u64,
    pub(crate) scan_duration_ms: Option<u64>,
    pub(crate) pull_duration_ms: Option<u64>,

    // Lifetime cumulative counters.
    pub(crate) scan_count_total: u64,
    pub(crate) pull_count_total: u64,
    pub(crate) pull_failed_total: u64,
    pub(crate) invalid_file_total: u64,
}

impl LogParser {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            offset: 0,
            leftover: Vec::new(),
            scan_starts: HashMap::new(),
            pull_starts: HashMap::new(),
            scan_count: 0,
            pull_count: 0,
            pull_failed: 0,
            invalid_file_count: 0,
            scan_duration_ms: None,
            pull_duration_ms: None,
            scan_count_total: 0,
            pull_count_total: 0,
            pull_failed_total: 0,
            invalid_file_total: 0,
        }
    }

    pub(crate) async fn update(&mut self) {
        let meta = match tokio::fs::metadata(&self.path).await {
            Ok(m) => m,
            Err(_) => return,
        };
        let size = meta.len();
        if size < self.offset {
            // Log rotated/truncated: start over.
            self.offset = 0;
            self.leftover.clear();
        }
        if size == self.offset {
            return;
        }

        let mut file = match tokio::fs::File::open(&self.path).await {
            Ok(f) => f,
            Err(_) => return,
        };
        if let Err(e) = file.seek(std::io::SeekFrom::Start(self.offset)).await {
            warn!("seek {} failed: {}", self.path.display(), e);
            return;
        }

        let mut buf = Vec::with_capacity((size - self.offset) as usize + self.leftover.len());
        buf.extend_from_slice(&self.leftover);
        if let Err(e) = file.read_to_end(&mut buf).await {
            warn!("read {} failed: {}", self.path.display(), e);
            return;
        }

        // Only process complete lines; keep the trailing fragment for next time.
        let last_newline = buf.iter().rposition(|b| *b == b'\n');
        let consumed = match last_newline {
            Some(idx) => idx + 1,
            None => {
                self.leftover = buf;
                self.offset = size;
                return;
            }
        };

        let text = String::from_utf8_lossy(&buf[..consumed]);
        for line in text.lines() {
            self.process_line(line);
        }

        self.leftover = buf[consumed..].to_vec();
        // Unprocessed bytes are at the current end of the file.
        self.offset = size - self.leftover.len() as u64;
    }

    fn process_line(&mut self, line: &str) {
        // Folder scan completed
        if line.contains("Folder scan completed") {
            self.scan_count += 1;
            self.scan_count_total += 1;
            if let Some(folder_id) = extract_kv(line, "folder_id") {
                if let Some(start) = self.scan_starts.remove(&folder_id) {
                    self.scan_duration_ms = Some(start.elapsed().as_millis() as u64);
                }
            }
            return;
        }

        // Starting folder scan
        if line.contains("Starting folder scan") {
            if let Some(folder_id) = extract_kv(line, "folder_id") {
                self.scan_starts.insert(folder_id, Instant::now());
            }
            return;
        }

        // Folder / Pull completed
        if line.contains("Folder pull completed") || line.contains("Pull completed") {
            self.pull_count += 1;
            self.pull_count_total += 1;
            if let Some(folder_id) = extract_kv(line, "folder_id") {
                if let Some(start) = self.pull_starts.remove(&folder_id) {
                    self.pull_duration_ms = Some(start.elapsed().as_millis() as u64);
                }
            }
            // Try to extract succeeded/failed counts, e.g. "succeeded=10 failed=2".
            if let Some(failed) = extract_kv(line, "failed") {
                if let Ok(n) = failed.parse::<u64>() {
                    self.pull_failed += n;
                    self.pull_failed_total += n;
                }
            }
            return;
        }

        // Starting folder pull
        if line.contains("Starting folder pull") {
            if let Some(folder_id) = extract_kv(line, "folder_id") {
                self.pull_starts.insert(folder_id, Instant::now());
            }
            return;
        }

        // InvalidFile warnings
        if line.contains("InvalidFile") {
            self.invalid_file_count += 1;
            self.invalid_file_total += 1;
        }
    }

    /// Take the current window metrics and reset the window counters.
    pub(crate) fn take_metrics(&mut self) -> LogMetrics {
        let m = LogMetrics {
            scan_count: self.scan_count,
            pull_count: self.pull_count,
            pull_failed: self.pull_failed,
            invalid_file_count: self.invalid_file_count,
            scan_count_total: self.scan_count_total,
            pull_count_total: self.pull_count_total,
            pull_failed_total: self.pull_failed_total,
            invalid_file_total: self.invalid_file_total,
            scan_duration_ms: self.scan_duration_ms,
            pull_duration_ms: self.pull_duration_ms,
        };
        self.scan_count = 0;
        self.pull_count = 0;
        self.pull_failed = 0;
        self.invalid_file_count = 0;
        self.scan_duration_ms = None;
        self.pull_duration_ms = None;
        m
    }
}

/// Extract the value of a `key=value` token from a log line.
fn extract_kv(line: &str, key: &str) -> Option<String> {
    let pattern = format!("{}=", key);
    let start = line.find(&pattern)? + pattern.len();
    let rest = &line[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// Aggregate window metrics from all log parsers.
pub(crate) fn aggregate_log_metrics(parsers: &mut [LogParser]) -> LogMetrics {
    let mut agg = LogMetrics::default();
    for p in parsers {
        let m = p.take_metrics();
        agg.scan_count += m.scan_count;
        agg.pull_count += m.pull_count;
        agg.pull_failed += m.pull_failed;
        agg.invalid_file_count += m.invalid_file_count;
        agg.scan_count_total += m.scan_count_total;
        agg.pull_count_total += m.pull_count_total;
        agg.pull_failed_total += m.pull_failed_total;
        agg.invalid_file_total += m.invalid_file_total;
        if agg.scan_duration_ms.is_none() || m.scan_duration_ms.is_some() {
            agg.scan_duration_ms = m.scan_duration_ms;
        }
        if agg.pull_duration_ms.is_none() || m.pull_duration_ms.is_some() {
            agg.pull_duration_ms = m.pull_duration_ms;
        }
    }
    agg
}
