use async_trait::async_trait;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::message::{AstrBotMessage, MessageChain, MessageComponent, MessageMember, MessageType, MessageHandler};
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use astrbot_core::net::SharedHttpClient;
use crate::adapter::PlatformAdapter;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};
use futures_util::{SinkExt, StreamExt};
use chrono::Utc;

// ---------------------------------------------------------------------------
// Discord Gateway data models
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct GatewayIdentify {
    op: u8,
    d: IdentifyPayload,
}

#[derive(Debug, Serialize)]
struct IdentifyPayload {
    token: String,
    intents: u64,
    properties: IdentifyProperties,
}

#[derive(Debug, Serialize)]
struct IdentifyProperties {
    #[serde(rename = "os")]
    os: String,
    #[serde(rename = "browser")]
    browser: String,
    #[serde(rename = "device")]
    device: String,
}

#[derive(Debug, Serialize)]
struct GatewayHeartbeat {
    op: u8,
    d: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GatewayPayload {
    op: u8,
    #[serde(rename = "d")]
    data: Option<serde_json::Value>,
    #[serde(rename = "s")]
    seq: Option<u64>,
    #[serde(rename = "t")]
    event_name: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct DiscordMessage {
    id: String,
    #[serde(rename = "channel_id")]
    channel_id: String,
    #[serde(rename = "guild_id")]
    guild_id: Option<String>,
    author: DiscordUser,
    content: String,
    #[serde(rename = "timestamp")]
    timestamp: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct DiscordUser {
    id: String,
    username: String,
    #[serde(rename = "global_name")]
    global_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HelloPayload {
    #[serde(rename = "heartbeat_interval")]
    heartbeat_interval: u64,
}

// ---------------------------------------------------------------------------
// Discord adapter
// ---------------------------------------------------------------------------

pub struct DiscordAdapter {
    metadata: PlatformMetadata,
    bot_token: String,
    handler: Option<Arc<dyn MessageHandler>>,
    running: Arc<AtomicBool>,
    heartbeat_handle: Option<JoinHandle<()>>,
    gateway_handle: Option<JoinHandle<()>>,
    seq: Arc<AtomicU64>,
    client: reqwest::Client,
    shared: Arc<Mutex<DiscordShared>>,
}

struct DiscordShared {
    bot_token: String,
    handler: Option<Arc<dyn MessageHandler>>,
}

impl DiscordAdapter {
    pub fn new(bot_token: String) -> Self {
        let shared = Arc::new(Mutex::new(DiscordShared {
            bot_token: bot_token.clone(),
            handler: None,
        }));
        Self {
            metadata: PlatformMetadata {
                id: "discord".to_string(),
                name: "Discord".to_string(),
                platform_type: PlatformType::Discord,
                enabled: true,
                extra: std::collections::HashMap::new(),
            },
            bot_token,
            handler: None,
            running: Arc::new(AtomicBool::new(false)),
            heartbeat_handle: None,
            gateway_handle: None,
            seq: Arc::new(AtomicU64::new(0)),
            client: SharedHttpClient::new().client(),
            shared,
        }
    }

    /// Get gateway URL from Discord API
    async fn get_gateway_url(&self) -> Result<String> {
        let resp = self.client
            .get("https://discord.com/api/v10/gateway/bot")
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await
            .map_err(|e| AstrBotError::Platform {
                adapter: "discord".to_string(),
                message: format!("gateway request failed: {}", e),
            })?;

        let data: serde_json::Value = resp.json().await.map_err(|e| AstrBotError::Platform {
            adapter: "discord".to_string(),
            message: format!("gateway json parse failed: {}", e),
        })?;

        let url = data.get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AstrBotError::Platform {
                adapter: "discord".to_string(),
                message: "gateway url missing".to_string(),
            })?;

        Ok(format!("{}?v=10&encoding=json", url))
    }

    /// Send a message to a Discord channel
    async fn send_channel_message(&self, channel_id: &str, content: &str) -> Result<()> {
        let url = format!("https://discord.com/api/v10/channels/{}/messages", channel_id);
        let body = serde_json::json!({ "content": content });

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Platform {
                adapter: "discord".to_string(),
                message: format!("send message failed: {}", e),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "discord".to_string(),
                message: format!("send message error {}: {}", status, text),
            });
        }

        Ok(())
    }

    /// Convert a Discord message to AstrBotMessage
    fn convert_message(msg: &DiscordMessage) -> AstrBotMessage {
        let chain = MessageChain::new().text(&msg.content);

        AstrBotMessage {
            message_id: msg.id.clone(),
            timestamp: Utc::now(),
            platform: PlatformType::Discord,
            session_id: msg.channel_id.clone(),
            sender: MessageMember {
                user_id: msg.author.id.clone(),
                nickname: Some(msg.author.global_name.clone().unwrap_or_else(|| msg.author.username.clone())),
                card: None,
                role: None,
                is_self: false,
            },
            message_type: MessageType::Channel,
            chain,
            raw_payload: Some(serde_json::to_value(msg).unwrap_or(serde_json::Value::Null)),
        }
    }

    /// Run the gateway connection
    async fn run_gateway(
        shared: Arc<Mutex<DiscordShared>>,
        running: Arc<AtomicBool>,
        seq: Arc<AtomicU64>,
        client: reqwest::Client,
    ) {
        let token = {
            let s = shared.lock().await;
            s.bot_token.clone()
        };

        let resp = client
            .get("https://discord.com/api/v10/gateway/bot")
            .header("Authorization", format!("Bot {}", token))
            .send()
            .await;

        let gateway_url = match resp {
            Ok(r) => {
                let data: serde_json::Value = match r.json().await {
                    Ok(v) => v,
                    Err(e) => {
                        error!("[Discord] Gateway JSON parse failed: {}", e);
                        return;
                    }
                };
                match data.get("url").and_then(|v| v.as_str()) {
                    Some(url) => format!("{}?v=10&encoding=json", url),
                    None => {
                        error!("[Discord] Gateway URL missing");
                        return;
                    }
                }
            }
            Err(e) => {
                error!("[Discord] Gateway request failed: {}", e);
                return;
            }
        };

        info!("[Discord] Connecting to gateway: {}", gateway_url);

        let (ws_stream, _) = match tokio_tungstenite::connect_async(&gateway_url).await {
            Ok(conn) => conn,
            Err(e) => {
                error!("[Discord] WebSocket connection failed: {}", e);
                return;
            }
        };

        let (mut write, mut read) = ws_stream.split();
        let mut heartbeat_interval: u64 = 45000;

        // Read hello
        if let Some(Ok(msg)) = read.next().await {
            if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                if let Ok(payload) = serde_json::from_str::<GatewayPayload>(&text) {
                    if payload.op == 10 {
                        if let Some(data) = payload.data {
                            if let Ok(hello) = serde_json::from_value::<HelloPayload>(data) {
                                heartbeat_interval = hello.heartbeat_interval;
                            }
                        }
                    }
                }
            }
        }

        info!("[Discord] Hello received, heartbeat interval: {}ms", heartbeat_interval);

        // Send identify
        let identify = GatewayIdentify {
            op: 2,
            d: IdentifyPayload {
                token: token.clone(),
                intents: 512, // GUILD_MESSAGES
                properties: IdentifyProperties {
                    os: "linux".to_string(),
                    browser: "AstrBot".to_string(),
                    device: "AstrBot".to_string(),
                },
            },
        };

        let identify_json = serde_json::to_string(&identify).unwrap();
        let _ = write.send(tokio_tungstenite::tungstenite::Message::Text(identify_json.into())).await;

        // Start heartbeat task
        let heartbeat_running = running.clone();
        let heartbeat_seq = seq.clone();
        let heartbeat_handle = tokio::spawn(async move {
            while heartbeat_running.load(Ordering::Relaxed) {
                sleep(Duration::from_millis(heartbeat_interval)).await;
                let s = heartbeat_seq.load(Ordering::Relaxed);
                let hb = GatewayHeartbeat {
                    op: 1,
                    d: if s > 0 { Some(s) } else { None },
                };
                info!("[Discord] Heartbeat (seq={:?})", hb.d);
            }
        });

        // Read messages
        while running.load(Ordering::Relaxed) {
            match tokio::time::timeout(Duration::from_secs(30), read.next()).await {
                Ok(Some(Ok(msg))) => {
                    if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                        if let Ok(payload) = serde_json::from_str::<GatewayPayload>(&text) {
                            if let Some(s) = payload.seq {
                                seq.store(s, Ordering::Relaxed);
                            }

                            if payload.op == 0 {
                                if let Some(event) = payload.event_name.as_deref() {
                                    if event == "MESSAGE_CREATE" {
                                        if let Some(data) = payload.data {
                                            if let Ok(dmsg) = serde_json::from_value::<DiscordMessage>(data) {
                                                let ab_msg = DiscordAdapter::convert_message(&dmsg);
                                                let s = shared.lock().await;
                                                if let Some(ref handler) = s.handler {
                                                    handler.on_message(ab_msg).await;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(Some(Err(e))) => {
                    error!("[Discord] WebSocket error: {}", e);
                    break;
                }
                Ok(None) => {
                    info!("[Discord] WebSocket closed");
                    break;
                }
                Err(_) => {
                    warn!("[Discord] Read timeout");
                }
            }
        }

        let _ = heartbeat_handle.await;
        info!("[Discord] Gateway loop ended");
    }
}

#[async_trait]
impl PlatformAdapter for DiscordAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!("[Discord] Adapter initialized");
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        if self.running.load(Ordering::Relaxed) {
            warn!("[Discord] Adapter already running");
            return Ok(());
        }

        self.running.store(true, Ordering::Relaxed);

        {
            let mut s = self.shared.lock().await;
            s.handler = self.handler.clone();
        }

        let shared = self.shared.clone();
        let running = self.running.clone();
        let seq = self.seq.clone();
        let client = self.client.clone();

        let handle = tokio::spawn(async move {
            DiscordAdapter::run_gateway(shared, running, seq, client).await;
        });

        self.gateway_handle = Some(handle);
        info!("[Discord] Adapter started");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.gateway_handle.take() {
            let _ = handle.await;
        }
        if let Some(handle) = self.heartbeat_handle.take() {
            let _ = handle.await;
        }
        info!("[Discord] Adapter stopped");
        Ok(())
    }

    async fn send_message(&self, target: &MessageSource, chain: &MessageChain) -> Result<()> {
        let text = chain.plain_text();
        self.send_channel_message(&target.session_id, &text).await
    }

    async fn reply_message(&self, original: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
        let text = chain.plain_text();
        self.send_channel_message(&original.session_id, &text).await
    }

    async fn health_check(&self) -> Result<bool> {
        Ok(self.running.load(Ordering::Relaxed))
    }

    fn set_message_handler(&mut self, handler: Arc<dyn MessageHandler>) {
        self.handler = Some(handler.clone());
    }
}
