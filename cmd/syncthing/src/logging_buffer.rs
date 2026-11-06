use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;
use tracing_subscriber::Layer;

/// One log entry with its level.
#[derive(Clone)]
pub struct LogEntry {
    pub msg: String,
    pub level: tracing::Level,
}

/// In-memory ring buffer of leveled log entries.
#[derive(Clone)]
pub struct MemoryBuffer {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
    capacity: usize,
}

impl MemoryBuffer {
    /// Creates a buffer holding the newest `capacity` entries.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    /// Appends an entry, evicting the oldest when full.
    pub fn push(&self, entry: LogEntry) {
        let mut guard = self.inner.lock();
        if guard.len() >= self.capacity {
            guard.pop_front();
        }
        guard.push_back(entry);
    }

    /// Returns all entries without filtering.
    #[allow(dead_code)]
    pub fn take_lines(&self, n: usize) -> Vec<LogEntry> {
        let guard = self.inner.lock();
        guard.iter().rev().take(n).cloned().rev().collect()
    }

    /// Returns the newest `max` entries at or above `min_level`.
    pub fn take_lines_filtered(&self, n: usize, min_level: &tracing::Level) -> Vec<LogEntry> {
        let guard = self.inner.lock();
        let filtered: Vec<_> = guard.iter().filter(|e| e.level >= *min_level).collect();
        filtered.into_iter().rev().take(n).cloned().rev().collect()
    }
}

/// tracing layer that records events into the memory buffer with their level.
pub struct MemoryLayer {
    buffer: MemoryBuffer,
}

impl MemoryLayer {
    /// Creates a layer recording into `buffer`.
    pub fn new(buffer: MemoryBuffer) -> Self {
        Self { buffer }
    }
}

impl<S> Layer<S> for MemoryLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        let meta = event.metadata();
        let msg = format!(
            "[{} {}] {}",
            meta.level(),
            meta.target()
                .split("::")
                .last()
                .unwrap_or_else(|| meta.target()),
            visitor.0
        );
        self.buffer.push(LogEntry {
            msg,
            level: *meta.level(),
        });
    }
}

struct MessageVisitor(String);

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{:?}", value).trim_matches('"').to_string();
        } else if self.0.is_empty() {
            self.0 = format!("{}={:?}", field.name(), value);
        } else {
            self.0.push_str(&format!(" {}={:?}", field.name(), value));
        }
    }
}
