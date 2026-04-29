use async_trait::async_trait;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::message::{AstrBotMessage, MessageChain, MessageComponent, MessageMember, MessageType, HandlerRef, MessageHandler};
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use astrbot_core::net::SharedHttpClient;
use crate::adapter::PlatformAdapter;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// Telegram Bot API data models
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TelegramUpdatesResponse {
    ok: bool,
    result: Vec<TelegramUpdate>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    #[serde(rename = "update_id")]
    update_id: i64,
    message: Option<TelegramMessage>,
    #[serde(rename = "edited_message")]
    edited_message: Option<TelegramMessage>,
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramMessage {
    #[serde(rename = "message_id")]
    message_id: i64,
    from: Option<TelegramUser>,
    chat: TelegramChat,
    date: i64,
    text: Option<String>,
    photo: Option<Vec<TelegramPhotoSize>>,
    voice: Option<TelegramVoice>,
    document: Option<TelegramDocument>,
    #[serde(rename = "reply_to_message")]
    reply_to_message: Option<Box<TelegramMessage>>,
    #[serde(default)]
    entities: Vec<TelegramMessageEntity>,
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramUser {
    id: i64,
    #[serde(rename = "first_name")]
    first_name: String,
    #[serde(rename = "last_name", default)]
    last_name: Option<String>,
    #[serde(default)]
    username: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramChat {
    id: i64,
    #[serde(rename = "type")]
    chat_type: String,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramPhotoSize {
    #[serde(rename = "file_id")]
    file_id: String,
    #[serde(rename = "file_unique_id")]
    file_unique_id: String,
    width: i32,
    height: i32,
    #[serde(default)]
    #[serde(rename = "file_size")]
    file_size: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramVoice {
    #[serde(rename = "file_id")]
    file_id: String,
    #[serde(default)]
    duration: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramDocument {
    #[serde(rename = "file_id")]
    file_id: String,
    #[serde(default)]
    #[serde(rename = "file_name")]
    file_name: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramMessageEntity {
    #[serde(rename = "type")]
    entity_type: String,
    offset: i32,
    length: i32,
}

// ---------------------------------------------------------------------------
// Send message request models
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct SendMessageRequest {
    #[serde(rename = "chat_id")]
    chat_id: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "reply_to_message_id")]
    reply_to_message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "parse_mode")]
    parse_mode: Option<String>,
}

#[derive(Debug, Serialize)]
struct SendPhotoRequest {
    #[serde(rename = "chat_id")]
    chat_id: String,
    photo: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "reply_to_message_id")]
    reply_to_message_id: Option<i64>,
}

#[derive(Debug, Serialize)]
struct SendDocumentRequest {
    #[serde(rename = "chat_id")]
    chat_id: String,
    document: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "reply_to_message_id")]
    reply_to_message_id: Option<i64>,
}

#[derive(Debug, Serialize)]
struct SendVoiceRequest {
    #[serde(rename = "chat_id")]
    chat_id: String,
    voice: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "reply_to_message_id")]
    reply_to_message_id: Option<i64>,
}

// ---------------------------------------------------------------------------
// Message format conversion
// ---------------------------------------------------------------------------

