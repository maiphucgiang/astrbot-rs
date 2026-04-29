use crate::errors::{AstrBotError, Result};
use async_trait::async_trait;
use tokio::sync::broadcast;
use tracing::{info, warn};

/// Server-Sent Events server for streaming responses
pub struct SseServer {
    tx: broadcast::Sender<SseEvent>,
}

/// SSE event payload
#[derive(Debug, Clone, serde::Serialize)]
pub struct SseEvent {
    pub id: String,
    pub event_type: String,
    pub data: String,
}

impl SseEvent {
    /// Create a new SSE event
    pub fn new(
        id: impl Into<String>,
        event_type: impl Into<String>,
        data: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            event_type: event_type.into(),
            data: data.into(),
        }
    }

    /// Create a message event
    pub fn message(data: impl Into<String>) -> Self {
        Self::new("", "message", data)
    }

    /// Create an error event
    pub fn error(data: impl Into<String>) -> Self {
        Self::new("", "error", data)
    }

    /// Create a ping event
    pub fn ping() -> Self {
        Self::new("", "ping", "")
    }
}

impl SseServer {
    /// Create a new SSE server with given channel capacity
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Subscribe to events
    pub fn subscribe(&self) -> broadcast::Receiver<SseEvent> {
        self.tx.subscribe()
    }

    /// Broadcast an event to all subscribers
    pub fn broadcast(&self, event: SseEvent) -> Result<usize> {
        self.tx
            .send(event)
            .map_err(|e| AstrBotError::Internal(format!("SSE broadcast failed: {}", e)))
    }

    /// Convenience: broadcast a message event
    pub fn broadcast_message(&self, data: impl Into<String>) -> Result<usize> {
        self.broadcast(SseEvent::message(data))
    }

    /// Get subscriber count
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse_server_creation() {
        let server = SseServer::new(16);
        assert_eq!(server.subscriber_count(), 0);
    }

    #[test]
    fn test_subscribe_and_broadcast() {
        let server = SseServer::new(16);
        let mut rx = server.subscribe();

        let count = server.broadcast_message("hello").unwrap();
        assert_eq!(count, 1);

        let event = rx.try_recv().unwrap();
        assert_eq!(event.event_type, "message");
        assert_eq!(event.data, "hello");
    }

    #[test]
    fn test_multiple_subscribers() {
        let server = SseServer::new(16);
        let mut rx1 = server.subscribe();
        let mut rx2 = server.subscribe();

        let count = server.broadcast(SseEvent::error("oops")).unwrap();
        assert_eq!(count, 2);

        let e1 = rx1.try_recv().unwrap();
        let e2 = rx2.try_recv().unwrap();
        assert_eq!(e1.data, "oops");
        assert_eq!(e2.data, "oops");
    }

    #[test]
    fn test_ping_event() {
        let event = SseEvent::ping();
        assert_eq!(event.event_type, "ping");
        assert_eq!(event.data, "");
    }
}
