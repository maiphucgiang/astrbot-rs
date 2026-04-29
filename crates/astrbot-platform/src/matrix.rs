use async_trait::async_trait;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::message::{AstrBotMessage, MessageChain, MessageComponent, MessageHandler, HandlerRef, MessageMember, MessageType};
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use astrbot_core::net::SharedHttpClient;
use crate::adapter::PlatformAdapter;
use axum::{routing::post, Router};
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// Matrix API data models
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct MatrixSendMessageRequest {
    #[serde(rename = "msgtype")]
    msgtype: String,
    body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "format")]
    format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "formatted_body")]
    formatted_body: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MatrixWebhookPayload {
    #[serde(default)]
    pub room_id: Option<String>,
    #[serde(default)]
    pub sender: Option<String>,
    #[serde(default)]
    pub content: Option<MatrixMessageContent>,
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(default)]
    pub origin_server_ts: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct MatrixMessageContent {
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default, rename = "msgtype")]
    pub msgtype: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub info: Option<serde_json::Value>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default, rename = "formatted_body")]
    pub formatted_body: Option<String>,
}

// /sync response models
#[derive(Debug, Clone, Deserialize)]
struct SyncResponse {
    #[serde(default)]
    next_batch: String,
    #[serde(default)]
    rooms: Option<SyncRooms>,
}

#[derive(Debug, Clone, Deserialize)]
struct SyncRooms {
    #[serde(default)]
    join: HashMap<String, SyncJoinedRoom>,
}

#[derive(Debug, Clone, Deserialize)]
struct SyncJoinedRoom {
    #[serde(default)]
    timeline: Option<SyncTimeline>,
}

