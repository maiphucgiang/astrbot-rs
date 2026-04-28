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
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// QQ Official API models
// https://bot.q.qq.com
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct QQAccessTokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    token_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct QQSendMessageRequest {
    #[serde(rename = "msg_type")]
    msg_type: i32,
    content: String,
}

#[derive(Debug, Clone, Deserialize)]
struct QQWebhookPayload {
    #[serde(default)]
    #[serde(rename = "op")]
    op: Option<i32>,
    #[serde(default)]
    d: Option<Value>,
    #[serde(default)]
    #[serde(rename = "t")]
    event_type: Option<String>,
}

// ---------------------------------------------------------------------------
// QQ shared state
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct QQOfficialShared {
    app_id: String,
    app_secret: String,
    http_client: reqwest::Client,
    token_cache: Arc<RwLock<Option<(String, i64)>>>,
    message_handler: HandlerRef,
}

impl QQOfficialShared {
    async fn get_access_token(&self) -> Result<String> {
        {
            let cache = self.token_cache.read().await;
            if let Some((token, expire)) = cache.as_ref() {
                let now = chrono::Utc::now().timestamp();
                if now < *expire - 60 {
                    return Ok(token.clone());
                }
            }
        }

        let resp = self.http_client
            .post("https://bots.qq.com/app/getAppAccessToken")
            .json(&serde_json::json!({
                "appId": self.app_id,
                "clientSecret": self.app_secret,
            }))
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("QQ token request: {}", e)))?;

        let data: QQAccessTokenResponse = resp.json().await
            .map_err(|e| AstrBotError::Serialization(format!("QQ token parse: {}", e)))?;

        let expire_at = chrono::Utc::now().timestamp() + data.expires_in.unwrap_or(7200);

        {
            let mut cache = self.token_cache.write().await;
            *cache = Some((data.access_token.clone(), expire_at));
        }

        Ok(data.access_token)
    }

    async fn send_message(&self, channel_id: &str, content: &str) -> Result<()> {
        let token = self.get_access_token().await?;

        let body = QQSendMessageRequest {
            msg_type: 0, // text
            content: content.to_string(),
        };

        let resp = self.http_client
            .post(format!("https://api.sgroup.qq.com/channels/{}/messages", channel_id))
            .header("Authorization", format!("QQBot {}", token))
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("QQ send message: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "qq_official".to_string(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// QQ Official adapter
// ---------------------------------------------------------------------------

pub struct QQOfficialAdapter {
    metadata: PlatformMetadata,
    shared: Arc<QQOfficialShared>,
    webhook_port: u16,
    running: Arc<AtomicBool>,
    server_task: Option<JoinHandle<()>>,
}

impl QQOfficialAdapter {
    pub fn new(app_id: String, app_secret: String, webhook_port: u16) -> Self {
        let metadata = PlatformMetadata {
            id: "qq_official".to_string(),
            name: "QQ Official".to_string(),
            platform_type: PlatformType::QqOfficial,
            enabled: true,
            extra: HashMap::new(),
        };

        let shared = Arc::new(QQOfficialShared {
            app_id,
            app_secret,
            http_client: reqwest::Client::new(),
            token_cache: Arc::new(RwLock::new(None)),
            message_handler: None,
        });

        Self {
            metadata,
            shared,
            webhook_port,
            running: Arc::new(AtomicBool::new(false)),
            server_task: None,
        }
    }
}

#[async_trait]
impl PlatformAdapter for QQOfficialAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        let _ = self.shared.get_access_token().await?;
        info!("QQ Official adapter initialized — access_token obtained");
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        self.running.store(true, Ordering::SeqCst);

        let shared = Arc::clone(&self.shared);
        let port = self.webhook_port;

        let app = Router::new()
            .route("/webhook", post(qq_webhook_handler))
            .with_state(shared);

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = tokio::net::TcpListener::bind(&addr).await
            .map_err(|e| AstrBotError::Network(format!("QQ bind failed: {}", e)))?;

        let task = tokio::spawn(async move {
            info!("QQ Official webhook server listening on {}", addr);
            let server = axum::serve(listener, app);
            if let Err(e) = server.await {
                error!("QQ server error: {}", e);
            }
        });

        self.server_task = Some(task);
        info!("QQ Official adapter started on port {}", port);
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        if let Some(task) = self.server_task.take() {
            task.abort();
        }
        info!("QQ Official adapter stopped");
        Ok(())
    }

    async fn send_message(&self, target: &MessageSource, chain: &MessageChain) -> Result<()> {
        let content = chain.plain_text();
        self.shared.send_message(&target.session_id, &content).await
    }

    async fn reply_message(&self, original: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
        let content = chain.plain_text();
        self.shared.send_message(&original.session_id, &content).await
    }

    async fn health_check(&self) -> Result<bool> {
        match self.shared.get_access_token().await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    fn set_message_handler(&mut self, handler: Arc<dyn MessageHandler>) {
        if let Some(shared_mut) = Arc::get_mut(&mut self.shared) {
            shared_mut.message_handler = Some(handler);
        }
    }
}

// ---------------------------------------------------------------------------
// Webhook handler
// ---------------------------------------------------------------------------

async fn qq_webhook_handler(
    State(_shared): State<Arc<QQOfficialShared>>,
    axum::Json(_payload): axum::Json<QQWebhookPayload>,
) -> StatusCode {
    // QQ Bot webhooks need signature verification.
    // Skeleton — implement ed25519 signature verification in production.
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_qq_official_adapter_new() {
        let adapter = QQOfficialAdapter::new(
            "app_test".to_string(),
            "secret_test".to_string(),
            9999,
        );
        assert_eq!(adapter.metadata.name, "QQ Official");
        assert!(adapter.metadata.enabled);
    }
}
