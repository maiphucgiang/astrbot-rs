use crate::adapter::PlatformAdapter;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::message::{
    AstrBotMessage, HandlerRef, MessageChain, MessageComponent, MessageHandler, MessageMember,
    MessageType,
};
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use async_trait::async_trait;
use axum::extract::State;
use axum::http::StatusCode;
use axum::{routing::post, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// WeChat Personal API models
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct WechatSendRequest {
    to: String,
    content: String,
}

#[derive(Debug, Clone, Deserialize)]
struct WechatWebhookPayload {
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub msg_id: Option<String>,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub is_group: Option<bool>,
}

// ---------------------------------------------------------------------------
// WeChat Personal shared state
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct WechatShared {
    bridge_endpoint: String,
    api_token: Option<String>,
    http_client: reqwest::Client,
    message_handler: Arc<std::sync::Mutex<HandlerRef>>,
}

impl WechatShared {
    async fn send_wechat_message(&self, to: &str, text: &str) -> Result<()> {
        let url = format!("{}/message/send", self.bridge_endpoint);

        let body = WechatSendRequest {
            to: to.to_string(),
            content: text.to_string(),
        };

        let mut req = self.http_client.post(&url).json(&body);

        if let Some(ref token) = self.api_token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }

        let resp = req
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("WeChat send message failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "WechatPersonal".to_string(),
                message: format!("WeChat API error: {} - {}", status, text),
            });
        }

        info!("[WechatPersonal] Message sent to {}", to);
        Ok(())
    }

    fn parse_webhook_payload(&self, payload: WechatWebhookPayload) -> Option<AstrBotMessage> {
        let from = payload.from?;
        let content = payload.content?;
        let msg_id = payload.msg_id.unwrap_or_else(|| "0".to_string());
        let is_group = payload.is_group.unwrap_or(false);
        let group_id = payload.group_id;

        let chain = MessageChain::new().text(&content);
        let member = MessageMember {
            user_id: from.clone(),
            nickname: Some(from.clone()),
            card: None,
            role: None,
            is_self: false,
        };

        let message_type = if is_group {
            MessageType::Group
        } else {
            MessageType::Private
        };

        Some(AstrBotMessage {
            message_id: msg_id,
            timestamp: Utc::now(),
            platform: PlatformType::WechatPersonal,
            session_id: group_id.unwrap_or_else(|| from.clone()),
            sender: member,
            message_type,
            chain,
            raw_payload: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Webhook handler
// ---------------------------------------------------------------------------

async fn wechat_webhook_handler(
    State(shared): State<Arc<WechatShared>>,
    axum::Json(payload): axum::Json<WechatWebhookPayload>,
) -> StatusCode {
    if let Some(msg) = shared.parse_webhook_payload(payload) {
        let guard = shared.message_handler.lock().unwrap();
        if let Some(ref handler) = *guard {
            let handler_clone = handler.clone();
            let msg_clone = msg.clone();
            tokio::spawn(async move {
                handler_clone.on_message(msg_clone).await;
            });
        }
    }
    StatusCode::OK
}

// ---------------------------------------------------------------------------
// WeChat Personal adapter
// ---------------------------------------------------------------------------

pub struct WechatPersonalAdapter {
    metadata: PlatformMetadata,
    shared: Arc<WechatShared>,
    webhook_port: u16,
    running: Arc<AtomicBool>,
    server_task: Mutex<Option<JoinHandle<()>>>,
}

impl WechatPersonalAdapter {
    pub fn new(
        id: String,
        bridge_endpoint: String,
        api_token: Option<String>,
        webhook_port: u16,
    ) -> Self {
        let metadata = PlatformMetadata {
            id: id.clone(),
            name: format!("WeChat Personal {}", id),
            platform_type: PlatformType::WechatPersonal,
            enabled: true,
            extra: {
                let mut map = HashMap::new();
                map.insert(
                    "bridge_endpoint".to_string(),
                    serde_json::Value::String(bridge_endpoint.clone()),
                );
                map.insert(
                    "webhook_port".to_string(),
                    serde_json::Value::Number(webhook_port.into()),
                );
                map
            },
        };
        let shared = Arc::new(WechatShared {
            bridge_endpoint,
            api_token,
            http_client: reqwest::Client::new(),
            message_handler: Arc::new(std::sync::Mutex::new(None)),
        });
        Self {
            metadata,
            shared,
            webhook_port,
            running: Arc::new(AtomicBool::new(false)),
            server_task: Mutex::new(None),
        }
    }
}

#[async_trait]
impl PlatformAdapter for WechatPersonalAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!("[WechatPersonal] Initializing adapter...");
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        self.running.store(true, Ordering::Relaxed);

        let shared = Arc::clone(&self.shared);
        let port = self.webhook_port;
        let app = Router::new()
            .route("/webhook", post(wechat_webhook_handler))
            .with_state(shared);
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| AstrBotError::Network(format!("WechatPersonal bind failed: {}", e)))?;
        let running = Arc::clone(&self.running);
        let handle = tokio::spawn(async move {
            info!("[WechatPersonal] Webhook server listening on {}", addr);
            let server = axum::serve(listener, app);
            if let Err(e) = server.await {
                error!("[WechatPersonal] Server error: {}", e);
            }
        });
        let mut guard = self.server_task.lock().await;
        *guard = Some(handle);
        info!("[WechatPersonal] Adapter started on port {}", port);
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("[WechatPersonal] Stopping adapter...");
        self.running.store(false, Ordering::Relaxed);
        let mut guard = self.server_task.lock().await;
        if let Some(handle) = guard.take() {
            let _ = handle.await;
        }
        info!("[WechatPersonal] Adapter stopped");
        Ok(())
    }

    async fn send_message(&self, target: &MessageSource, chain: &MessageChain) -> Result<()> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(AstrBotError::Platform {
                adapter: "WechatPersonal".to_string(),
                message: "adapter not running".to_string(),
            });
        }
        let text = chain.plain_text();
        if text.is_empty() {
            return Ok(());
        }
        let to = &target.user_id;
        self.shared.send_wechat_message(to, &text).await
    }

    async fn reply_message(&self, original: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
        let source = MessageSource {
            platform: PlatformType::WechatPersonal,
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
        let mut guard = self.shared.message_handler.lock().unwrap();
        *guard = Some(handler);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "binds to network port; may conflict or hang in CI"]
    async fn test_wechat_personal_lifecycle() {
        let mut adapter = WechatPersonalAdapter::new(
            "wx-personal-1".to_string(),
            "http://localhost:8080".to_string(),
            None,
            0,
        );
        assert_eq!(
            adapter.metadata().platform_type,
            PlatformType::WechatPersonal
        );
        adapter.initialize().await.unwrap();
        adapter.start().await.unwrap();
        assert!(adapter.health_check().await.unwrap());
        adapter.stop().await.unwrap();
    }

    #[test]
    fn test_wechat_send_request_serialize() {
        let req = WechatSendRequest {
            to: "wxid_abc123".to_string(),
            content: "Hello WeChat".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("wxid_abc123"));
        assert!(json.contains("Hello WeChat"));
    }

    #[test]
    fn test_wechat_webhook_payload_parse() {
        let json = r#"{
            "from": "wxid_user123",
            "content": "Hello bot",
            "msg_id": "msg_12345",
            "is_group": false
        }"#;
        let payload: WechatWebhookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.from, Some("wxid_user123".to_string()));
        assert_eq!(payload.content, Some("Hello bot".to_string()));
        assert_eq!(payload.msg_id, Some("msg_12345".to_string()));
        assert_eq!(payload.is_group, Some(false));
    }
}
