use axum::{
    extract::State,
    response::{sse::Event, Sse},
    routing::get,
    Router,
};
use futures_util::stream::Stream;
use serde_json::json;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::{broadcast, RwLock};
use tokio::time::{interval, Duration};
use uuid::Uuid;

use crate::server::AppState;

/// Dashboard 实时事件类型
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "event_type")]
pub enum DashboardEvent {
    ProviderStatusChange {
        provider_id: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    PluginInstall {
        plugin_id: String,
        action: String,
        success: bool,
    },
    ConfigUpdate {
        updated_keys: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<String>,
    },
    SystemNotice {
        level: String,
        message: String,
    },
}

impl DashboardEvent {
    pub fn to_sse_event(&self) -> Result<Event, serde_json::Error> {
        let data = serde_json::to_string(self)?;
        Ok(Event::default().data(data))
    }
}

pub struct SseClient {
    pub id: String,
    pub rx: broadcast::Receiver<DashboardEvent>,
}

pub struct SseBroadcaster {
    tx: broadcast::Sender<DashboardEvent>,
    clients: Arc<RwLock<HashMap<String, ()>>>,
}

impl SseBroadcaster {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self {
            tx,
            clients: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn add_client(&self) -> SseClient {
        let id = Uuid::new_v4().to_string();
        let rx = self.tx.subscribe();
        let mut clients = self.clients.write().await;
        clients.insert(id.clone(), ());
        SseClient { id, rx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DashboardEvent> {
        self.tx.subscribe()
    }

    pub async fn remove_client(&self, client_id: &str) {
        let mut clients = self.clients.write().await;
        clients.remove(client_id);
    }

    pub async fn client_count(&self) -> usize {
        let clients = self.clients.read().await;
        clients.len()
    }

    pub fn broadcast_event(&self, event: DashboardEvent) {
        let _ = self.tx.send(event);
    }

    pub fn broadcast_provider_status(&self, provider_id: impl Into<String>, status: impl Into<String>, error: Option<String>) {
        self.broadcast_event(DashboardEvent::ProviderStatusChange {
            provider_id: provider_id.into(),
            status: status.into(),
            error,
        });
    }

    pub fn broadcast_plugin_install(&self, plugin_id: impl Into<String>, action: impl Into<String>, success: bool) {
        self.broadcast_event(DashboardEvent::PluginInstall {
            plugin_id: plugin_id.into(),
            action: action.into(),
            success,
        });
    }

    pub fn broadcast_config_update(&self, updated_keys: Vec<String>, source: Option<String>) {
        self.broadcast_event(DashboardEvent::ConfigUpdate {
            updated_keys,
            source,
        });
    }
}

impl Default for SseBroadcaster {
    fn default() -> Self {
        Self::new(128)
    }
}

pub struct DashboardEventStream {
    client_id: String,
    rx: broadcast::Receiver<DashboardEvent>,
    heartbeat: tokio::time::Interval,
    broadcaster: Arc<SseBroadcaster>,
}

impl Stream for DashboardEventStream {
    type Item = Result<Event, std::convert::Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.rx.try_recv() {
            Ok(event) => {
                if let Ok(evt) = event.to_sse_event() {
                    return Poll::Ready(Some(Ok(evt)));
                }
            }
            Err(broadcast::error::TryRecvError::Empty) => {}
            Err(broadcast::error::TryRecvError::Closed) => {
                return Poll::Ready(None);
            }
            Err(broadcast::error::TryRecvError::Lagged(_)) => {
                let evt = Event::default()
                    .data(json!({"event_type": "system_notice", "level": "warn", "message": "events lagged, some events may have been skipped"}).to_string());
                return Poll::Ready(Some(Ok(evt)));
            }
        }

        match Pin::new(&mut self.heartbeat).poll_tick(cx) {
            Poll::Ready(_) => Poll::Ready(Some(Ok(Event::default().comment("heartbeat")))),
            Poll::Pending => Poll::Pending,
        }
    }
}

pub async fn events_handler(State(state): State<AppState>) -> Sse<DashboardEventStream> {
    let (client_id, rx, broadcaster) = if let Some(ref b) = state.sse_broadcaster {
        let client = b.add_client().await;
        let broadcaster = Arc::clone(b);
        (client.id, client.rx, broadcaster)
    } else {
        let b = Arc::new(SseBroadcaster::new(1));
        let client = b.add_client().await;
        (client.id, client.rx, b)
    };

    let stream = DashboardEventStream {
        client_id,
        rx,
        heartbeat: interval(Duration::from_secs(30)),
        broadcaster,
    };

    Sse::new(stream)
}

pub fn add_sse_routes(router: Router<AppState>) -> Router<AppState> {
    router.route("/api/events", get(events_handler))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_broadcaster_add_remove_client() {
        let broadcaster = SseBroadcaster::new(10);
        let client = broadcaster.add_client().await;
        assert_eq!(broadcaster.client_count().await, 1);
        broadcaster.remove_client(&client.id).await;
        assert_eq!(broadcaster.client_count().await, 0);
    }

    #[tokio::test]
    async fn test_broadcast_reaches_client() {
        let broadcaster = SseBroadcaster::new(10);
        let client = broadcaster.add_client().await;
        let mut rx = client.rx;
        broadcaster.broadcast_event(DashboardEvent::SystemNotice {
            level: "info".to_string(),
            message: "hello sse".to_string(),
        });
        let received = rx.try_recv().expect("should receive event");
        match received {
            DashboardEvent::SystemNotice { level, message } => {
                assert_eq!(level, "info");
                assert_eq!(message, "hello sse");
            }
            _ => panic!("unexpected event type"),
        }
    }

    #[tokio::test]
    async fn test_broadcast_multi_clients() {
        let broadcaster = SseBroadcaster::new(10);
        let c1 = broadcaster.add_client().await;
        let c2 = broadcaster.add_client().await;
        let mut rx1 = c1.rx;
        let mut rx2 = c2.rx;
        broadcaster.broadcast_provider_status("openai", "connected", None);
        let e1 = rx1.try_recv().expect("client1 should receive");
        let e2 = rx2.try_recv().expect("client2 should receive");
        assert!(matches!(e1, DashboardEvent::ProviderStatusChange { .. }));
        assert!(matches!(e2, DashboardEvent::ProviderStatusChange { .. }));
    }

    #[tokio::test]
    async fn test_broadcast_plugin_install() {
        let broadcaster = SseBroadcaster::new(10);
        let client = broadcaster.add_client().await;
        let mut rx = client.rx;
        broadcaster.broadcast_plugin_install("weather", "install", true);
        let received = rx.try_recv().expect("should receive");
        match received {
            DashboardEvent::PluginInstall { plugin_id, action, success } => {
                assert_eq!(plugin_id, "weather");
                assert_eq!(action, "install");
                assert!(success);
            }
            _ => panic!("expected PluginInstall"),
        }
    }

    #[tokio::test]
    async fn test_broadcast_config_update() {
        let broadcaster = SseBroadcaster::new(10);
        let client = broadcaster.add_client().await;
        let mut rx = client.rx;
        broadcaster.broadcast_config_update(
            vec!["providers".to_string(), "plugins".to_string()],
            Some("dashboard".to_string()),
        );
        let received = rx.try_recv().expect("should receive");
        match received {
            DashboardEvent::ConfigUpdate { updated_keys, source } => {
                assert_eq!(updated_keys, vec!["providers", "plugins"]);
                assert_eq!(source, Some("dashboard".to_string()));
            }
            _ => panic!("expected ConfigUpdate"),
        }
    }

    #[tokio::test]
    async fn test_client_removed_does_not_receive() {
        let broadcaster = SseBroadcaster::new(10);
        let c1 = broadcaster.add_client().await;
        let c2 = broadcaster.add_client().await;
        let mut rx1 = c1.rx;
        let mut rx2 = c2.rx;
        broadcaster.remove_client(&c1.id).await;
        broadcaster.broadcast_event(DashboardEvent::SystemNotice {
            level: "warn".to_string(),
            message: "after remove".to_string(),
        });
        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
        assert_eq!(broadcaster.client_count().await, 1);
    }
}
