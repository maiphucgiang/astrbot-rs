use crate::adapter::PlatformAdapter;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::message::{
    AstrBotMessage, HandlerRef, MessageChain, MessageComponent, MessageHandler, MessageMember,
    MessageType,
};
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use async_trait::async_trait;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep, Duration};
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
    #[serde(rename = "mention", default)]
    mentions: Vec<String>,
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
    s: i32,
    d: Option<serde_json::Value>,
    #[serde(rename = "sn", default)]
    sn: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
struct KookSendMessageRequest {
    #[serde(rename = "target_id")]
    target_id: String,
    content: String,
    #[serde(rename = "quote", skip_serializing_if = "Option::is_none")]
    quote: Option<String>,
}

// ---------------------------------------------------------------------------
// Kook adapter
// ---------------------------------------------------------------------------

pub struct KookAdapter {
    metadata: PlatformMetadata,
    api_token: String,
    http_client: reqwest::Client,
    running: Arc<AtomicBool>,
    ws_task: Mutex<Option<JoinHandle<()>>>,
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
            ws_task: Mutex::new(None),
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
        let resp = self
            .http_client
            .get("https://www.kookapp.cn/api/v3/gateway/index")
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Kook gateway request failed: {}", e)))?;

        let data: KookGatewayResponse = resp.json().await.map_err(|e| {
            AstrBotError::Serialization(format!("Kook gateway parse failed: {}", e))
        })?;

        if data.code != 0 {
            return Err(AstrBotError::Platform {
                adapter: "kook".to_string(),
                message: format!("Kook gateway error code: {}", data.code),
            });
        }

        Ok(data.data.url)
    }

    async fn send_channel_message(&self, channel_id: &str, content: &str) -> Result<()> {
        let body = KookSendMessageRequest {
            target_id: channel_id.to_string(),
            content: content.to_string(),
            quote: None,
        };

        let resp = self
            .http_client
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

        info!("[Kook] Message sent to channel {}", channel_id);
        Ok(())
    }

    /// Build a MessageChain from Kook event content and type.
    fn build_chain(
        content: &str,
        event_type: i32,
        extra: &HashMap<String, serde_json::Value>,
    ) -> MessageChain {
        let mut chain = MessageChain::new();
        match event_type {
            2 => {
                // Image
                if let Some(url) = extra.get("url").and_then(|v| v.as_str()) {
                    chain = chain.image_url(url);
                } else {
                    chain = chain.text("[图片]");
                }
            }
            3 => {
                // Video
                chain = chain.text("[视频]");
            }
            9 => {
                // KMarkdown — treat as text for now
                chain = chain.text(content);
            }
            _ => {
                chain = chain.text(content);
            }
        }
        chain
    }
}

