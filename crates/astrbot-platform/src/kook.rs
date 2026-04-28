use async_trait::async_trait;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::message::{AstrBotMessage, MessageChain, MessageComponent, MessageMember, MessageType, HandlerRef, MessageHandler};
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use crate::adapter::PlatformAdapter;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// Kook API data models
// https://developer.kookapp.cn
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct KookMessage {
    #[serde(rename = "id")]
    id: String,
    #[serde(rename = "type")]
    msg_type: i32,
    content: String,
    author: KookUser,
    #[serde(rename = "channel_id")]
    channel_id: String,
    #[serde(rename = "guild_id", default)]
    guild_id: Option<String>,
    #[serde(rename = "mention", default)]
    mentions: Vec<String>,
    #[serde(default)]
    quote: Option<KookQuote>,
}

#[derive(Debug, Clone, Deserialize)]
struct KookUser {
    #[serde(rename = "id")]
    id: String,
    username: String,
    #[serde(rename = "avatar", default)]
    avatar: Option<String>,
    #[serde(rename = "bot", default)]
    bot: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct KookQuote {
    #[serde(rename = "id")]
    id: String,
    content: String,
    author: KookUser,
}

#[derive(Debug, Clone, Deserialize)]
struct KookEvent {
    #[serde(rename = "channel_type")]
    channel_type: String,
    #[serde(rename = "type")]
    event_type: i32,
    #[serde(rename = "target_id")]
    target_id: String,
    #[serde(rename = "author_id")]
    author_id: String,
    content: String,
    #[serde(rename = "msg_id", default)]
    msg_id: Option<String>,
    #[serde(rename = "msg_timestamp")]
    msg_timestamp: i64,
    nonce: String,
    #[serde(default)]
    extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct KookGatewayResponse {
    code: i32,
    data: KookGatewayData,
}

#[derive(Debug, Clone, Deserialize)]
struct KookGatewayData {
    url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct KookWsPayload {
    s: i32,  // signal type
    d: Option<serde_json::Value>,
    #[serde(rename = "sn", default)]
    sn: Option<i64>,
}

// ---------------------------------------------------------------------------
// Kook adapter
// ---------------------------------------------------------------------------

pub struct KookAdapter {
    metadata: PlatformMetadata,
    api_token: String,
    http_client: reqwest::Client,
    running: Arc<AtomicBool>,
    ws_task: Option<JoinHandle<()>>,
    message_handler: HandlerRef,
}

impl KookAdapter {
    pub fn new(api_token: String) -> Self {
        let metadata = PlatformMetadata {
            id: "kook".to_string(),
            name: "Kook".to_string(),
            platform_type: PlatformType::Custom,
            enabled: true,
            extra: HashMap::new(),
        };

        Self {
            metadata,
            api_token,
            http_client: reqwest::Client::new(),
            running: Arc::new(AtomicBool::new(false)),
            ws_task: None,
            message_handler: None,
        }
    }

    fn auth_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bot {}", self.api_token))
                .unwrap_or_else(|_| reqwest::header::HeaderValue::from_static("")),
        );
        headers
    }

    async fn get_gateway_url(&self) -> Result<String> {
        let resp = self.http_client
            .get("https://www.kookapp.cn/api/v3/gateway/index")
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Kook gateway request failed: {}", e)))?;

        let data: KookGatewayResponse = resp.json().await
            .map_err(|e| AstrBotError::Serialization(format!("Kook gateway parse failed: {}", e)))?;

        if data.code != 0 {
            return Err(AstrBotError::Platform {
                adapter: "kook".to_string(),
                message: format!("Kook gateway error code: {}", data.code),
            });
        }

        Ok(data.data.url)
    }

    async fn send_channel_message(
        &self,
        channel_id: &str,
        content: &str,
    ) -> Result<()> {
        let body = serde_json::json!({
            "target_id": channel_id,
            "content": content,
        });

        let resp = self.http_client
            .post("https://www.kookapp.cn/api/v3/message/create")
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Kook send message failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "kook".to_string(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        Ok(())
    }
}

#[async_trait]
impl PlatformAdapter for KookAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!("Kook adapter initialized");
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        self.running.store(true, Ordering::SeqCst);

