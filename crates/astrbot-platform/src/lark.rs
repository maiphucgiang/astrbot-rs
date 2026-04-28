use async_trait::async_trait;
use axum::{routing::post, Router};
use axum::extract::State;
use axum::http::StatusCode;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::message::{AstrBotMessage, MessageChain, MessageComponent, MessageMember, MessageType, HandlerRef, MessageHandler};
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use crate::adapter::PlatformAdapter;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// Feishu / Lark API models
// https://open.feishu.cn
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct LarkCallbackPayload {
    #[serde(default)]
    schema: Option<String>,
    #[serde(default)]
    header: Option<LarkEventHeader>,
    #[serde(default)]
    event: Option<Value>,
    #[serde(default)]
    challenge: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct LarkEventHeader {
    #[serde(rename = "event_id")]
    event_id: String,
    #[serde(rename = "event_type")]
    event_type: String,
    #[serde(rename = "create_time")]
    create_time: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LarkMessageEvent {
    message: LarkMessage,
    sender: LarkSender,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LarkMessage {
    #[serde(rename = "message_id")]
    message_id: String,
    #[serde(rename = "chat_id")]
    chat_id: String,
    #[serde(rename = "chat_type")]
    chat_type: String,
    #[serde(rename = "message_type")]
    message_type: String,
    content: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LarkSender {
    #[serde(rename = "sender_id")]
    sender_id: LarkSenderId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LarkSenderId {
    #[serde(rename = "open_id")]
    open_id: String,
}

#[derive(Debug, Clone, Serialize)]
struct LarkTenantAccessTokenRequest {
    #[serde(rename = "app_id")]
    app_id: String,
    #[serde(rename = "app_secret")]
    app_secret: String,
}

#[derive(Debug, Clone, Deserialize)]
struct LarkTenantAccessTokenResponse {
    code: i32,
    #[serde(default)]
    tenant_access_token: Option<String>,
    #[serde(default)]
    expire: Option<i64>,
    #[serde(default)]
    msg: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct LarkSendMessageRequest {
    #[serde(rename = "receive_id")]
    receive_id: String,
    #[serde(rename = "msg_type")]
    msg_type: String,
    content: String,
}

// ---------------------------------------------------------------------------
// Shared state — safe to clone into Axum handlers
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct LarkShared {
    app_id: String,
    app_secret: String,
    http_client: reqwest::Client,
    token_cache: Arc<RwLock<Option<(String, i64)>>>, // (token, expire_at_ms)
    message_handler: HandlerRef,
}

impl LarkShared {
    async fn get_tenant_access_token(&self) -> Result<String> {
        // Check cache first
        {
            let cache = self.token_cache.read().await;
            if let Some((token, expire)) = cache.as_ref() {
                let now = chrono::Utc::now().timestamp_millis();
                if now < *expire - 60000 {
                    return Ok(token.clone());
                }
            }
        }

        let body = LarkTenantAccessTokenRequest {
            app_id: self.app_id.clone(),
            app_secret: self.app_secret.clone(),
        };

        let resp = self.http_client
            .post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Lark token request failed: {}", e)))?;

        let data: LarkTenantAccessTokenResponse = resp.json().await
            .map_err(|e| AstrBotError::Serialization(format!("Lark token parse failed: {}", e)))?;

        if data.code != 0 {
            return Err(AstrBotError::Platform {
                adapter: "lark".to_string(),
                message: format!("Lark token error {}: {}", data.code, data.msg.unwrap_or_default()),
            });
        }

        let token = data.tenant_access_token
            .ok_or_else(|| AstrBotError::Platform {
                adapter: "lark".to_string(),
                message: "No tenant_access_token in response".to_string(),
            })?;

        let expire_at = chrono::Utc::now().timestamp_millis() + (data.expire.unwrap_or(7200) * 1000);

        {
            let mut cache = self.token_cache.write().await;
            *cache = Some((token.clone(), expire_at));
        }

        Ok(token)
    }

    async fn send_lark_message(&self, receive_id: &str, content: &str) -> Result<()> {
        let token = self.get_tenant_access_token().await?;

        let body = LarkSendMessageRequest {
            receive_id: receive_id.to_string(),
            msg_type: "text".to_string(),
            content: serde_json::json!({"text": content}).to_string(),
        };

        let resp = self.http_client
            .post("https://open.feishu.cn/open-apis/im/v1/messages")
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Lark send message failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "lark".to_string(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        Ok(())
    }

    fn parse_message_content(content: &str) -> MessageChain {
        if let Ok(val) = serde_json::from_str::<Value>(content) {
            if let Some(text) = val.get("text").and_then(|v| v.as_str()) {
                return MessageChain::new().text(text);
            }
        }
        MessageChain::new().text(content)
    }

    async fn handle_callback(&self, payload: LarkCallbackPayload) -> Result<Value> {
        // URL verification challenge
        if let Some(challenge) = payload.challenge {
            return Ok(serde_json::json!({"challenge": challenge}));
        }

        if let Some(header) = payload.header {
            match header.event_type.as_str() {
                "im.message.receive_v1" => {
                    if let Some(event) = payload.event {
                        if let Ok(msg_event) = serde_json::from_value::<LarkMessageEvent>(event) {
                            let platform = PlatformType::Feishu;
                            let source = MessageSource {
                                platform,
                                session_id: msg_event.message.chat_id.clone(),
                                message_id: msg_event.message.message_id.clone(),
                                user_id: msg_event.sender.sender_id.open_id.clone(),
                            };
                            let member = MessageMember {
                                user_id: msg_event.sender.sender_id.open_id.clone(),
                                nickname: None,
                                card: None,
                                role: None,
                                is_self: false,
                            };
                            let chain = Self::parse_message_content(&msg_event.message.content);
                            let message_type = match msg_event.message.chat_type.as_str() {
                                "p2p" => MessageType::Private,
                                _ => MessageType::Group,
                            };
                            let message = AstrBotMessage {
                                message_id: msg_event.message.message_id.clone(),
                                timestamp: Utc::now(),
                                platform,
                                session_id: msg_event.message.chat_id.clone(),
                                sender: member,
                                message_type,
                                chain,
                                raw_payload: Some(serde_json::to_value(&msg_event).unwrap_or_default()),
                            };
                            if let Some(ref h) = self.message_handler {
                                h.on_message(message).await;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(serde_json::json!({"code": 0}))
    }
}

// ---------------------------------------------------------------------------
// Lark adapter
// ---------------------------------------------------------------------------

pub struct LarkAdapter {
    metadata: PlatformMetadata,
    shared: Arc<LarkShared>,
    callback_port: u16,
    running: Arc<AtomicBool>,
    server_task: Option<JoinHandle<()>>,
}

impl LarkAdapter {
    pub fn new(app_id: String, app_secret: String, callback_port: u16) -> Self {
        let metadata = PlatformMetadata {
            id: "lark".to_string(),
            name: "Lark / Feishu".to_string(),
            platform_type: PlatformType::Feishu,
            enabled: true,
            extra: HashMap::new(),
        };

        let shared = Arc::new(LarkShared {
            app_id,
            app_secret,
            http_client: reqwest::Client::new(),
            token_cache: Arc::new(RwLock::new(None)),
            message_handler: None,
        });

        Self {
            metadata,
            shared,
            callback_port,
            running: Arc::new(AtomicBool::new(false)),
            server_task: None,
        }
    }
}

#[async_trait]
impl PlatformAdapter for LarkAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        let _ = self.shared.get_tenant_access_token().await?;
        info!("Lark adapter initialized — tenant_access_token obtained");
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        self.running.store(true, Ordering::SeqCst);

        let shared = Arc::clone(&self.shared);
        let port = self.callback_port;

        let app = Router::new()
            .route("/callback", post(lark_callback_handler))
            .with_state(shared);

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = tokio::net::TcpListener::bind(&addr).await
            .map_err(|e| AstrBotError::Network(format!("Lark bind failed: {}", e)))?;

        let task = tokio::spawn(async move {
            info!("Lark callback server listening on {}", addr);
            let server = axum::serve(listener, app);
            if let Err(e) = server.await {
                error!("Lark server error: {}", e);
            }
        });

        self.server_task = Some(task);
        info!("Lark adapter started on port {}", port);
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        if let Some(task) = self.server_task.take() {
            task.abort();
        }
        info!("Lark adapter stopped");
        Ok(())
    }

    async fn send_message(&self, target: &MessageSource, chain: &MessageChain) -> Result<()> {
        let content = chain.plain_text();
        self.shared.send_lark_message(&target.session_id, &content).await
    }

    async fn reply_message(&self, original: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
        let content = chain.plain_text();
        self.shared.send_lark_message(&original.session_id, &content).await
    }

    async fn health_check(&self) -> Result<bool> {
        match self.shared.get_tenant_access_token().await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    fn set_message_handler(&mut self, handler: Arc<dyn MessageHandler>) {
        // Need to update the shared state — use unsafe for now, or restructure
        // SAFETY: This is called before start(), so no concurrent access
        let shared_mut = Arc::get_mut(&mut self.shared)
            .expect("set_message_handler must be called before start() when shared has only one owner");
        shared_mut.message_handler = Some(handler);
    }
}

// ---------------------------------------------------------------------------
// Axum handler
// ---------------------------------------------------------------------------

async fn lark_callback_handler(
    State(shared): State<Arc<LarkShared>>,
    axum::Json(payload): axum::Json<LarkCallbackPayload>,
) -> (StatusCode, axum::Json<Value>) {
    match shared.handle_callback(payload).await {
        Ok(response) => (StatusCode::OK, axum::Json(response)),
        Err(e) => {
            error!("Lark callback error: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"code": -1, "msg": e.to_string()})))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lark_parse_message_content_json() {
        let chain = LarkShared::parse_message_content(r#"{"text":"hello"}"#);
        assert_eq!(chain.plain_text(), "hello");
    }

    #[test]
    fn test_lark_parse_message_content_plain() {
        let chain = LarkShared::parse_message_content("hello world");
        assert_eq!(chain.plain_text(), "hello world");
    }

    #[tokio::test]
    async fn test_lark_adapter_new() {
        let adapter = LarkAdapter::new(
            "test_app_id".to_string(),
            "test_secret".to_string(),
            8888,
        );
        assert_eq!(adapter.metadata.name, "Lark / Feishu");
        assert!(adapter.metadata.enabled);
    }
}
