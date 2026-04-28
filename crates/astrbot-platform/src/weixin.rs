//! WeChat Official Account (公众号) platform adapter
//!
//! Supports HTTP callback mode for receiving messages.
//! Reference: https://developers.weixin.qq.com/doc/offiaccount/en/Getting_Started/Overview.html

use async_trait::async_trait;
use axum::{extract::Query, routing::{get, post}, Router};
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
use sha1::Digest;

// ---------------------------------------------------------------------------
// WeChat API models
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct WeixinTokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    errcode: Option<i32>,
    #[serde(default)]
    errmsg: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct WeixinSendMessageRequest {
    touser: String,
    msgtype: String,
    text: WeixinTextContent,
}

#[derive(Debug, Clone, Serialize)]
struct WeixinTextContent {
    content: String,
}

/// Query parameters for GET verification (echostr)
#[derive(Debug, Clone, Deserialize)]
struct WeixinVerifyQuery {
    signature: String,
    timestamp: String,
    nonce: String,
    #[serde(default)]
    echostr: Option<String>,
}

/// XML-like message body (WeChat posts XML, but we accept JSON for simplicity
/// or parse from forwarded JSON. For full XML support a body parser layer is needed.)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WeixinMessagePayload {
    #[serde(default)]
    #[serde(rename = "MsgType")]
    msg_type: Option<String>,
    #[serde(default)]
    #[serde(rename = "FromUserName")]
    from_user_name: Option<String>,
    #[serde(default)]
    #[serde(rename = "ToUserName")]
    to_user_name: Option<String>,
    #[serde(default)]
    #[serde(rename = "Content")]
    content: Option<String>,
    #[serde(default)]
    #[serde(rename = "MsgId")]
    msg_id: Option<String>,
    #[serde(default)]
    #[serde(rename = "CreateTime")]
    create_time: Option<String>,
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct WeixinShared {
    app_id: String,
    app_secret: String,
    token: String,
    #[allow(dead_code)]
    aes_key: Option<String>,
    #[allow(dead_code)]
    base_url: Option<String>,
    http_client: reqwest::Client,
    token_cache: Arc<RwLock<Option<(String, i64)>>>, // (access_token, expire_at_unix)
    message_handler: HandlerRef,
}

impl WeixinShared {
    /// Verify WeChat signature for URL verification or message validation.
    /// Algorithm: sort(token, timestamp, nonce) lexicographically,
    /// concatenate, SHA1 digest, compare with signature.
    fn verify_signature(token: &str, timestamp: &str, nonce: &str, signature: &str) -> bool {
        let mut parts = vec![token, timestamp, nonce];
        parts.sort();
        let joined = parts.concat();
        let hash = format!("{:x}", sha1::Sha1::digest(joined.as_bytes()));
        hash.eq_ignore_ascii_case(signature)
    }

    async fn get_access_token(&self) -> Result<String> {
        // Check cache
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
            .get("https://api.weixin.qq.com/cgi-bin/token")
            .query(&[
                ("grant_type", "client_credential"),
                ("appid", self.app_id.as_str()),
                ("secret", self.app_secret.as_str()),
            ])
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Weixin token request: {}", e)))?;

        let data: WeixinTokenResponse = resp.json().await
            .map_err(|e| AstrBotError::Serialization(format!("Weixin token parse: {}", e)))?;

        if let Some(errcode) = data.errcode {
            if errcode != 0 {
                return Err(AstrBotError::Platform {
                    adapter: "weixin".to_string(),
                    message: format!("Weixin token error {}: {}", errcode, data.errmsg.unwrap_or_default()),
                });
            }
        }

        let token = data.access_token
            .ok_or_else(|| AstrBotError::Platform {
                adapter: "weixin".to_string(),
                message: "No access_token in response".to_string(),
            })?;

        let expire_at = chrono::Utc::now().timestamp() + data.expires_in.unwrap_or(7200);

        {
            let mut cache = self.token_cache.write().await;
            *cache = Some((token.clone(), expire_at));
        }

        Ok(token)
    }

