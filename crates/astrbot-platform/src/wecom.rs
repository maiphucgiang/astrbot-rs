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
use base64::{Engine as _, engine::general_purpose};
use cbc::cipher::{BlockDecryptMut, KeyIvInit};
use cbc::cipher::block_padding::Pkcs7;
use sha1::{Sha1, Digest};

// ---------------------------------------------------------------------------
// WeCom API models
// https://developer.work.weixin.qq.com
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct WeComTokenResponse {
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
struct WeComMessageRequest {
    #[serde(rename = "touser")]
    to_user: String,
    #[serde(rename = "msgtype")]
    msg_type: String,
    text: WeComTextContent,
    #[serde(rename = "agentid")]
    agent_id: i64,
}

#[derive(Debug, Clone, Serialize)]
struct WeComTextContent {
    content: String,
}

#[derive(Debug, Clone, Deserialize)]
struct WeComCallbackPayload {
    #[serde(default)]
    #[serde(rename = "msg_signature")]
    msg_signature: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    echostr: Option<String>,
    #[serde(default)]
    encrypt: Option<String>,
}

// ---------------------------------------------------------------------------
// WeCom shared state
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct WeComShared {
    corp_id: String,
    corp_secret: String,
    agent_id: i64,
    token: Option<String>,
    encoding_aes_key: Option<String>,
    http_client: reqwest::Client,
    token_cache: Arc<RwLock<Option<(String, i64)>>>,
    message_handler: HandlerRef,
}

impl WeComShared {
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
            .get("https://qyapi.weixin.qq.com/cgi-bin/gettoken")
            .query(&[
                ("corpid", self.corp_id.as_str()),
                ("corpsecret", self.corp_secret.as_str()),
            ])
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("WeCom token request: {}", e)))?;

        let data: WeComTokenResponse = resp.json().await
            .map_err(|e| AstrBotError::Serialization(format!("WeCom token parse: {}", e)))?;

        if let Some(errcode) = data.errcode {
            if errcode != 0 {
                return Err(AstrBotError::Platform {
                    adapter: "wecom".to_string(),
                    message: format!("WeCom token error {}: {}", errcode, data.errmsg.unwrap_or_default()),
                });
            }
        }

        let token = data.access_token
            .ok_or_else(|| AstrBotError::Platform {
                adapter: "wecom".to_string(),
                message: "No access_token in response".to_string(),
            })?;

        let expire_at = chrono::Utc::now().timestamp() + data.expires_in.unwrap_or(7200);

        {
            let mut cache = self.token_cache.write().await;
            *cache = Some((token.clone(), expire_at));
        }

        Ok(token)
    }

    async fn send_message(&self, user_id: &str, content: &str) -> Result<()> {
        let token = self.get_access_token().await?;

        let body = WeComMessageRequest {
            to_user: user_id.to_string(),
            msg_type: "text".to_string(),
            text: WeComTextContent {
                content: content.to_string(),
            },
            agent_id: self.agent_id,
        };

        let resp = self.http_client
            .post(format!("https://qyapi.weixin.qq.com/cgi-bin/message/send?access_token={}", token))
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("WeCom send message: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "wecom".to_string(),
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
// WeCom adapter
// ---------------------------------------------------------------------------

pub struct WeComAdapter {
    metadata: PlatformMetadata,
    shared: Arc<WeComShared>,
    callback_port: u16,
    running: Arc<AtomicBool>,
    server_task: Option<JoinHandle<()>>,
}

impl WeComAdapter {
    pub fn new(corp_id: String, corp_secret: String, agent_id: i64, callback_port: u16) -> Self {
        let metadata = PlatformMetadata {
            id: "wecom".to_string(),
            name: "WeCom".to_string(),
            platform_type: PlatformType::Wecom,
            enabled: true,
            extra: HashMap::new(),
        };

        let shared = Arc::new(WeComShared {
            corp_id,
            corp_secret,
            agent_id,
            token: None,
            encoding_aes_key: None,
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
impl PlatformAdapter for WeComAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        let _ = self.shared.get_access_token().await?;
        info!("WeCom adapter initialized — access_token obtained");
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        self.running.store(true, Ordering::SeqCst);

        let shared = Arc::clone(&self.shared);
        let port = self.callback_port;

        let app = Router::new()
            .route("/callback", post(wecom_callback_handler))
            .with_state(shared);

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = tokio::net::TcpListener::bind(&addr).await
            .map_err(|e| AstrBotError::Network(format!("WeCom bind failed: {}", e)))?;

        let task = tokio::spawn(async move {
            info!("WeCom callback server listening on {}", addr);
            let server = axum::serve(listener, app);
            if let Err(e) = server.await {
                error!("WeCom server error: {}", e);
            }
        });

        self.server_task = Some(task);
        info!("WeCom adapter started on port {}", port);
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        if let Some(task) = self.server_task.take() {
            task.abort();
        }
        info!("WeCom adapter stopped");
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

async fn wecom_callback_handler(
    State(shared): State<Arc<WeComShared>>,
    axum::Json(payload): axum::Json<WeComCallbackPayload>,
) -> (StatusCode, String) {
    let timestamp = payload.timestamp.unwrap_or_default();
    let nonce = payload.nonce.unwrap_or_default();
    let encrypt = payload.encrypt.unwrap_or_default();
    let msg_signature = payload.msg_signature.unwrap_or_default();
    let echostr = payload.echostr.unwrap_or_default();
    
    let token = match &shared.token {
        Some(t) => t.as_str(),
        None => {
            warn!("[WeCom] callback received but no token configured");
            return (StatusCode::INTERNAL_SERVER_ERROR, "No token configured".to_string());
        }
    };
    
    // URL verification: echostr present
    if !echostr.is_empty() {
        let mut params = vec![token, &timestamp, &nonce, &echostr];
        params.sort();
        let computed = format!("{:x}", Sha1::digest(params.join("").as_bytes()));
        if computed != msg_signature {
            return (StatusCode::UNAUTHORIZED, "signature mismatch".to_string());
        }
        if let Some(ref aes_key) = shared.encoding_aes_key {
            match decrypt_wecom_message(aes_key, &echostr, &shared.corp_id) {
                Ok(decrypted) => return (StatusCode::OK, decrypted),
                Err(e) => return (StatusCode::BAD_REQUEST, format!("decrypt failed: {}", e)),
            }
        } else {
            return (StatusCode::OK, echostr);
        }
    }
    
    // Message callback
    if encrypt.is_empty() {
        return (StatusCode::BAD_REQUEST, "empty payload".to_string());
    }
    
    let mut params = vec![token, &timestamp, &nonce, &encrypt];
    params.sort();
    let computed = format!("{:x}", Sha1::digest(params.join("").as_bytes()));
    if computed != msg_signature {
        return (StatusCode::UNAUTHORIZED, "signature mismatch".to_string());
    }
    
    let msg_content = if let Some(ref aes_key) = shared.encoding_aes_key {
        match decrypt_wecom_message(aes_key, &encrypt, &shared.corp_id) {
            Ok(content) => content,
            Err(e) => return (StatusCode::BAD_REQUEST, format!("decrypt failed: {}", e)),
        }
    } else {
        encrypt
    };
    
    info!("[WeCom] received message: {}", msg_content);
    
    let text = extract_wecom_text(&msg_content).unwrap_or_else(|| msg_content.clone());
    
    // Build message and call handler
    if let Some(ref handler) = shared.message_handler {
        let chain = WeComShared::parse_message_content(&text);
        let sender = MessageMember {
            user_id: "wecom_user".to_string(),
            nickname: Some("WeCom".to_string()),
            card: None,
            role: None,
            is_self: false,
        };
        let message = AstrBotMessage {
            message_id: format!("wc_{}", timestamp),
            timestamp: chrono::Utc::now(),
            platform: PlatformType::Wecom,
            session_id: "default".to_string(),
            sender,
            message_type: MessageType::Private,
            chain,
            raw_payload: None,
        };
        let handler_clone = Arc::clone(handler);
        tokio::spawn(async move {
            handler_clone.on_message(message).await;
        });
    }

    (StatusCode::OK, "success".to_string())
}

fn decrypt_wecom_message(encoding_aes_key: &str, encrypt: &str, corp_id: &str) -> std::result::Result<String, String> {
    let key = general_purpose::STANDARD
        .decode(encoding_aes_key)
        .map_err(|e| format!("base64 decode aes key failed: {}", e))?;
    if key.len() != 32 {
        return Err(format!("AES key length must be 32, got {}", key.len()));
    }
    let encrypted_data = general_purpose::STANDARD
        .decode(encrypt)
        .map_err(|e| format!("base64 decode encrypt failed: {}", e))?;
    type Aes256Cbc = cbc::Decryptor<aes::Aes256>;
    let mut decryptor = Aes256Cbc::new_from_slices(&key, &key[..16])
        .map_err(|e| format!("cipher init failed: {:?}", e))?;
    let mut buf = encrypted_data.clone();
    let decrypted = decryptor
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|e| format!("decrypt failed: {:?}", e))?;
    if decrypted.len() < 20 {
        return Err("decrypted data too short".to_string());
    }
    let msg_len = u32::from_be_bytes([
        decrypted[16], decrypted[17], decrypted[18], decrypted[19]
    ]) as usize;
    if 20 + msg_len > decrypted.len() {
        return Err(format!("msg_len {} exceeds data length {}", msg_len, decrypted.len()));
    }
    let msg = String::from_utf8_lossy(&decrypted[20..20 + msg_len]).to_string();
    let trailing_corp_id = String::from_utf8_lossy(&decrypted[20 + msg_len..]).to_string();
    if !trailing_corp_id.is_empty() && trailing_corp_id != corp_id {
        return Err(format!("corp_id mismatch: expected {}, got {}", corp_id, trailing_corp_id));
    }
    Ok(msg)
}

fn extract_wecom_text(xml: &str) -> Option<String> {
    let start = xml.find("<Content>")? + 9;
    let end = xml.find("</Content>")?;
    Some(xml[start..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_wecom_adapter_new() {
        let adapter = WeComAdapter::new(
            "corp_test".to_string(),
            "secret_test".to_string(),
            1000001,
            9999,
        );
        assert_eq!(adapter.metadata.name, "WeCom");
        assert!(adapter.metadata.enabled);
    }

    #[test]
    fn test_parse_message_content() {
        let chain = WeComShared::parse_message_content("hello");
        assert_eq!(chain.plain_text(), "hello");
    }
}
