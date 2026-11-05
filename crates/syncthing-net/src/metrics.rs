//! Lightweight in-memory metrics collection for syncthing-net.

use parking_lot::Mutex;
use std::io::Write;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Global metrics collector
static GLOBAL_METRICS: OnceLock<MetricsCollector> = OnceLock::new();

/// Get the global metrics collector
pub fn global() -> &'static MetricsCollector {
    GLOBAL_METRICS.get_or_init(MetricsCollector::new)
}

/// A single recorded metric event.
#[derive(Debug, Clone)]
pub struct MetricRecord {
    /// When the event was recorded.
    pub timestamp: Instant,
    /// Event name (e.g. `tls_handshake`, `bep_sent:<type>`).
    pub event: String,
    /// Device identifier associated with the event, if any.
    pub device_id: Option<String>,
    /// Event duration in milliseconds, if applicable.
    pub duration_ms: Option<u64>,
    /// Number of bytes transferred, if applicable.
    pub bytes: Option<u64>,
}

/// In-memory collector for network-layer metric events.
#[derive(Debug, Clone, Default)]
pub struct MetricsCollector {
    records: Arc<Mutex<Vec<MetricRecord>>>,
}

impl MetricsCollector {
    /// Creates an empty collector.
    pub fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Records a generic metric event.
    pub fn record(
        &self,
        event: impl Into<String>,
        device_id: Option<String>,
        duration: Option<Duration>,
        bytes: Option<u64>,
    ) {
        let rec = MetricRecord {
            timestamp: Instant::now(),
            event: event.into(),
            device_id,
            duration_ms: duration.map(|d| d.as_millis() as u64),
            bytes,
        };
        self.records.lock().push(rec);
    }

    /// Records the duration of a completed TLS handshake.
    pub fn record_tls_handshake(&self, duration: Duration) {
        self.record("tls_handshake", None, Some(duration), None);
    }

    /// Records a BEP message sent to a device.
    pub fn record_bep_message_sent(&self, device_id: String, msg_type: &str, bytes: u64) {
        self.record(
            format!("bep_sent:{}", msg_type),
            Some(device_id),
            None,
            Some(bytes),
        );
    }

    /// Records a BEP message received from a device.
    pub fn record_bep_message_recv(
        &self,
        device_id: String,
        msg_type: &str,
        latency: Duration,
        bytes: u64,
    ) {
        self.record(
            format!("bep_recv:{}", msg_type),
            Some(device_id),
            Some(latency),
            Some(bytes),
        );
    }

    /// Records a reconnection attempt to a device.
    pub fn record_reconnect(&self, device_id: String) {
        self.record("reconnect", Some(device_id), None, None);
    }

    /// Records the RTT of a single successful dial, for connection-quality analysis.
    pub fn record_dial_rtt(
        &self,
        device_id: String,
        addr: std::net::SocketAddr,
        rtt: Duration,
        scheme: &str,
    ) {
        self.record(
            format!("dial_rtt:{}:{}", scheme, addr),
            Some(device_id),
            Some(rtt),
            None,
        );
    }

    /// Writes all recorded metrics to a CSV file.
    ///
    /// # Errors
    /// Returns an I/O error if the output file cannot be created or written.
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

    /// Returns the number of recorded events.
    pub fn len(&self) -> usize {
        self.records.lock().len()
    }

    /// Returns `true` if no events have been recorded.
    pub fn is_empty(&self) -> bool {
        self.records.lock().is_empty()
    }
}