        let gateway_url = self.get_gateway_url().await?;
        info!("Kook gateway URL: {}", gateway_url);

        let token = self.api_token.clone();
        let running = Arc::clone(&self.running);
        let handler = self.message_handler.clone();

        let task = tokio::spawn(async move {
            let mut retry_count = 0;
            let max_retries = 5;

            while running.load(Ordering::SeqCst) && retry_count < max_retries {
                match tokio_tungstenite::connect_async(&gateway_url).await {
                    Ok((ws_stream, _)) => {
                        info!("Kook WebSocket connected");
                        retry_count = 0;

                        let (mut write, mut read) = ws_stream.split();

                        // Send identify
                        let identify = serde_json::json!({
                            "s": 2,
                            "d": {
                                "token": format!("Bot {}", token),
                                "intents": 0,
                                "property": {
                                    "os": "linux",
                                    "browser": "AstrBot",
                                    "device": "AstrBot"
                                }
                            }
                        });

                        if let Err(e) = write.send(tokio_tungstenite::tungstenite::Message::Text(
                            identify.to_string().into()
                        )).await {
                            error!("Kook identify failed: {}", e);
                            continue;
                        }

                        while running.load(Ordering::SeqCst) {
                            match tokio::time::timeout(
                                Duration::from_secs(30),
                                read.next()
                            ).await {
                                Ok(Some(Ok(msg))) => {
                                    if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                                        if let Ok(payload) = serde_json::from_str::<KookWsPayload>(&text) {
                                            match payload.s {
                                                0 => {
                                                    // Event dispatch
                                                    if let Some(data) = payload.d {
                                                        if let Ok(event) = serde_json::from_value::<KookEvent>(data.clone()) {
                                                            if event.event_type == 1 {
                                                                let platform = PlatformType::Custom;
                                                                let source = MessageSource {
                                                                    platform,
                                                                    session_id: event.target_id.clone(),
                                                                    message_id: event.msg_id.clone().unwrap_or_default(),
                                                                    user_id: event.author_id.clone(),
                                                                };
                                                                let member = MessageMember {
                                                                    user_id: event.author_id.clone(),
                                                                    nickname: Some(event.content.clone()),
                                                                    card: None,
                                                                    role: None,
                                                                    is_self: false,
                                                                };
                                                                let message = AstrBotMessage {
                                                                    message_id: event.msg_id.clone().unwrap_or_default(),
                                                                    timestamp: Utc::now(),
                                                                    platform,
                                                                    session_id: event.target_id.clone(),
                                                                    sender: member,
                                                                    message_type: if event.channel_type == "PERSON" { MessageType::Private } else { MessageType::Group },
                                                                    chain: MessageChain::new().text(&event.content),
                                                                    raw_payload: Some(data),
                                                                };
                                                                if let Some(ref h) = handler {
                                                                    h.on_message(message).await;
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                                1 => info!("Kook WS hello"),
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                                Ok(Some(Err(e))) => {
                                    error!("Kook WS error: {}", e);
                                    break;
                                }
                                Ok(None) => {
                                    info!("Kook WS closed");
                                    break;
                                }
                                Err(_) => {
                                    warn!("Kook WS heartbeat timeout");
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Kook WS connect failed: {}", e);
                        retry_count += 1;
                        sleep(Duration::from_secs(2u64.pow(retry_count.min(5)))).await;
                    }
                }
            }
        });

        self.ws_task = Some(task);
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        if let Some(task) = self.ws_task.take() {
            task.abort();
        }
        info!("Kook adapter stopped");
        Ok(())
    }

    async fn send_message(&self, target: &MessageSource, chain: &MessageChain) -> Result<()> {
        let content = chain.plain_text();
        self.send_channel_message(&target.session_id, &content).await
    }

    async fn reply_message(&self, original: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
        let content = chain.plain_text();
        self.send_channel_message(&original.session_id, &content).await
    }

    async fn health_check(&self) -> Result<bool> {
        match self.get_gateway_url().await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    fn set_message_handler(&mut self, handler: Arc<dyn MessageHandler>) {
        self.message_handler = Some(handler);
    }
}
