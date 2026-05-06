use axum::{
    extract::State,
    response::{sse::Event, Sse},
    routing::get,
    Router,
};
use futures_util::stream::Stream;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::broadcast;
use tokio::time::{interval, Duration};

use crate::app_state::AppState;
use crate::sse::DashboardEvent;

/// Log broadcast channel — holds the last N log lines
pub struct LogBroadcaster {
    tx: broadcast::Sender<String>,
}

impl LogBroadcaster {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn broadcast(&self, line: String) {
        let _ = self.tx.send(line);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }
}

impl Default for LogBroadcaster {
    fn default() -> Self {
        Self::new(1000)
    }
}

/// SSE stream for real-time logs
pub struct LogStream {
    rx: broadcast::Receiver<DashboardEvent>,
    heartbeat: tokio::time::Interval,
}

impl Stream for LogStream {
    type Item = Result<Event, std::convert::Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.rx.try_recv() {
            Ok(event) => {
                if let Ok(evt) = event.to_sse_event() {
                    return Poll::Ready(Some(Ok(evt)));
                }
            }
            Err(broadcast::error::TryRecvError::Empty) => {}
            Err(_) => {
                return Poll::Ready(None);
            }
        }

        match Pin::new(&mut self.heartbeat).poll_tick(cx) {
            Poll::Ready(_) => {
                Poll::Ready(Some(Ok(Event::default().comment("heartbeat"))))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Handler for SSE log stream
pub async fn log_stream_handler(State(state): State<Arc<AppState>>) -> Sse<LogStream> {
    let rx = if let Some(ref broadcaster) = state.sse_broadcaster {
        broadcaster.subscribe()
    } else {
        let (_, rx) = broadcast::channel(1);
        rx
    };

    let stream = LogStream {
        rx,
        heartbeat: interval(Duration::from_secs(30)),
    };

    Sse::new(stream)
}

/// Add log stream route to router
pub fn add_log_stream_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router.route("/api/logs/stream", get(log_stream_handler))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_broadcaster() {
        let broadcaster = LogBroadcaster::new(10);
        let mut rx = broadcaster.subscribe();
        broadcaster.broadcast("test log".to_string());
        assert_eq!(rx.try_recv().unwrap(), "test log");
    }

    #[test]
    fn test_log_broadcaster_multi_subscribers() {
        let broadcaster = LogBroadcaster::new(10);
        let mut rx1 = broadcaster.subscribe();
        let mut rx2 = broadcaster.subscribe();
        broadcaster.broadcast("hello".to_string());
        assert_eq!(rx1.try_recv().unwrap(), "hello");
        assert_eq!(rx2.try_recv().unwrap(), "hello");
    }
}
