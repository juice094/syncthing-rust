use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;
use tracing_subscriber::Layer;

/// 带级别的日志条目
#[derive(Clone)]
pub struct LogEntry {
    pub msg: String,
    pub level: tracing::Level,
}

/// 内存日志 Ring Buffer（带级别）
#[derive(Clone)]
pub struct MemoryBuffer {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
    capacity: usize,
}

impl MemoryBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    pub fn push(&self, entry: LogEntry) {
        let mut guard = self.inner.lock();
        if guard.len() >= self.capacity {
            guard.pop_front();
        }
        guard.push_back(entry);
    }

    /// 返回全部条目（不过滤）
    #[allow(dead_code)]
    pub fn take_lines(&self, n: usize) -> Vec<LogEntry> {
        let guard = self.inner.lock();
        guard.iter().rev().take(n).cloned().rev().collect()
    }

    /// 按最低级别过滤后返回（最新 N 条）
    pub fn take_lines_filtered(&self, n: usize, min_level: &tracing::Level) -> Vec<LogEntry> {
        let guard = self.inner.lock();
        let filtered: Vec<_> = guard.iter().filter(|e| e.level >= *min_level).collect();
        filtered.into_iter().rev().take(n).cloned().rev().collect()
    }
}

/// tracing Layer 实现 —— 写入时带上 level
pub struct MemoryLayer {
    buffer: MemoryBuffer,
}

impl MemoryLayer {
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
