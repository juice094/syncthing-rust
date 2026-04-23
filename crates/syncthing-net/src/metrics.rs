//! Lightweight metrics collection for syncthing-net

use std::io::Write;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use parking_lot::Mutex;

/// Global metrics collector
static GLOBAL_METRICS: OnceLock<MetricsCollector> = OnceLock::new();

/// Get the global metrics collector
pub fn global() -> &'static MetricsCollector {
    GLOBAL_METRICS.get_or_init(MetricsCollector::new)
}

#[derive(Debug, Clone)]
pub struct MetricRecord {
    pub timestamp: Instant,
    pub event: String,
    pub device_id: Option<String>,
    pub duration_ms: Option<u64>,
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct MetricsCollector {
    records: Arc<Mutex<Vec<MetricRecord>>>,
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn record(&self, event: impl Into<String>, device_id: Option<String>, duration: Option<Duration>, bytes: Option<u64>) {
        let rec = MetricRecord {
            timestamp: Instant::now(),
            event: event.into(),
            device_id,
            duration_ms: duration.map(|d| d.as_millis() as u64),
            bytes,
        };
        self.records.lock().push(rec);
    }

    pub fn record_tls_handshake(&self, duration: Duration) {
        self.record("tls_handshake", None, Some(duration), None);
    }

    pub fn record_bep_message_sent(&self, device_id: String, msg_type: &str, bytes: u64) {
        self.record(format!("bep_sent:{}", msg_type), Some(device_id), None, Some(bytes));
    }

    pub fn record_bep_message_recv(&self, device_id: String, msg_type: &str, latency: Duration, bytes: u64) {
        self.record(format!("bep_recv:{}", msg_type), Some(device_id), Some(latency), Some(bytes));
    }

    pub fn record_reconnect(&self, device_id: String) {
        self.record("reconnect", Some(device_id), None, None);
    }

    pub fn flush_to_csv(&self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        let records = self.records.lock().clone();
        let mut file = std::fs::File::create(path)?;
        writeln!(file, "index,event,device_id,duration_ms,bytes")?;
        for (i, r) in records.iter().enumerate() {
            writeln!(
                file,
                "{},{},{},{},{}",
                i,
                r.event,
                r.device_id.as_deref().unwrap_or(""),
                r.duration_ms.map(|d| d.to_string()).unwrap_or_default(),
                r.bytes.map(|b| b.to_string()).unwrap_or_default(),
            )?;
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.records.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.lock().is_empty()
    }
}
