use async_trait::async_trait;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::message::{AstrBotMessage, MessageChain, MessageComponent, MessageMember, MessageType, HandlerRef, MessageHandler};
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use crate::adapter::PlatformAdapter;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration, interval};
use tracing::{error, info, warn};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMessage};

// ---------------------------------------------------------------------------
// Mattermost API data models
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
struct CreatePostRequest {
    channel_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "root_id")]
    root_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MattermostPost {
    id: String,
    #[serde(rename = "channel_id")]
    channel_id: String,
    #[serde(rename = "user_id")]
    user_id: String,
    message: String,
    #[serde(rename = "create_at")]
    create_at: i64,
    #[serde(rename = "root_id")]
    root_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MattermostUser {
    id: String,
    username: String,
    #[serde(rename = "first_name")]
    #[serde(default)]
    first_name: Option<String>,
    #[serde(rename = "last_name")]
    #[serde(default)]
    last_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MattermostWebSocketEvent {
    #[serde(rename = "event")]
    event: String,
    #[serde(rename = "broadcast")]
    #[serde(default)]
    broadcast: Option<serde_json::Value>,
    #[serde(rename = "data")]
    #[serde(default)]
    data: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(rename = "seq")]
    #[serde(default)]
    seq: i64,
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// Shared state for the Mattermost adapter, usable across tasks
#[derive(Debug, Clone)]
pub struct MattermostShared {
    pub ws_url: String,
    pub api_url: String,
    pub token: String,
    pub bot_id: String,
    pub http_client: reqwest::Client,
}

impl MattermostShared {
    pub fn new(ws_url: impl Into<String>, api_url: impl Into<String>, token: impl Into<String>, bot_id: impl Into<String>) -> Self {
        Self {
            ws_url: ws_url.into(),
            api_url: api_url.into(),
            token: token.into(),
            bot_id: bot_id.into(),
            http_client: reqwest::Client::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Message conversion
// ---------------------------------------------------------------------------

fn parse_mattermost_post(post: &MattermostPost, user: Option<&MattermostUser>) -> AstrBotMessage {
    let sender = if let Some(u) = user {
        let name = u.first_name.clone().unwrap_or_else(|| u.username.clone());
        MessageMember {
            user_id: u.id.clone(),
            nickname: Some(name),
            card: u.last_name.clone(),
            role: None,
            is_self: false,
        }
    } else {
        MessageMember {
            user_id: post.user_id.clone(),
            nickname: Some(post.user_id.clone()),
            card: None,
            role: None,
            is_self: false,
        }
    };

    let mut chain = MessageChain::new();
    if !post.message.is_empty() {
        chain.0.push(MessageComponent::Plain { text: post.message.clone() });
    }

    AstrBotMessage {
        message_id: post.id.clone(),
        timestamp: chrono::DateTime::from_timestamp_millis(post.create_at)
            .unwrap_or_else(chrono::Utc::now),
        platform: PlatformType::Mattermost,
        session_id: post.channel_id.clone(),
        sender,
        message_type: MessageType::Group,
        chain,
        raw_payload: None,
    }
}

fn chain_to_mattermost_message(chain: &MessageChain) -> String {
    chain.0.iter().map(|c| match c {
        MessageComponent::Plain { text } => text.as_str(),
        MessageComponent::At { target, display } => {
            display.as_deref().unwrap_or(target.as_str())
        }
        _ => "",
    }).collect::<String>()
}

// ---------------------------------------------------------------------------
// WebSocket loop
// ---------------------------------------------------------------------------

async fn run_websocket_loop(
    shared: MattermostShared,
    running: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    handler: Arc<std::sync::Mutex<HandlerRef>>,
) {
    let ws_url = format!("{}?token={}", shared.ws_url, shared.token);

    match connect_async(&ws_url).await {
        Ok((mut ws_stream, _)) => {
            info!("[Mattermost] WebSocket connected");
            connected.store(true, Ordering::Relaxed);

            let mut heartbeat = interval(Duration::from_secs(30));

            while running.load(Ordering::Relaxed) {
                tokio::select! {
                    _ = heartbeat.tick() => {
                        let ping = serde_json::json!({"action":"ping","seq":0});
                        if let Err(e) = ws_stream.send(WsMessage::Text(ping.to_string().into())).await {
                            warn!("[Mattermost] Heartbeat send failed: {}", e);
                            break;
                        }
                    }
                    msg = ws_stream.next() => {
                        match msg {
                            Some(Ok(WsMessage::Text(text))) => {
                                if let Ok(event) = serde_json::from_str::<MattermostWebSocketEvent>(&text) {
                                    if event.event == "posted" {
                                        if let Some(ref data) = event.data {
                                            if let Some(post_raw) = data.get("post") {
                                                if let Ok(post) = serde_json::from_str::<MattermostPost>(post_raw.as_str().unwrap_or("{}")) {
                                                    if post.user_id != shared.bot_id {
                                                        let user = data.get("sender_name").and_then(|v| {
                                                            let u = MattermostUser {
                                                                id: post.user_id.clone(),
                                                                username: v.as_str()?.to_string(),
                                                                first_name: None,
                                                                last_name: None,
                                                            };
                                                            Some(u)
                                                        });
                                                        let astr_msg = parse_mattermost_post(&post, user.as_ref());
                                                        let handler_opt = handler.lock().unwrap().clone();
                                                        if let Some(ref h) = handler_opt {
                                                            h.on_message(astr_msg).await;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Some(Ok(WsMessage::Close(_))) => {
                                info!("[Mattermost] WebSocket closed by server");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!("[Mattermost] WebSocket error: {}", e);
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }

            connected.store(false, Ordering::Relaxed);
            info!("[Mattermost] WebSocket loop stopped");
        }
        Err(e) => {
            error!("[Mattermost] Failed to connect WebSocket: {}", e);
            connected.store(false, Ordering::Relaxed);
        }
    }
}

// ---------------------------------------------------------------------------
// Mattermost Adapter
// ---------------------------------------------------------------------------

pub struct MattermostAdapter {
    metadata: PlatformMetadata,
    shared: MattermostShared,
    connected: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    handler: Arc<std::sync::Mutex<HandlerRef>>,
    ws_handle: Mutex<Option<JoinHandle<()>>>,
}

impl MattermostAdapter {
    pub fn new(shared: MattermostShared) -> Self {
        Self {
            metadata: PlatformMetadata {
                id: "mattermost".to_string(),
                name: "Mattermost".to_string(),
                platform_type: PlatformType::Mattermost,
                enabled: true,
                extra: HashMap::new(),
            },
            shared,
            connected: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
            handler: Arc::new(std::sync::Mutex::new(None)),
            ws_handle: Mutex::new(None),
        }
    }
}

#[async_trait]
impl PlatformAdapter for MattermostAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!("[Mattermost] Initializing adapter...");
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        info!("[Mattermost] Starting adapter...");
        self.running.store(true, Ordering::Relaxed);

        let shared = self.shared.clone();
        let running = Arc::clone(&self.running);
        let connected = Arc::clone(&self.connected);
        let handler = Arc::clone(&self.handler);

        let handle = tokio::spawn(async move {
            run_websocket_loop(shared, running, connected, handler).await;
        });

        let mut guard = self.ws_handle.lock().await;
        *guard = Some(handle);

        info!("[Mattermost] Adapter started");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("[Mattermost] Stopping adapter...");
        self.running.store(false, Ordering::Relaxed);
        self.connected.store(false, Ordering::Relaxed);

        let mut guard = self.ws_handle.lock().await;
        if let Some(handle) = guard.take() {
            let _ = handle.await;
        }
        Ok(())
    }

    async fn send_message(&self, target: &MessageSource, chain: &MessageChain) -> Result<()> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(AstrBotError::Platform {
                adapter: "Mattermost".to_string(),
                message: "adapter not running".to_string(),
            });
        }

        let text = chain_to_mattermost_message(chain);
        let req = CreatePostRequest {
            channel_id: target.session_id.clone(),
            message: Some(text),
            root_id: None,
        };

        let url = format!("{}/api/v4/posts", self.shared.api_url);
        let response = self.shared.http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.shared.token))
            .json(&req)
            .send()
            .await
            .map_err(|e| AstrBotError::Platform {
                adapter: "Mattermost".to_string(),
                message: format!("HTTP request failed: {}", e),
            })?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "Mattermost".to_string(),
                message: format!("API error: {}", body),
            });
        }

        info!("[Mattermost] Message sent successfully");
        Ok(())
    }

    async fn reply_message(&self, original: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
        let text = chain_to_mattermost_message(chain);
        let req = CreatePostRequest {
            channel_id: original.session_id.clone(),
            message: Some(text),
            root_id: Some(original.message_id.clone()),
        };

        let url = format!("{}/api/v4/posts", self.shared.api_url);
        let response = self.shared.http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.shared.token))
            .json(&req)
            .send()
            .await
            .map_err(|e| AstrBotError::Platform {
                adapter: "Mattermost".to_string(),
                message: format!("HTTP request failed: {}", e),
            })?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "Mattermost".to_string(),
                message: format!("API error: {}", body),
            });
        }

        Ok(())
    }

    async fn health_check(&self) -> Result<bool> {
        Ok(self.running.load(Ordering::Relaxed) && self.connected.load(Ordering::Relaxed))
    }

    fn set_message_handler(&mut self, handler: Arc<dyn MessageHandler>) {
        let mut h = self.handler.lock().unwrap();
        *h = Some(handler);
    }
}
