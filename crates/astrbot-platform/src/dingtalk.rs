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
use hmac::{Hmac, Mac};
use sha2::Sha256;
use base64::{Engine as _, engine::general_purpose};

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
    encoding_aes_key: Option<String>,
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
    State(shared): State<Arc<DingTalkShared>>,
    axum::Json(payload): axum::Json<DingTalkCallbackPayload>,
) -> StatusCode {
    // Extract parameters
    let timestamp = payload.timestamp.unwrap_or_default();
    let nonce = payload.nonce.unwrap_or_default();
    let encrypt = payload.encrypt.unwrap_or_default();
    let msg_signature = payload.msg_signature.unwrap_or_default();
    
    if encrypt.is_empty() {
        warn!("[DingTalk] callback received with empty encrypt");
        return StatusCode::BAD_REQUEST;
    }
    
    // 1. Verify signature
    let sign_content = format!("{}\n{}\n{}", timestamp, nonce, encrypt);
    let mut mac = match Hmac::<Sha256>::new_from_slice(shared.app_secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => {
            error!("[DingTalk] failed to init HMAC");
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    };
    mac.update(sign_content.as_bytes());
    let computed_sig = general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    
    if computed_sig != msg_signature {
        warn!(
            "[DingTalk] signature mismatch: computed={}, received={}",
            computed_sig, msg_signature
        );
        return StatusCode::UNAUTHORIZED;
    }
    
    // 2. Decrypt message (AES-256-CBC)
    let decrypted = match decrypt_dingtalk_message(shared.encoding_aes_key.as_deref(), &encrypt) {
        Ok(data) => data,
        Err(e) => {
            warn!("[DingTalk] decrypt failed: {}", e);
            return StatusCode::BAD_REQUEST;
        }
    };
    
    // 3. Parse decrypted JSON and invoke handler
    let msg_text = match String::from_utf8(decrypted) {
        Ok(s) => s,
        Err(_) => {
            warn!("[DingTalk] decrypted data is not valid UTF-8");
            return StatusCode::BAD_REQUEST;
        }
    };
    
    info!("[DingTalk] received message: {}", msg_text);
    
    // Parse the JSON to extract actual content
    let content = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&msg_text) {
        json.get("text")
            .and_then(|t| t.get("content"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                json.get("content").and_then(|c| c.as_str()).map(|s| s.to_string())
            })
            .unwrap_or_else(|| msg_text.clone())
    } else {
        msg_text.clone()
    };
    
    // 4. Build AstrBotMessage and call handler
    if let Some(ref handler) = shared.message_handler {
        let chain = DingTalkShared::parse_message_content(&content);
        let sender = MessageMember {
            user_id: "dingtalk_user".to_string(),
            nickname: "DingTalk".to_string(),
            avatar: None,
        };
        let message = AstrBotMessage {
            message_id: format!("dt_{}", timestamp),
            sender,
            timestamp: chrono::Utc::now().timestamp() as u64,
            message_type: MessageType::Text,
            raw_message: chain.clone(),
            processed_message: None,
            group_id: None,
            is_at_me: true,
        };
        
        let handler_clone = Arc::clone(handler);
        tokio::spawn(async move {
            if let Err(e) = handler_clone.handle_message(message).await {
                warn!("[DingTalk] handler error: {}", e);
            }
        });
    }
    
    StatusCode::OK
}

/// Decrypt DingTalk AES-encrypted message
/// Key: base64_decode(EncodingAESKey) — 43 chars base64 → 32 bytes AES key
/// IV: first 16 bytes of the key
fn decrypt_dingtalk_message(encoding_aes_key: Option<&str>, encrypt: &str) -> Result<Vec<u8>, String> {
    use cbc::cipher::{BlockDecryptMut, KeyIvInit};
    use cbc::cipher::block_padding::Pkcs7;
    
    let aes_key = match encoding_aes_key {
        Some(key) => key,
        None => {
            // No AES key configured — dev mode, try base64 decode as plaintext
            return general_purpose::STANDARD
                .decode(encrypt)
                .map_err(|e| format!("base64 decode failed: {}", e));
        }
    };
    
    // Decode base64 encrypted data
    let encrypted_data = general_purpose::STANDARD
        .decode(encrypt)
        .map_err(|e| format!("base64 decode failed: {}", e))?;
    
    // Derive key: base64_decode(EncodingAESKey) → 32 bytes
    let key = general_purpose::STANDARD
        .decode(aes_key)
        .map_err(|e| format!("base64 decode aes key failed: {}", e))?;
    
    if key.len() != 32 {
        return Err(format!("AES key length must be 32, got {}", key.len()));
    }
    
    let iv: [u8; 16] = key[..16]
        .try_into()
        .map_err(|_| "iv derivation failed".to_string())?;
    let key: [u8; 32] = key[..32]
        .try_into()
        .map_err(|_| "key derivation failed".to_string())?;
    
    // AES-256-CBC decrypt
    type Aes256Cbc = cbc::Decryptor<aes::Aes256>;
    let mut decryptor = Aes256Cbc::new_from_slices(&key, &iv)
        .map_err(|e| format!("cipher init failed: {:?}", e))?;
    
    let mut buf = encrypted_data.clone();
    let decrypted = decryptor
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|e| format!("decrypt failed: {:?}", e))?;
    
    Ok(decrypted.to_vec())
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
