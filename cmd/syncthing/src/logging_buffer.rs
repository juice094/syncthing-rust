use std::collections::VecDeque;
use std::sync::Arc;
use parking_lot::Mutex;
use tracing_subscriber::Layer;

/// 内存日志 Ring Buffer
#[derive(Clone)]
pub struct MemoryBuffer {
    inner: Arc<Mutex<VecDeque<String>>>,
    capacity: usize,
}

impl MemoryBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    pub fn take_lines(&self, n: usize) -> Vec<String> {
        let guard = self.inner.lock();
        guard.iter().rev().take(n).cloned().rev().collect()
    }

    pub fn push(&self, msg: String) {
        let mut guard = self.inner.lock();
        if guard.len() >= self.capacity {
            guard.pop_front();
        }
        guard.push_back(msg);
    }
}

/// tracing Layer 实现
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
            meta.target().split("::").last().unwrap_or(meta.target()),
            visitor.0
        );
        self.buffer.push(msg);
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
