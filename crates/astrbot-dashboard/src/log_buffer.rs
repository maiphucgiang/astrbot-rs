use std::collections::VecDeque;
use std::sync::Mutex;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::fmt::format::Writer;

/// A single log entry
#[derive(Clone, Debug, serde::Serialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub target: String,
    pub message: String,
}

/// Ring buffer for recent log lines (thread-safe)
pub struct LogBuffer {
    inner: Mutex<VecDeque<LogEntry>>,
    capacity: usize,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    /// Push a new log entry, dropping oldest if at capacity
    pub fn push(&self, entry: LogEntry) {
        let mut guard = self.inner.lock().unwrap();
        if guard.len() >= self.capacity {
            guard.pop_front();
        }
        guard.push_back(entry);
    }

    /// Get recent entries in chronological order (oldest first)
    pub fn get_recent(&self, limit: usize) -> Vec<LogEntry> {
        let guard = self.inner.lock().unwrap();
        guard.iter().rev().take(limit).rev().cloned().collect()
    }

    /// Get total count of buffered entries
    pub fn len(&self) -> usize {
        let guard = self.inner.lock().unwrap();
        guard.len()
    }
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new(1000)
    }
}

/// Tracing Layer that captures log events into a LogBuffer
#[derive(Clone)]
pub struct LogCaptureLayer {
    buffer: std::sync::Arc<LogBuffer>,
}

impl LogCaptureLayer {
    pub fn new(buffer: std::sync::Arc<LogBuffer>) -> Self {
        Self { buffer }
    }
}

impl<S> Layer<S> for LogCaptureLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let level = metadata.level().to_string();
        let target = metadata.target().to_string();
        let timestamp = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string();

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        self.buffer.push(LogEntry {
            timestamp,
            level,
            target,
            message: visitor.message,
        });
    }
}

/// Visitor to extract the message from tracing fields
#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let val_str = format!("{:?}", value);
        // Trim surrounding quotes for string-like values
        let val_clean = val_str.trim_matches('"');

        if field.name() == "message" {
            self.message = val_clean.to_string();
        } else {
            if !self.message.is_empty() {
                self.message.push_str(" ");
            }
            self.message.push_str(&format!("{}={}", field.name(), val_clean));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            if !self.message.is_empty() {
                self.message.push_str(" ");
            }
            self.message.push_str(&format!("{}={}", field.name(), value));
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        if !self.message.is_empty() {
            self.message.push_str(" ");
        }
        self.message.push_str(&format!("{}={}", field.name(), value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        if !self.message.is_empty() {
            self.message.push_str(" ");
        }
        self.message.push_str(&format!("{}={}", field.name(), value));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        if !self.message.is_empty() {
            self.message.push_str(" ");
        }
        self.message.push_str(&format!("{}={}", field.name(), value));
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        if !self.message.is_empty() {
            self.message.push_str(" ");
        }
        self.message.push_str(&format!("{}={}", field.name(), value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_buffer_ring() {
        let buf = LogBuffer::new(3);
        buf.push(LogEntry {
            timestamp: "t1".to_string(),
            level: "INFO".to_string(),
            target: "test".to_string(),
            message: "msg1".to_string(),
        });
        buf.push(LogEntry {
            timestamp: "t2".to_string(),
            level: "INFO".to_string(),
            target: "test".to_string(),
            message: "msg2".to_string(),
        });
        buf.push(LogEntry {
            timestamp: "t3".to_string(),
            level: "INFO".to_string(),
            target: "test".to_string(),
            message: "msg3".to_string(),
        });
        buf.push(LogEntry {
            timestamp: "t4".to_string(),
            level: "INFO".to_string(),
            target: "test".to_string(),
            message: "msg4".to_string(),
        });

        let recent = buf.get_recent(10);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].message, "msg2");
        assert_eq!(recent[2].message, "msg4");
    }

    #[test]
    fn test_log_buffer_empty() {
        let buf = LogBuffer::new(100);
        assert_eq!(buf.len(), 0);
        assert!(buf.get_recent(10).is_empty());
    }
}