#[derive(Debug, Clone, Deserialize)]
struct SyncTimeline {
    #[serde(default)]
    events: Vec<SyncEvent>,
    #[serde(default)]
    prev_batch: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SyncEvent {
    #[serde(rename = "event_id")]
    event_id: String,
    #[serde(rename = "type")]
    event_type: String,
    sender: String,
    #[serde(default)]
    content: serde_json::Value,
    #[serde(default)]
    #[serde(rename = "origin_server_ts")]
    origin_server_ts: Option<i64>,
}

// ---------------------------------------------------------------------------
// Matrix shared state
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct MatrixShared {
    homeserver: String,
    access_token: String,
    user_id: String,
    http_client: reqwest::Client,
    message_handler: Arc<RwLock<HandlerRef>>,
}

impl MatrixShared {
    async fn send_matrix_message(
        &self,
        room_id: &str,
        chain: &MessageChain,
    ) -> Result<()> {
        let txn_id = format!("astrbot-{}", Utc::now().timestamp_millis());
        let url = format!(
            "{}/_matrix/client/v3/rooms/{}/send/m.room.message/{}",
            self.homeserver,
            room_id,
            txn_id
        );

        let text = chain.plain_text();
        if text.is_empty() {
            return Ok(());
        }

        // Check if chain contains HTML-formatted content
        let (body, formatted, format_str) = self.build_matrix_content(chain, &text);

        let body_req = MatrixSendMessageRequest {
            msgtype: "m.text".to_string(),
            body,
            format: format_str,
            formatted_body: formatted,
        };

        let resp = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .json(&body_req)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Matrix send message failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "Matrix".to_string(),
                message: format!("Matrix API error: {} - {}", status, text),
            });
        }

        info!("[Matrix] Message sent successfully");
        Ok(())
    }

    fn build_matrix_content(&self, chain: &MessageChain, plain: &str) -> (String, Option<String>, Option<String>) {
        let mut has_html = false;
        let mut html_parts: Vec<String> = Vec::new();

        for comp in &chain.0 {
            match comp {
                MessageComponent::Plain { text } => {
                    html_parts.push(html_escape(text));
                }
                MessageComponent::At { target, display } => {
                    has_html = true;
                    let disp = display.as_deref().unwrap_or(target.as_str());
                    html_parts.push(format!("<a href=\"https://matrix.to/#/{0}\">{1}</a>",
                        html_escape(target), html_escape(disp)));
                }
                MessageComponent::Image { url, .. } => {
                    if let Some(u) = url {
                        has_html = true;
                        html_parts.push(format!("<img src=\"{}\" />", html_escape(u)));
                    }
                }
                MessageComponent::Reply { message_id, .. } => {
                    has_html = true;
                    html_parts.push(format!("<mx-reply><blockquote>In reply to <a href=\"#\">{0}</a></blockquote></mx-reply>",
                        html_escape(message_id)));
                }
                _ => {}
            }
        }

        if has_html {
            (plain.to_string(), Some(html_parts.concat()), Some("org.matrix.custom.html".to_string()))
        } else {
            (plain.to_string(), None, None)
        }
    }

    fn parse_webhook_payload(&self, payload: MatrixWebhookPayload) -> Option<AstrBotMessage> {
        let content = payload.content?;
        let body = content.body?;
        let room_id = payload.room_id?;
        let sender = payload.sender?;
        let event_id = payload.event_id.unwrap_or_else(|| "0".to_string());

        // Skip own messages
        if sender == self.user_id {
            return None;
        }

        let mut chain = MessageChain::new();

        // Parse formatted body if available
        if let Some(ref fmt) = content.formatted_body {
            chain.0.push(MessageComponent::Plain { text: body.clone() });
        } else {
            chain.0.push(MessageComponent::Plain { text: body.clone() });
        }

        // Handle images
        if content.msgtype.as_deref() == Some("m.image") {
            if let Some(ref url) = content.url {
                chain.0.push(MessageComponent::Image {
                    url: Some(url.clone()),
                    file_id: None,
                    base64: None,
                });
            }
        }

        let member = MessageMember {
            user_id: sender.clone(),
            nickname: Some(sender.clone()),
            card: None,
            role: None,
            is_self: false,
        };

        let timestamp = if let Some(ts) = payload.origin_server_ts {
            chrono::DateTime::from_timestamp_millis(ts)
                .unwrap_or_else(chrono::Utc::now)
        } else {
            chrono::Utc::now()
        };

        Some(AstrBotMessage {
            message_id: event_id,
            timestamp,
            platform: PlatformType::Matrix,
            session_id: room_id,
            sender: member,
            message_type: MessageType::Group,
            chain,
            raw_payload: None,
        })
    }

    fn parse_sync_event(&self, room_id: &str, event: &SyncEvent) -> Option<AstrBotMessage> {
        if event.event_type != "m.room.message" {
            return None;
        }

        // Skip own messages
        if event.sender == self.user_id {
            return None;
        }

        let content: MatrixMessageContent = serde_json::from_value(event.content.clone()).ok()?;
        let body = content.body?;

        let mut chain = MessageChain::new();

        // Handle images
        if content.msgtype.as_deref() == Some("m.image") {
            if let Some(ref url) = content.url {
                chain.0.push(MessageComponent::Image {
                    url: Some(url.clone()),
                    file_id: None,
                    base64: None,
                });
            }
        }

        // Always add text body
        chain.0.push(MessageComponent::Plain { text: body.clone() });

        let member = MessageMember {
            user_id: event.sender.clone(),
            nickname: Some(event.sender.clone()),
            card: None,
            role: None,
            is_self: false,
        };

        let timestamp = if let Some(ts) = event.origin_server_ts {
            chrono::DateTime::from_timestamp_millis(ts)
                .unwrap_or_else(chrono::Utc::now)
        } else {
            chrono::Utc::now()
        };

        Some(AstrBotMessage {
            message_id: event.event_id.clone(),
            timestamp,
            platform: PlatformType::Matrix,
            session_id: room_id.to_string(),
            sender: member,
            message_type: MessageType::Group,
            chain,
            raw_payload: None,
        })
    }

    async fn sync_once(
        &self,
        since: Option<&str>,
    ) -> Result<Option<String>> {
        let mut url = format!("{}/_matrix/client/v3/sync", self.homeserver);
        url.push_str("?timeout=30000");
        if let Some(s) = since {
            url.push_str(&format!("&since={}", s));
        }

        let resp = self
            .http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Matrix sync failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "Matrix".to_string(),
                message: format!("Matrix sync error: {} - {}", status, text),
            });
        }

        let sync: SyncResponse = resp.json().await
            .map_err(|e| AstrBotError::Serialization(e))?;

        // Process events
        if let Some(ref rooms) = sync.rooms {
            for (room_id, joined_room) in &rooms.join {
                if let Some(ref timeline) = joined_room.timeline {
                    for event in &timeline.events {
                        if let Some(msg) = self.parse_sync_event(room_id, event) {
                            let handler_opt = self.message_handler.read().await.clone();
                            if let Some(ref handler) = handler_opt {
                                handler.on_message(msg).await;
                            }
                        }
                    }
                }
            }
        }

        if !sync.next_batch.is_empty() {
            Ok(Some(sync.next_batch))
        } else {
            Ok(None)
        }
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ---------------------------------------------------------------------------
// Webhook handler
// ---------------------------------------------------------------------------

async fn matrix_webhook_handler(
    State(shared): State<Arc<MatrixShared>>,
    axum::Json(payload): axum::Json<MatrixWebhookPayload>,
) -> StatusCode {
    if let Some(msg) = shared.parse_webhook_payload(payload) {
        let handler_opt = shared.message_handler.read().await.clone();
        if let Some(ref handler) = handler_opt {
            tokio::spawn(async move {
                handler.on_message(msg).await;
            });
        }
    }
    StatusCode::OK
}

