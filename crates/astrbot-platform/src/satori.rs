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
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// Satori Protocol models
// https://satori.js.org
// Satori is a universal chatbot protocol by Koishi team.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
struct SatoriLoginPayload {
    op: u8,
    body: SatoriLoginBody,
}

#[derive(Debug, Clone, Serialize)]
struct SatoriLoginBody {
    token: String,
}

#[derive(Debug, Clone, Deserialize)]
struct SatoriEvent {
    op: u8,
    #[serde(default)]
    body: Option<SatoriEventBody>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SatoriEventBody {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    #[serde(rename = "selfId")]
    self_id: Option<String>,
    #[serde(default)]
    timestamp: Option<i64>,
    #[serde(default)]
    #[serde(rename = "channel")]
    channel: Option<SatoriChannel>,
    #[serde(default)]
    #[serde(rename = "user")]
    user: Option<SatoriUser>,
    #[serde(default)]
    #[serde(rename = "message")]
    message: Option<SatoriMessageBody>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SatoriChannel {
    id: String,
    #[serde(default)]
    #[serde(rename = "type")]
    channel_type: Option<i32>, // 0=direct, 1=text, 2=voice, 3=category
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SatoriUser {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    nick: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SatoriMessageBody {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SatoriMessageCreateRequest {
    #[serde(rename = "channelId")]
    channel_id: String,
    content: String,
}

// ---------------------------------------------------------------------------
// Satori adapter
// ---------------------------------------------------------------------------

pub struct SatoriAdapter {
    metadata: PlatformMetadata,
    endpoint: String,
    token: String,
    running: Arc<AtomicBool>,
    ws_task: Option<JoinHandle<()>>,
    message_handler: HandlerRef,
}

impl SatoriAdapter {
    pub fn new(endpoint: String, token: String) -> Self {
        let metadata = PlatformMetadata {
            id: "satori".to_string(),
            name: "Satori".to_string(),
            platform_type: PlatformType::Satori,
            enabled: true,
            extra: HashMap::new(),
        };

        Self {
            metadata,
            endpoint,
            token,
            running: Arc::new(AtomicBool::new(false)),
            ws_task: None,
            message_handler: None,
        }
    }
}

#[async_trait]
impl PlatformAdapter for SatoriAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!("Satori adapter initialized — endpoint: {}", self.endpoint);
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        self.running.store(true, Ordering::SeqCst);

        let ws_url = format!("{}/v1/events", self.endpoint.replace("http://", "ws://").replace("https://", "wss://"));
        let token = self.token.clone();
        let running = Arc::clone(&self.running);
        let handler = self.message_handler.clone();

        let task = tokio::spawn(async move {
            let mut retry_count = 0;
            let max_retries = 5;

            while running.load(Ordering::SeqCst) && retry_count < max_retries {
                match tokio_tungstenite::connect_async(&ws_url).await {
                    Ok((mut ws_stream, _)) => {
                        info!("Satori WebSocket connected");
                        retry_count = 0;

                        // Send login payload
                        let login = SatoriLoginPayload {
                            op: 1,
                            body: SatoriLoginBody { token: token.clone() },
                        };
                        let login_json = serde_json::to_string(&login).unwrap_or_default();
                        let _ = ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(login_json.into())).await;

                        while running.load(Ordering::SeqCst) {
                            match tokio::time::timeout(
                                Duration::from_secs(60),
                                ws_stream.next()
                            ).await {
                                Ok(Some(Ok(msg))) => {
                                    if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                                        if let Ok(event) = serde_json::from_str::<SatoriEvent>(&text) {
                                            if event.op == 0 && event.body.is_some() {
                                                let body = event.body.unwrap();
                                                if body.event_type == "message-created" {
                                                    if let (Some(channel), Some(user), Some(message)) = 
                                                        (body.channel.as_ref(), body.user.as_ref(), body.message.as_ref()) {
                                                        let platform = PlatformType::Satori;
                                                        let source = MessageSource {
                                                            platform,
                                                            session_id: channel.id.clone(),
                                                            message_id: message.id.clone().unwrap_or_default(),
                                                            user_id: user.id.clone(),
                                                        };
                                                        let member = MessageMember {
                                                            user_id: user.id.clone(),
                                                            nickname: user.name.clone(),
                                                            card: user.nick.clone(),
                                                            role: None,
                                                            is_self: false,
                                                        };
                                                        let msg_type = match channel.channel_type {
                                                            Some(0) => MessageType::Private,
                                                            _ => MessageType::Group,
                                                        };
                                                        let astr_message = AstrBotMessage {
                                                            message_id: message.id.clone().unwrap_or_default(),
                                                            timestamp: Utc::now(),
                                                            platform,
                                                            session_id: channel.id.clone(),
                                                            sender: member,
                                                            message_type: msg_type,
                                                            chain: MessageChain::new().text(&message.content.clone().unwrap_or_default()),
                                                            raw_payload: Some(serde_json::to_value(&body).unwrap_or_default()),
                                                        };
                                                        if let Some(ref h) = handler {
                                                            h.on_message(astr_message).await;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Ok(Some(Err(e))) => {
                                    error!("Satori WS error: {}", e);
                                    break;
                                }
                                Ok(None) => {
                                    info!("Satori WS closed");
                                    break;
                                }
                                Err(_) => {
                                    warn!("Satori WS heartbeat timeout");
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Satori WS connect failed: {}", e);
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
        info!("Satori adapter stopped");
        Ok(())
    }

    async fn send_message(&self, target: &MessageSource, chain: &MessageChain) -> Result<()> {
        let content = chain.plain_text();
        let client = reqwest::Client::new();

        let body = SatoriMessageCreateRequest {
            channel_id: target.session_id.clone(),
            content,
        };

        let resp = client
            .post(format!("{}/v1/message.create", self.endpoint))
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Satori send: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "satori".to_string(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        Ok(())
    }

    async fn reply_message(&self, original: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
        let content = chain.plain_text();
        let client = reqwest::Client::new();

        let body = SatoriMessageCreateRequest {
            channel_id: original.session_id.clone(),
            content,
        };

        let resp = client
            .post(format!("{}/v1/message.create", self.endpoint))
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Satori reply: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "satori".to_string(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        Ok(())
    }

    async fn health_check(&self) -> Result<bool> {
        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{}/v1/login", self.endpoint))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await;

        match resp {
            Ok(r) => Ok(r.status().is_success()),
            Err(_) => Ok(false),
        }
    }

    fn set_message_handler(&mut self, handler: Arc<dyn MessageHandler>) {
        self.message_handler = Some(handler);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_satori_adapter_new() {
        let adapter = SatoriAdapter::new(
            "http://localhost:5140".to_string(),
            "token_test".to_string(),
        );
        assert_eq!(adapter.metadata.name, "Satori");
        assert!(adapter.metadata.enabled);
    }
}
