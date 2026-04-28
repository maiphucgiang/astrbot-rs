use async_trait::async_trait;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::message::{AstrBotMessage, MessageChain, MessageComponent, MessageMember, MessageType, HandlerRef, MessageHandler};
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use crate::adapter::PlatformAdapter;
use axum::{routing::post, Router};
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// Line API models
// https://developers.line.biz
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
struct LineReplyRequest {
    reply_token: String,
    messages: Vec<LineMessage>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
enum LineMessage {
    #[serde(rename = "text")]
    Text { text: String },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LineWebhookPayload {
    events: Vec<LineEvent>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LineEvent {
    #[serde(rename = "type")]
    event_type: String,
    timestamp: i64,
    source: LineEventSource,
    #[serde(default)]
    message: Option<LineEventMessage>,
    #[serde(default)]
    #[serde(rename = "replyToken")]
    reply_token: Option<String>,
    #[serde(default)]
    #[serde(rename = "messageId")]
    message_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LineEventSource {
    #[serde(rename = "type")]
    source_type: String,
    #[serde(rename = "userId")]
    user_id: Option<String>,
    #[serde(rename = "groupId")]
    group_id: Option<String>,
    #[serde(rename = "roomId")]
    room_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LineEventMessage {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(default)]
    text: Option<String>,
    id: String,
}

// ---------------------------------------------------------------------------
// Line adapter
// ---------------------------------------------------------------------------

pub struct LineAdapter {
    metadata: PlatformMetadata,
    channel_access_token: String,
    channel_secret: String,
    http_client: reqwest::Client,
    webhook_port: u16,
    running: Arc<AtomicBool>,
    server_task: Option<JoinHandle<()>>,
    message_handler: HandlerRef,
}

impl LineAdapter {
    pub fn new(channel_access_token: String, channel_secret: String, webhook_port: u16) -> Self {
        let metadata = PlatformMetadata {
            id: "line".to_string(),
            name: "Line".to_string(),
            platform_type: PlatformType::Line,
            enabled: true,
            extra: HashMap::new(),
        };

        Self {
            metadata,
            channel_access_token,
            channel_secret,
            http_client: reqwest::Client::new(),
            webhook_port,
            running: Arc::new(AtomicBool::new(false)),
            server_task: None,
            message_handler: None,
        }
    }

    async fn send_reply(&self, reply_token: &str, text: &str) -> Result<()> {
        let body = LineReplyRequest {
            reply_token: reply_token.to_string(),
            messages: vec![LineMessage::Text {
                text: text.to_string(),
            }],
        };

        let resp = self.http_client
            .post("https://api.line.me/v2/bot/message/reply")
            .header("Authorization", format!("Bearer {}", self.channel_access_token))
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Line reply: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "line".to_string(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        Ok(())
    }

    async fn send_push(&self, user_id: &str, text: &str) -> Result<()> {
        let body = serde_json::json!({
            "to": user_id,
            "messages": [{ "type": "text", "text": text }],
        });

        let resp = self.http_client
            .post("https://api.line.me/v2/bot/message/push")
            .header("Authorization", format!("Bearer {}", self.channel_access_token))
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Line push: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "line".to_string(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        Ok(())
    }
}

#[async_trait]
impl PlatformAdapter for LineAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!("Line adapter initialized");
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        self.running.store(true, Ordering::SeqCst);

        let token = self.channel_access_token.clone();
        let secret = self.channel_secret.clone();
        let handler = self.message_handler.clone();
        let client = self.http_client.clone();
        let port = self.webhook_port;

        let app = Router::new()
            .route("/webhook", post(line_webhook_handler))
            .with_state((token, secret, handler, client));

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = tokio::net::TcpListener::bind(&addr).await
            .map_err(|e| AstrBotError::Network(format!("Line bind failed: {}", e)))?;

        let task = tokio::spawn(async move {
            info!("Line webhook server listening on {}", addr);
            let server = axum::serve(listener, app);
            if let Err(e) = server.await {
                error!("Line server error: {}", e);
            }
        });

        self.server_task = Some(task);
        info!("Line adapter started on port {}", port);
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        if let Some(task) = self.server_task.take() {
            task.abort();
        }
        info!("Line adapter stopped");
        Ok(())
    }

    async fn send_message(&self, target: &MessageSource, chain: &MessageChain) -> Result<()> {
        let content = chain.plain_text();
        self.send_push(&target.user_id, &content).await
    }

    async fn reply_message(&self, original: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
        let content = chain.plain_text();
        self.send_push(&original.sender.user_id, &content).await
    }

    async fn health_check(&self) -> Result<bool> {
        let resp = self.http_client
            .get("https://api.line.me/v2/bot/info")
            .header("Authorization", format!("Bearer {}", self.channel_access_token))
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

// ---------------------------------------------------------------------------
// Webhook handler
// ---------------------------------------------------------------------------

async fn line_webhook_handler(
    State((token, _secret, handler, client)): State<(String, String, HandlerRef, reqwest::Client)>,
    axum::Json(payload): axum::Json<LineWebhookPayload>,
) -> StatusCode {
    for event in payload.events {
        if event.event_type == "message" {
            if let Some(ref msg) = event.message {
                if msg.msg_type == "text" {
                    let session_id = event.source.group_id
                        .clone()
                        .or_else(|| event.source.room_id.clone())
                        .or_else(|| event.source.user_id.clone())
                        .unwrap_or_default();

                    let user_id = event.source.user_id.clone().unwrap_or_default();
                    let platform = PlatformType::Line;

                    let source = MessageSource {
                        platform,
                        session_id: session_id.clone(),
                        message_id: msg.id.clone(),
                        user_id: user_id.clone(),
                    };

                    let member = MessageMember {
                        user_id: user_id.clone(),
                        nickname: None,
                        card: None,
                        role: None,
                        is_self: false,
                    };

                    let message = AstrBotMessage {
                        message_id: msg.id.clone(),
                        timestamp: Utc::now(),
                        platform,
                        session_id,
                        sender: member,
                        message_type: match event.source.source_type.as_str() {
                            "user" => MessageType::Private,
                            _ => MessageType::Group,
                        },
                        chain: MessageChain::new().text(msg.text.as_deref().unwrap_or("")),
                        raw_payload: Some(serde_json::to_value(&event).unwrap_or_default()),
                    };

                    if let Some(ref h) = handler {
                        h.on_message(message).await;
                    }
                }
            }
        }
    }

    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_line_adapter_new() {
        let adapter = LineAdapter::new(
            "token_test".to_string(),
            "secret_test".to_string(),
            9999,
        );
        assert_eq!(adapter.metadata.name, "Line");
        assert!(adapter.metadata.enabled);
    }
}