// ---------------------------------------------------------------------------
// Sync polling loop
// ---------------------------------------------------------------------------

async fn run_sync_loop(
    shared: Arc<MatrixShared>,
    running: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
) {
    let mut since: Option<String> = None;

    while running.load(Ordering::Relaxed) {
        match shared.sync_once(since.as_deref()).await {
            Ok(next_batch) => {
                connected.store(true, Ordering::Relaxed);
                if let Some(batch) = next_batch {
                    since = Some(batch);
                }
            }
            Err(e) => {
                warn!("[Matrix] Sync error: {}", e);
                connected.store(false, Ordering::Relaxed);
                sleep(Duration::from_secs(5)).await;
            }
        }
    }

    connected.store(false, Ordering::Relaxed);
    info!("[Matrix] Sync loop stopped");
}

// ---------------------------------------------------------------------------
// Matrix adapter
// ---------------------------------------------------------------------------

pub struct MatrixAdapter {
    metadata: PlatformMetadata,
    shared: Arc<MatrixShared>,
    webhook_port: Option<u16>,
    use_sync: bool,
    running: Arc<AtomicBool>,
    server_task: Mutex<Option<JoinHandle<()>>>,
    sync_task: Mutex<Option<JoinHandle<()>>>,
}

impl MatrixAdapter {
    pub fn new(
        id: String,
        homeserver: String,
        access_token: String,
        user_id: String,
        webhook_port: Option<u16>,
        use_sync: bool,
    ) -> Self {
        let metadata = PlatformMetadata {
            id: id.clone(),
            name: format!("Matrix {}", id),
            platform_type: PlatformType::Matrix,
            enabled: true,
            extra: {
                let mut map = HashMap::new();
                map.insert("homeserver".to_string(), serde_json::Value::String(homeserver.clone()));
                if let Some(port) = webhook_port {
                    map.insert("webhook_port".to_string(), serde_json::Value::Number(port.into()));
                }
                map.insert("mode".to_string(), if use_sync {
                    serde_json::Value::String("sync".to_string())
                } else {
                    serde_json::Value::String("webhook".to_string())
                });
                map
            },
        };

        let shared = Arc::new(MatrixShared {
            homeserver,
            access_token,
            user_id,
            http_client: SharedHttpClient::new().client(),
            message_handler: Arc::new(RwLock::new(None)),
        });

        Self {
            metadata,
            shared,
            webhook_port,
            use_sync,
            running: Arc::new(AtomicBool::new(false)),
            server_task: Mutex::new(None),
            sync_task: Mutex::new(None),
        }
    }
}