    async fn send_message(&self, openid: &str, content: &str) -> Result<()> {
        let token = self.get_access_token().await?;

        let body = WeixinSendMessageRequest {
            touser: openid.to_string(),
            msgtype: "text".to_string(),
            text: WeixinTextContent {
                content: content.to_string(),
            },
        };

        let resp = self.http_client
            .post(format!(
                "https://api.weixin.qq.com/cgi-bin/message/custom/send?access_token={}",
                token
            ))
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Weixin send message: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "weixin".to_string(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        Ok(())
    }

    async fn handle_verification(&self, query: WeixinVerifyQuery) -> Result<String> {
        if Self::verify_signature(&self.token, &query.timestamp, &query.nonce, &query.signature) {
            Ok(query.echostr.unwrap_or_default())
        } else {
            Err(AstrBotError::Platform {
                adapter: "weixin".to_string(),
                message: "Signature verification failed".to_string(),
            })
        }
    }

    async fn handle_message(&self, payload: WeixinMessagePayload) -> Result<()> {
        let from_user = payload.from_user_name
            .clone()
            .unwrap_or_default();
        let content = payload.content.clone().unwrap_or_default();
        let msg_id = payload.msg_id.clone().unwrap_or_default();

        if from_user.is_empty() || content.is_empty() {
            return Ok(());
        }

        let platform = PlatformType::Weixin;
        let source = MessageSource {
            platform,
            session_id: from_user.clone(),
            message_id: msg_id.clone(),
            user_id: from_user.clone(),
        };
        let member = MessageMember {
            user_id: from_user.clone(),
            nickname: None,
            card: None,
            role: None,
            is_self: false,
        };
        let chain = MessageChain::new().text(&content);

        let message = AstrBotMessage {
            message_id: msg_id.clone(),
            timestamp: Utc::now(),
            platform,
            session_id: from_user.clone(),
            sender: member,
            message_type: MessageType::Private,
            chain,
            raw_payload: Some(serde_json::to_value(&payload).unwrap_or_default()),
        };

        if let Some(ref h) = self.message_handler {
            h.on_message(message).await;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Weixin adapter
// ---------------------------------------------------------------------------

pub struct WeixinAdapter {
    metadata: PlatformMetadata,
    shared: Arc<WeixinShared>,
    callback_port: u16,
    running: Arc<AtomicBool>,
    server_task: Option<JoinHandle<()>>,
}

impl WeixinAdapter {
    pub fn new(
        app_id: String,
        app_secret: String,
        token: String,
        aes_key: Option<String>,
        base_url: Option<String>,
        callback_port: u16,
    ) -> Self {
        let metadata = PlatformMetadata {
            id: "weixin".to_string(),
            name: "WeChat Official Account".to_string(),
            platform_type: PlatformType::Weixin,
            enabled: true,
            extra: HashMap::new(),
        };

        let shared = Arc::new(WeixinShared {
            app_id,
            app_secret,
            token,
            aes_key,
            base_url,
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
impl PlatformAdapter for WeixinAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        let _ = self.shared.get_access_token().await?;
        info!("Weixin adapter initialized — access_token obtained");
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        self.running.store(true, Ordering::SeqCst);

        let shared = Arc::clone(&self.shared);
        let port = self.callback_port;

        let app = Router::new()
            .route("/callback", get(weixin_verify_handler))
            .route("/callback", post(weixin_message_handler))
            .with_state(shared);

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = tokio::net::TcpListener::bind(&addr).await
            .map_err(|e| AstrBotError::Network(format!("Weixin bind failed: {}", e)))?;

        let task = tokio::spawn(async move {
            info!("Weixin callback server listening on {}", addr);
            let server = axum::serve(listener, app);
            if let Err(e) = server.await {
                error!("Weixin server error: {}", e);
            }
        });

        self.server_task = Some(task);
        info!("Weixin adapter started on port {}", port);
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        if let Some(task) = self.server_task.take() {
            task.abort();
        }
        info!("Weixin adapter stopped");
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
// Axum handlers
// ---------------------------------------------------------------------------

async fn weixin_verify_handler(
    State(shared): State<Arc<WeixinShared>>,
    Query(query): Query<WeixinVerifyQuery>,
) -> (StatusCode, String) {
    match shared.handle_verification(query).await {
        Ok(echostr) => {
            if echostr.is_empty() {
                (StatusCode::OK, String::new())
            } else {
                (StatusCode::OK, echostr)
            }
        }
        Err(e) => {
            warn!("Weixin verification failed: {}", e);
            (StatusCode::FORBIDDEN, String::new())
        }
    }
}

async fn weixin_message_handler(
    State(shared): State<Arc<WeixinShared>>,
    axum::Json(payload): axum::Json<WeixinMessagePayload>,
) -> StatusCode {
    match shared.handle_message(payload).await {
        Ok(()) => StatusCode::OK,
        Err(e) => {
            error!("Weixin message handler error: {}", e);
            StatusCode::OK // WeChat expects 200 even on errors
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_weixin_adapter_new() {
        let adapter = WeixinAdapter::new(
            "test_app_id".to_string(),
            "test_app_secret".to_string(),
            "test_token".to_string(),
            None,
            None,
            9999,
        );
        assert_eq!(adapter.metadata.name, "WeChat Official Account");
        assert!(adapter.metadata.enabled);
    }
}