fn parse_telegram_message(msg: &TelegramMessage) -> AstrBotMessage {
    let msg_type = match msg.chat.chat_type.as_str() {
        "private" => MessageType::Private,
        "group" | "supergroup" => MessageType::Group,
        "channel" => MessageType::Channel,
        _ => MessageType::Unknown,
    };

    let sender = if let Some(ref from) = msg.from {
        let name = from
            .username
            .clone()
            .unwrap_or_else(|| from.first_name.clone());
        MessageMember {
            user_id: from.id.to_string(),
            nickname: Some(name),
            card: from.last_name.clone(),
            role: None,
            is_self: false,
        }
    } else {
        MessageMember {
            user_id: "0".to_string(),
            nickname: None,
            card: None,
            role: None,
            is_self: false,
        }
    };

    // Build message chain from text + entities
    let mut chain = MessageChain::new();

    // Handle reply
    if let Some(ref reply_to) = msg.reply_to_message {
        chain.0.push(MessageComponent::Reply {
            message_id: reply_to.message_id.to_string(),
            chain: None,
        });
    }

    // Handle photo
    if let Some(ref photos) = msg.photo {
        if let Some(photo) = photos.last() {
            chain.0.push(MessageComponent::Image {
                url: Some(format!(
                    "https://api.telegram.org/file/bot<token>/{}?download=1",
                    photo.file_id
                )),
                file_id: Some(photo.file_id.clone()),
                base64: None,
            });
        }
    }

    // Handle voice
    if let Some(ref voice) = msg.voice {
        chain.0.push(MessageComponent::Voice {
            url: None,
            file_id: Some(voice.file_id.clone()),
            base64: None,
        });
    }

    // Handle document
    if let Some(ref doc) = msg.document {
        chain.0.push(MessageComponent::File {
            name: doc.file_name.clone().unwrap_or_else(|| "file".to_string()),
            url: None,
            file_id: Some(doc.file_id.clone()),
        });
    }

    // Handle text (with entities for @mentions)
    if let Some(ref text) = msg.text {
        let mut entities = msg.entities.clone();
        // Sort by offset so we can iterate in order
        entities.sort_by_key(|e| e.offset);
        
        let mut last_offset: usize = 0;
        let text_len = text.len();
        
        for entity in entities {
            let start = entity.offset as usize;
            let end = (entity.offset + entity.length) as usize;
            
            // Skip out-of-bounds entities
            if start > text_len || end > text_len {
                continue;
            }
            
            // Add plain text before this entity
            if start > last_offset {
                let plain = &text[last_offset..start];
                if !plain.is_empty() {
                    chain.0.push(MessageComponent::Plain { text: plain.to_string() });
                }
            }
            
            match entity.entity_type.as_str() {
                "mention" => {
                    let mention_text = &text[start..end];
                    // Remove the leading '@' for target
                    let target = mention_text.strip_prefix('@').unwrap_or(mention_text).to_string();
                    chain.0.push(MessageComponent::At { target, display: Some(mention_text.to_string()) });
                }
                "text_mention" => {
                    let mention_text = &text[start..end];
                    chain.0.push(MessageComponent::At { target: mention_text.to_string(), display: Some(mention_text.to_string()) });
                }
                _ => {
                    // Other entity types: add as plain text
                    let entity_text = &text[start..end];
                    if !entity_text.is_empty() {
                        chain.0.push(MessageComponent::Plain { text: entity_text.to_string() });
                    }
                }
            }
            
            last_offset = end;
        }
        
        // Add remaining plain text after last entity
        if last_offset < text_len {
            let plain = &text[last_offset..];
            if !plain.is_empty() {
                chain.0.push(MessageComponent::Plain { text: plain.to_string() });
            }
        }
        
        // If no entities at all, add the whole text as plain
        if msg.entities.is_empty() && !text.is_empty() {
            chain.0.push(MessageComponent::Plain { text: text.clone() });
        }
    }

    AstrBotMessage {
        message_id: msg.message_id.to_string(),
        timestamp: chrono::DateTime::from_timestamp(msg.date, 0)
            .unwrap_or_else(chrono::Utc::now),
        platform: PlatformType::Telegram,
        session_id: msg.chat.id.to_string(),
        sender,
        message_type: msg_type,
        chain,
        raw_payload: None,
    }
}

fn chain_to_telegram_text(chain: &MessageChain) -> (String, Option<String>) {
    let mut text = String::new();
    let mut photo_url: Option<String> = None;

    for comp in &chain.0 {
        match comp {
            MessageComponent::Plain { text: t } => text.push_str(t),
            MessageComponent::At { target, display } => {
                text.push('@');
                text.push_str(display.as_deref().unwrap_or(target.as_str()));
            }
            MessageComponent::Image { url, file_id, .. } => {
                // For Telegram, if there's a URL, use it. Otherwise, we need to use file_id with API
                if let Some(u) = url {
                    photo_url = Some(u.clone());
                } else if let Some(fid) = file_id {
                    photo_url = Some(fid.clone());
                }
            }
            _ => {}
        }
    }

    (text, photo_url)
}