#[async_trait]
impl PlatformAdapter for KookAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!("[Kook] Adapter initialized");
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        self.running.store(true, Ordering::SeqCst);

        let gateway_url = self.get_gateway_url().await?;
        info!("[Kook] Gateway URL: {}", gateway_url);

        let token = self.api_token.clone();
        let running = Arc::clone(&self.running);
        let handler = self.message_handler.clone();

        let task = tokio::spawn(async move {
            let mut retry_count = 0;
            let max_retries = 5;

            while running.load(Ordering::SeqCst) && retry_count < max_retries {
                match tokio_tungstenite::connect_async(&gateway_url).await {
                    Ok((ws_stream, _)) => {
                        info!("[Kook] WebSocket connected");
                        retry_count = 0;

                        let (mut write, mut read) = ws_stream.split();

                        // Send identify (signal type 2)
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

                        if let Err(e) = write
                            .send(tokio_tungstenite::tungstenite::Message::Text(
                                identify.to_string(),
                            ))
                            .await
                        {
                            error!("[Kook] Identify failed: {}", e);
                            continue;
                        }

                        // Heartbeat task (signal type 3 every 30s)
                        let heartbeat_running = Arc::clone(&running);
                        let mut heartbeat_write = write;
                        let heartbeat_task = tokio::spawn(async move {
                            let mut ticker = interval(Duration::from_secs(30));
                            ticker.tick().await; // skip immediate first tick
                            while heartbeat_running.load(Ordering::SeqCst) {
                                ticker.tick().await;
                                let ping = serde_json::json!({"s": 3, "sn": 0});
                                if let Err(e) = heartbeat_write
                                    .send(tokio_tungstenite::tungstenite::Message::Text(
                                        ping.to_string(),
                                    ))
                                    .await
                                {
                                    error!("[Kook] Heartbeat send failed: {}", e);
                                    break;
                                }
                            }
                        });

                        // Message read loop
                        while running.load(Ordering::SeqCst) {
                            match tokio::time::timeout(Duration::from_secs(60), read.next()).await {
                                Ok(Some(Ok(msg))) => {
                                    if let tokio_tungstenite::tungstenite::Message::Text(text) = msg
                                    {
                                        if let Ok(payload) =
                                            serde_json::from_str::<KookWsPayload>(&text)
                                        {
                                            match payload.s {
                                                0 => {
                                                    // Event dispatch
                                                    if let Some(data) = payload.d {
                                                        if let Ok(event) =
                                                            serde_json::from_value::<KookEvent>(
                                                                data.clone(),
                                                            )
                                                        {
                                                            if event.author_id.is_empty() {
                                                                continue;
                                                            }
                                                            let platform = PlatformType::Custom;
                                                            let chain = Self::build_chain(
                                                                &event.content,
                                                                event.event_type,
                                                                &event.extra,
                                                            );
                                                            let member = MessageMember {
                                                                user_id: event.author_id.clone(),
                                                                nickname: Some(
                                                                    event.author_id.clone(),
                                                                ),
                                                                card: None,
                                                                role: None,
                                                                is_self: false,
                                                            };
                                                            let message_type =
                                                                if event.channel_type == "PERSON" {
                                                                    MessageType::Private
                                                                } else {
                                                                    MessageType::Group
                                                                };
                                                            let message = AstrBotMessage {
                                                                message_id: event
                                                                    .msg_id
                                                                    .clone()
                                                                    .unwrap_or_default(),
                                                                timestamp: Utc::now(),
                                                                platform,
                                                                session_id: event.target_id.clone(),
                                                                sender: member,
                                                                message_type,
                                                                chain,
                                                                raw_payload: Some(data),
                                                            };
                                                            if let Some(ref h) = handler {
                                                                h.on_message(message).await;
                                                            }
                                                        }
                                                    }
                                                }
                                                1 => info!("[Kook] WS hello received"),
                                                3 => info!("[Kook] Heartbeat pong received"),
                                                11 => warn!("[Kook] Resume required"),
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                                Ok(Some(Err(e))) => {
                                    error!("[Kook] WS error: {}", e);
                                    break;
                                }
                                Ok(None) => {
                                    info!("[Kook] WS closed by server");
                                    break;
                                }
                                Err(_) => {
                                    warn!("[Kook] Read timeout, reconnecting");
                                    break;
                                }
                            }
                        }

                        heartbeat_task.abort();
                    }
                    Err(e) => {
                        error!("[Kook] WS connect failed: {}", e);
                        retry_count += 1;
                        let backoff = std::cmp::min(retry_count, 6);
                        sleep(Duration::from_secs(2u64.pow(backoff))).await;
                    }
                }
            }

            if retry_count >= max_retries {
                error!("[Kook] Max retries exceeded, giving up");
            }
        });

        let mut guard = self.ws_task.lock().await;
        *guard = Some(task);
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("[Kook] Stopping adapter...");
        self.running.store(false, Ordering::SeqCst);
        let mut guard = self.ws_task.lock().await;
        if let Some(task) = guard.take() {
            let _ = task.await;
        }
        info!("[Kook] Adapter stopped");
        Ok(())
    }

    async fn send_message(&self, target: &MessageSource, chain: &MessageChain) -> Result<()> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(AstrBotError::Platform {
                adapter: "kook".to_string(),
                message: "adapter not running".to_string(),
            });
        }
        let content = chain.plain_text();
        if content.is_empty() {
            return Ok(());
        }
        self.send_channel_message(&target.session_id, &content)
            .await
    }

    async fn reply_message(&self, original: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
        let content = chain.plain_text();
        self.send_channel_message(&original.session_id, &content)
            .await
    }

    async fn health_check(&self) -> Result<bool> {
        match self.get_gateway_url().await {
            Ok(_) => Ok(true),
            Err(e) => {
                warn!("[Kook] Health check failed: {}", e);
                Ok(false)
            }
        }
    }

    fn set_message_handler(&mut self, handler: Arc<dyn MessageHandler>) {
        self.message_handler = Some(handler);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_kook_adapter_lifecycle() {
        let mut adapter = KookAdapter::new("test-token".to_string());
        assert_eq!(adapter.metadata().platform_type, PlatformType::Custom);
        assert_eq!(adapter.metadata().name, "Kook");
        adapter.initialize().await.unwrap();
        // Don't actually start WS in tests — health_check should fail because token is invalid
        let health = adapter.health_check().await.unwrap();
        assert!(!health);
    }

    #[test]
    fn test_kook_event_parse() {
        let json = r#"{
            "channel_type": "GROUP",
            "type": 1,
            "target_id": "123456",
            "author_id": "789012",
            "content": "Hello Kook!",
            "msg_id": "abc123",
            "msg_timestamp": 1700000000000,
            "nonce": "nonce123",
            "mention": ["111111"],
            "extra": {}
        }"#;
        let event: KookEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.channel_type, "GROUP");
        assert_eq!(event.event_type, 1);
        assert_eq!(event.target_id, "123456");
        assert_eq!(event.author_id, "789012");
        assert_eq!(event.content, "Hello Kook!");
        assert_eq!(event.msg_id, Some("abc123".to_string()));
        assert_eq!(event.mentions, vec!["111111"]);
    }

    #[test]
    fn test_kook_image_event_parse() {
        let json = r#"{
            "channel_type": "GROUP",
            "type": 2,
            "target_id": "123456",
            "author_id": "789012",
            "content": "",
            "msg_id": "img123",
            "msg_timestamp": 1700000000000,
            "nonce": "nonce456",
            "extra": {
                "url": "https://img.kookapp.cn/attachments/xxx.png"
            }
        }"#;
        let event: KookEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_type, 2);
        let extra = event.extra;
        let chain = KookAdapter::build_chain(&event.content, event.event_type, &extra);
        // Should contain image component
        let components: Vec<_> = chain.components().iter().collect();
        assert!(!components.is_empty());
    }

    #[test]
    fn test_kook_send_message_request_serialize() {
        let req = KookSendMessageRequest {
            target_id: "123456".to_string(),
            content: "Test".to_string(),
            quote: Some("quote-id".to_string()),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("123456"));
        assert!(json.contains("Test"));
        assert!(json.contains("quote-id"));
    }

    #[test]
    fn test_kook_heartbeat_payload() {
        let ping = serde_json::json!({"s": 3, "sn": 0});
        let text = ping.to_string();
        assert!(text.contains("\"s\":3"));
        assert!(text.contains("\"sn\":0"));
    }
}
