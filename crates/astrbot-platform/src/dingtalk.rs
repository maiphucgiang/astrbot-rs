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
// DingTalk API models
// https://open.dingtalk.com
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
struct DingTalkTokenRequest {
    #[serde(rename = "appkey")]
    appkey: String,
    #[serde(rename = "appsecret")]
    appsecret: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DingTalkTokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    errcode: Option<i64>,
    #[serde(default)]
    errmsg: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct DingTalkSendMessageRequest {
    #[serde(rename = "agent_id")]
    agent_id: String,
    #[serde(rename = "userid_list")]
    userid_list: String,
    #[serde(default)]
    #[serde(rename = "dept_id_list")]
    dept_id_list: Option<String>,
    #[serde(default)]
    #[serde(rename = "to_all_user")]
    to_all_user: Option<bool>,
    msg: DingTalkMessageContent,
}

#[derive(Debug, Clone, Serialize)]
struct DingTalkMessageContent {
    #[serde(rename = "msgtype")]
    msg_type: String,
    text: DingTalkTextContent,
}

#[derive(Debug, Clone, Serialize)]
struct DingTalkTextContent {
    content: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DingTalkCallbackPayload {
    #[serde(default)]
    #[serde(rename = "msg_signature")]
    msg_signature: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    encrypt: Option<String>,
}

// ---------------------------------------------------------------------------
// DingTalk shared state
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct DingTalkShared {
    app_key: String,
    app_secret: String,
    agent_id: String,
    http_client: reqwest::Client,
    token_cache: Arc<RwLock<Option<(String, i64)>>>,
    message_handler: HandlerRef,
}

impl DingTalkShared {
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
            .get("https://oapi.dingtalk.com/gettoken")
            .query(&[
                ("appkey", self.app_key.as_str()),
                ("appsecret", self.app_secret.as_str()),
            ])
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("DingTalk token request: {}", e)))?;

        let data: DingTalkTokenResponse = resp.json().await
            .map_err(|e| AstrBotError::Serialization(format!("DingTalk token parse: {}", e)))?;

        if let Some(errcode) = data.errcode {
            if errcode != 0 {
                return Err(AstrBotError::Platform {
                    adapter: "dingtalk".to_string(),
                    message: format!("DingTalk token error {}: {}", errcode, data.errmsg.unwrap_or_default()),
                });
            }
        }

        let token = data.access_token
            .ok_or_else(|| AstrBotError::Platform {
                adapter: "dingtalk".to_string(),
                message: "No access_token in response".to_string(),
            })?;

        let expire_at = chrono::Utc::now().timestamp() + data.expires_in.unwrap_or(7200);

        {
            let mut cache = self.token_cache.write().await;
            *cache = Some((token.clone(), expire_at));
        }

        Ok(token)
    }

    async fn send_message(
        &self,
        user_id: &str,
        content: &str,
    ) -> Result<()> {
        let token = self.get_access_token().await?;

        let body = DingTalkSendMessageRequest {
            agent_id: self.agent_id.clone(),
            userid_list: user_id.to_string(),
            dept_id_list: None,
            to_all_user: None,
            msg: DingTalkMessageContent {
                msg_type: "text".to_string(),
                text: DingTalkTextContent {
                    content: content.to_string(),
                },
            },
        };

        let resp = self.http_client
            .post(format!("https://oapi.dingtalk.com/topapi/message/corpconversation/asyncsend_v2?access_token={}", token))
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("DingTalk send message: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "dingtalk".to_string(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        Ok(())
    }

    fn parse_message_content(content: &str) -> MessageChain {
        MessageChain::new().text(content)
    }
}

// ---------------------------------------------------------------------------
// DingTalk adapter
// ---------------------------------------------------------------------------

pub struct DingTalkAdapter {
    metadata: PlatformMetadata,
    shared: Arc<DingTalkShared>,
    callback_port: u16,
    running: Arc<AtomicBool>,
    server_task: Option<JoinHandle<()>>,
}

impl DingTalkAdapter {
    pub fn new(app_key: String, app_secret: String, agent_id: String, callback_port: u16) -> Self {
        let metadata = PlatformMetadata {
            id: "dingtalk".to_string(),
            name: "DingTalk".to_string(),
            platform_type: PlatformType::Dingtalk,
            enabled: true,
            extra: HashMap::new(),
        };

        let shared = Arc::new(DingTalkShared {
            app_key,
            app_secret,
            agent_id,
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
impl PlatformAdapter for DingTalkAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        let _ = self.shared.get_access_token().await?;
        info!("DingTalk adapter initialized — access_token obtained");
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        self.running.store(true, Ordering::SeqCst);

        let shared = Arc::clone(&self.shared);
        let port = self.callback_port;

        let app = Router::new()
            .route("/callback", post(dingtalk_callback_handler))
            .with_state(shared);

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = tokio::net::TcpListener::bind(&addr).await
            .map_err(|e| AstrBotError::Network(format!("DingTalk bind failed: {}", e)))?;

        let task = tokio::spawn(async move {
            info!("DingTalk callback server listening on {}", addr);
            let server = axum::serve(listener, app);
            if let Err(e) = server.await {
                error!("DingTalk server error: {}", e);
            }
        });

        self.server_task = Some(task);
        info!("DingTalk adapter started on port {}", port);
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        if let Some(task) = self.server_task.take() {
            task.abort();
        }
        info!("DingTalk adapter stopped");
        Ok(())
    }

    async fn send_message(&self, target: &MessageSource, chain: &MessageChain) -> Result<()> {
        let content = chain.plain_text();
        self.shared.send_message(&target.user_id, &content).await
    }

    async fn reply_message(&self, original: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
        let content = chain.plain_text();
        self.shared.send_message(&original.sender.user_id, &content).await
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
// Axum handler
// ---------------------------------------------------------------------------

async fn dingtalk_callback_handler(
    State(_shared): State<Arc<DingTalkShared>>,
    axum::Json(_payload): axum::Json<DingTalkCallbackPayload>,
) -> StatusCode {
    // DingTalk callbacks require encryption/decryption in production.
    // This is a skeleton — implement signature verification and message parsing.
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dingtalk_adapter_new() {
        let adapter = DingTalkAdapter::new(
            "test_key".to_string(),
            "test_secret".to_string(),
            "test_agent".to_string(),
            9999,
        );
        assert_eq!(adapter.metadata.name, "DingTalk");
        assert!(adapter.metadata.enabled);
    }

    #[test]
    fn test_parse_message_content() {
        let chain = DingTalkShared::parse_message_content("hello");
        assert_eq!(chain.plain_text(), "hello");
    }
}