// ---------------------------------------------------------------------------
// Polling loop
// ---------------------------------------------------------------------------

async fn run_polling_loop(
    api_base: String,
    bot_token: String,
    running: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    handler: Arc<std::sync::Mutex<HandlerRef>>,
    client: reqwest::Client,
) {
    let mut offset: Option<i64> = None;
    let base_url = format!("{}/bot{}", api_base, bot_token);

    while running.load(Ordering::Relaxed) {
        let url = if let Some(off) = offset {
            format!("{}/getUpdates?offset={}&limit=100", base_url, off)
        } else {
            format!("{}/getUpdates?limit=100", base_url)
        };

        match client.get(&url).timeout(Duration::from_secs(30)).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<TelegramUpdatesResponse>().await {
                        Ok(updates) => {
                            if !updates.result.is_empty() {
                                connected.store(true, Ordering::Relaxed);
                            }
                            for update in updates.result {
                                // Update offset to skip processed updates
                                offset = Some(update.update_id + 1);

                                let msg = update
                                    .message
                                    .as_ref()
                                    .or(update.edited_message.as_ref());

                                if let Some(ref telegram_msg) = msg {
                                    let astr_msg = parse_telegram_message(telegram_msg);
                                    let handler_opt = handler.lock().unwrap().clone();
                                    if let Some(ref handler) = handler_opt {
                                        handler.on_message(astr_msg).await;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!("[Telegram] Failed to parse updates: {}", e);
                        }
                    }
                } else {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    error!("[Telegram] getUpdates failed: {} - {}", status, body);
                }
            }
            Err(e) => {
                warn!("[Telegram] Polling request failed: {}", e);
                connected.store(false, Ordering::Relaxed);
            }
        }

        // Wait before next poll (short interval for tests)
        sleep(Duration::from_millis(100)).await;
    }

    connected.store(false, Ordering::Relaxed);
    info!("[Telegram] Polling loop stopped");
}

// ---------------------------------------------------------------------------
// Telegram Adapter
// ---------------------------------------------------------------------------

pub struct TelegramAdapter {
    metadata: PlatformMetadata,
    bot_token: String,
    api_base: String,
    webhook_url: Option<String>,
    connected: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    handler: Arc<std::sync::Mutex<HandlerRef>>,
    poll_handle: Mutex<Option<JoinHandle<()>>>,
    http_client: reqwest::Client,
}

impl TelegramAdapter {
    pub fn new(
        bot_token: String,
        api_base: Option<String>,
        #[allow(dead_code)]
        webhook_url: Option<String>,
    ) -> Self {
        Self {
            metadata: PlatformMetadata {
                id: "telegram".to_string(),
                name: "Telegram".to_string(),
                platform_type: PlatformType::Telegram,
                enabled: true,
                extra: HashMap::new(),
            },
            bot_token,
            api_base: api_base.unwrap_or_else(|| "https://api.telegram.org".to_string()),
            webhook_url,
            connected: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
            handler: Arc::new(std::sync::Mutex::new(None)),
            poll_handle: Mutex::new(None),
            http_client: SharedHttpClient::new().client(),
        }
    }
}