#[async_trait]
impl PlatformAdapter for MatrixAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!("[Matrix] Initializing adapter for {}...", self.shared.homeserver);
        // Verify token by calling /api/v3/account/whoami
        let url = format!("{}/_matrix/client/v3/account/whoami", self.shared.homeserver.trim_end_matches('/'));
        let resp = self.shared.http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.shared.access_token))
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Matrix auth check failed: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "Matrix".to_string(),
                message: format!("auth check failed: {}", body),
            });
        }

        info!("[Matrix] Token valid, adapter ready");
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        info!("[Matrix] Starting adapter (mode: {})...",
            if self.use_sync { "sync" } else { "webhook" });
        self.running.store(true, Ordering::Relaxed);

        // Start webhook server if configured
        if let Some(port) = self.webhook_port {
            let shared = Arc::clone(&self.shared);
            let app = Router::new()
                .route("/webhook", post(matrix_webhook_handler))
                .with_state(shared);

            let addr = SocketAddr::from(([0, 0, 0, 0], port));
            let listener = tokio::net::TcpListener::bind(&addr).await
                .map_err(|e| AstrBotError::Network(format!("Matrix bind failed: {}", e)))?;

            let running = Arc::clone(&self.running);
            let handle = tokio::spawn(async move {
                info!("[Matrix] Webhook server listening on {}", addr);
                let server = axum::serve(listener, app);
                if let Err(e) = server.await {
                    error!("[Matrix] Server error: {}", e);
                }
            });

            let mut guard = self.server_task.lock().await;
            *guard = Some(handle);
        }

        // Start sync loop if configured
        if self.use_sync {
            let shared = Arc::clone(&self.shared);
            let running = Arc::clone(&self.running);
            let connected = Arc::new(AtomicBool::new(false));
            let handle = tokio::spawn(async move {
                run_sync_loop(shared, running, connected).await;
            });

            let mut guard = self.sync_task.lock().await;
            *guard = Some(handle);
        }

        info!("[Matrix] Adapter started");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("[Matrix] Stopping adapter...");
        self.running.store(false, Ordering::Relaxed);

        let mut guard = self.server_task.lock().await;
        if let Some(handle) = guard.take() {
            let _ = handle.await;
        }

        let mut guard = self.sync_task.lock().await;
        if let Some(handle) = guard.take() {
            let _ = handle.await;
        }

        info!("[Matrix] Adapter stopped");
        Ok(())
    }

    async fn send_message(
        &self,
        target: &MessageSource,
        chain: &MessageChain,
    ) -> Result<()> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(AstrBotError::Platform {
                adapter: "Matrix".to_string(),
                message: "adapter not running".to_string(),
            });
        }

        let room_id = &target.session_id;
        self.shared.send_matrix_message(room_id, chain).await
    }

    async fn reply_message(
        &self,
        original: &AstrBotMessage,
        chain: &MessageChain,
    ) -> Result<()> {
        let source = MessageSource {
            platform: PlatformType::Matrix,
            session_id: original.session_id.clone(),
            message_id: original.message_id.clone(),
            user_id: original.sender.user_id.clone(),
        };
        self.send_message(&source, chain).await
    }

    async fn health_check(&self) -> Result<bool> {
        Ok(self.running.load(Ordering::Relaxed))
    }

    fn set_message_handler(&mut self, handler: Arc<dyn MessageHandler>) {
        let shared = Arc::clone(&self.shared);
        tokio::spawn(async move {
            let mut guard = shared.message_handler.write().await;
            *guard = Some(handler);
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_matrix_adapter_lifecycle() {
        let mut adapter = MatrixAdapter::new(
            "matrix-1".to_string(),
            "https://matrix.org".to_string(),
            "syt_test".to_string(),
            "@bot:matrix.org".to_string(),
            None,
            false,
        );
        assert_eq!(adapter.metadata().platform_type, PlatformType::Matrix);
        adapter.initialize().await.unwrap();
        adapter.start().await.unwrap();
        assert!(adapter.health_check().await.unwrap());
        adapter.stop().await.unwrap();
    }

    #[test]
    fn test_matrix_webhook_payload_parse() {
        let json = r#"{
            "room_id": "!room:matrix.org",
            "sender": "@user:matrix.org",
            "event_id": "$abc123",
            "content": {
                "body": "Hello Matrix!",
                "msgtype": "m.text"
            },
            "origin_server_ts": 1700000000000
        }"#;
        let payload: MatrixWebhookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.room_id, Some("!room:matrix.org".to_string()));
        assert_eq!(payload.sender, Some("@user:matrix.org".to_string()));
        assert_eq!(payload.content.as_ref().unwrap().body, Some("Hello Matrix!".to_string()));
    }

    #[test]
    fn test_matrix_sync_event_parse() {
        let shared = MatrixShared {
            homeserver: "https://matrix.org".to_string(),
            access_token: "test".to_string(),
            user_id: "@bot:matrix.org".to_string(),
            http_client: reqwest::Client::new(),
            message_handler: Arc::new(RwLock::new(None)),
        };

        let event = SyncEvent {
            event_id: "$abc".to_string(),
            event_type: "m.room.message".to_string(),
            sender: "@user:matrix.org".to_string(),
            content: serde_json::json!({
                "body": "hello",
                "msgtype": "m.text"
            }),
            origin_server_ts: Some(1700000000000),
        };

        let msg = shared.parse_sync_event("!room:matrix.org", &event);
        assert!(msg.is_some());
        let msg = msg.unwrap();
        assert_eq!(msg.message_id, "$abc");
        assert_eq!(msg.session_id, "!room:matrix.org");
        assert_eq!(msg.chain.plain_text(), "hello");
    }

    #[test]
    fn test_matrix_send_message_request_serialize() {
        let req = MatrixSendMessageRequest {
            msgtype: "m.text".to_string(),
            body: "Test message".to_string(),
            format: None,
            formatted_body: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("m.text"));
        assert!(json.contains("Test message"));
    }

    #[test]
    fn test_matrix_html_content_build() {
        let shared = MatrixShared {
            homeserver: "https://matrix.org".to_string(),
            access_token: "test".to_string(),
            user_id: "@bot:matrix.org".to_string(),
            http_client: reqwest::Client::new(),
            message_handler: Arc::new(RwLock::new(None)),
        };

        let mut chain = MessageChain::new();
        chain.0.push(MessageComponent::Plain { text: "hi ".to_string() });
        chain.0.push(MessageComponent::At {
            target: "@alice:matrix.org".to_string(),
            display: Some("Alice".to_string()),
        });

        let (body, formatted, format) = shared.build_matrix_content(&chain, "hi Alice");
        assert_eq!(body, "hi Alice");
        assert!(formatted.is_some());
        let html = formatted.unwrap();
        assert!(html.contains("<a href"));
        assert_eq!(format, Some("org.matrix.custom.html".to_string()));
    }
}