#[async_trait]
impl PlatformAdapter for TelegramAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!("[Telegram] Initializing adapter...");
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        info!("[Telegram] Starting adapter...");
        self.running.store(true, Ordering::Relaxed);

        let api_base = self.api_base.clone();
        let bot_token = self.bot_token.clone();
        let running = Arc::clone(&self.running);
        let connected = Arc::clone(&self.connected);
        let handler = Arc::clone(&self.handler);
        let client = self.http_client.clone();

        let handle = tokio::spawn(async move {
            run_polling_loop(api_base, bot_token, running, connected, handler, client).await;
        });

        let mut guard = self.poll_handle.lock().await;
        *guard = Some(handle);

        info!("[Telegram] Adapter started (polling)");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("[Telegram] Stopping adapter...");
        self.running.store(false, Ordering::Relaxed);
        self.connected.store(false, Ordering::Relaxed);

        let mut guard = self.poll_handle.lock().await;
        if let Some(handle) = guard.take() {
            let _ = handle.await;
        }
        Ok(())
    }

    async fn send_message(&self, target: &MessageSource, chain: &MessageChain) -> Result<()> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(AstrBotError::Platform {
                adapter: "Telegram".to_string(),
                message: "adapter not running".to_string(),
            });
        }

        let chat_id = &target.session_id;
        let (text, photo_url) = chain_to_telegram_text(chain);

        let base_url = format!("{}/bot{}", self.api_base, self.bot_token);

        // If there's a photo, use sendPhoto API
        if let Some(photo) = photo_url {
            let req = SendPhotoRequest {
                chat_id: chat_id.clone(),
                photo,
                caption: if text.is_empty() { None } else { Some(text) },
                reply_to_message_id: None,
            };

            let url = format!("{}/sendPhoto", base_url);
            let response = self
                .http_client
                .post(&url)
                .json(&req)
                .send()
                .await
                .map_err(|e| AstrBotError::Platform {
                    adapter: "Telegram".to_string(),
                    message: format!("HTTP request failed: {}", e),
                })?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(AstrBotError::Platform {
                    adapter: "Telegram".to_string(),
                    message: format!("Telegram API error: {} - {}", status, body),
                });
            }

            info!("[Telegram] Photo sent successfully");
            return Ok(());
        }

        // Otherwise, send text message
        if !text.is_empty() {
            let req = SendMessageRequest {
                chat_id: chat_id.clone(),
                text,
                reply_to_message_id: None,
                parse_mode: None,
            };

            let url = format!("{}/sendMessage", base_url);
            let response = self
                .http_client
                .post(&url)
                .json(&req)
                .send()
                .await
                .map_err(|e| AstrBotError::Platform {
                    adapter: "Telegram".to_string(),
                    message: format!("HTTP request failed: {}", e),
                })?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(AstrBotError::Platform {
                    adapter: "Telegram".to_string(),
                    message: format!("Telegram API error: {} - {}", status, body),
                });
            }
        }

        info!("[Telegram] Message sent successfully");
        Ok(())
    }

    async fn reply_message(&self, original: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
        let chat_id = &original.session_id;
        let (text, photo_url) = chain_to_telegram_text(chain);

        let base_url = format!("{}/bot{}", self.api_base, self.bot_token);
        let reply_id = original.message_id.parse::<i64>().ok();

        if let Some(photo) = photo_url {
            let req = SendPhotoRequest {
                chat_id: chat_id.clone(),
                photo,
                caption: if text.is_empty() { None } else { Some(text) },
                reply_to_message_id: reply_id,
            };

            let url = format!("{}/sendPhoto", base_url);
            let response = self
                .http_client
                .post(&url)
                .json(&req)
                .send()
                .await
                .map_err(|e| AstrBotError::Platform {
                    adapter: "Telegram".to_string(),
                    message: format!("HTTP request failed: {}", e),
                })?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(AstrBotError::Platform {
                    adapter: "Telegram".to_string(),
                    message: format!("Telegram API error: {} - {}", status, body),
                });
            }
            return Ok(());
        }

        if !text.is_empty() {
            let req = SendMessageRequest {
                chat_id: chat_id.clone(),
                text,
                reply_to_message_id: reply_id,
                parse_mode: None,
            };

            let url = format!("{}/sendMessage", base_url);
            let response = self
                .http_client
                .post(&url)
                .json(&req)
                .send()
                .await
                .map_err(|e| AstrBotError::Platform {
                    adapter: "Telegram".to_string(),
                    message: format!("HTTP request failed: {}", e),
                })?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(AstrBotError::Platform {
                    adapter: "Telegram".to_string(),
                    message: format!("Telegram API error: {} - {}", status, body),
                });
            }
        }

        Ok(())
    }

    async fn health_check(&self) -> Result<bool> {
        Ok(self.running.load(Ordering::Relaxed))
    }

    fn set_message_handler(&mut self, handler: Arc<dyn MessageHandler>) {
        let mut h = self.handler.lock().unwrap();
        *h = Some(handler);
    }
}
