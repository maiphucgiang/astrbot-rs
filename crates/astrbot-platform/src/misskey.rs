use crate::adapter::PlatformAdapter;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::message::{
    AstrBotMessage, HandlerRef, MessageChain, MessageComponent, MessageHandler, MessageMember,
    MessageType,
};
use astrbot_core::net::SharedHttpClient;
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// Misskey API data models
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct MisskeyNote {
    id: String,
    text: Option<String>,
    user: MisskeyUser,
    #[serde(default)]
    reply: Option<Box<MisskeyNote>>,
    #[serde(default)]
    visibility: String,
    #[serde(rename = "createdAt", default)]
    created_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MisskeyUser {
    id: String,
    #[serde(default)]
    name: Option<String>,
    username: String,
}

#[derive(Debug, Serialize)]
struct NotesMentionsRequest {
    i: String,
    #[serde(default)]
    limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    since_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct NotesCreateRequest {
    i: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "replyId")]
    reply_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct ICheckRequest {
    i: String,
}

// ---------------------------------------------------------------------------
// Message format conversion
// ---------------------------------------------------------------------------

fn parse_misskey_note(note: &MisskeyNote) -> AstrBotMessage {
    let msg_type = match note.visibility.as_str() {
        "specified" | "direct" => MessageType::Private,
        _ => MessageType::Group,
    };

    let sender = MessageMember {
        user_id: note.user.id.clone(),
        nickname: note.user.name.clone(),
        card: Some(note.user.username.clone()),
        role: None,
        is_self: false,
    };

    let mut chain = MessageChain::new();

    // Handle reply
    if let Some(ref reply) = note.reply {
        chain.0.push(MessageComponent::Reply {
            message_id: reply.id.clone(),
            chain: None,
        });
    }

    // Handle text
    if let Some(ref text) = note.text {
        if !text.is_empty() {
            chain.0.push(MessageComponent::Plain { text: text.clone() });
        }
    }

    let timestamp = if let Some(ref dt) = note.created_at {
        chrono::DateTime::parse_from_rfc3339(dt)
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    } else {
        chrono::Utc::now()
    };

    AstrBotMessage {
        message_id: note.id.clone(),
        timestamp,
        platform: PlatformType::Misskey,
        session_id: note.user.id.clone(),
        sender,
        message_type: msg_type,
        chain,
        raw_payload: None,
    }
}

fn chain_to_misskey_text(chain: &MessageChain) -> String {
    let mut text = String::new();
    for comp in &chain.0 {
        match comp {
            MessageComponent::Plain { text: t } => text.push_str(t),
            MessageComponent::At { target, display } => {
                text.push('@');
                text.push_str(display.as_deref().unwrap_or(target.as_str()));
                text.push(' ');
            }
            MessageComponent::Image { url, .. } => {
                if let Some(u) = url {
                    text.push_str(u);
                    text.push('\n');
                }
            }
            MessageComponent::Reply { .. } => {
                // Reply handled separately via reply_id
            }
            _ => {}
        }
    }
    text.trim().to_string()
}

// ---------------------------------------------------------------------------
// Polling loop
// ---------------------------------------------------------------------------

async fn run_polling_loop(
    instance_url: String,
    api_token: String,
    running: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    handler: Arc<std::sync::Mutex<HandlerRef>>,
    client: reqwest::Client,
) {
    let api_base = instance_url.trim_end_matches('/').to_string();
    let mut since_id: Option<String> = None;

    while running.load(Ordering::Relaxed) {
        let req = NotesMentionsRequest {
            i: api_token.clone(),
            limit: 100,
            since_id: since_id.clone(),
        };

        let url = format!("{}/api/notes/mentions", api_base);
        match client
            .post(&url)
            .json(&req)
            .timeout(Duration::from_secs(30))
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<Vec<MisskeyNote>>().await {
                        Ok(notes) => {
                            if !notes.is_empty() {
                                connected.store(true, Ordering::Relaxed);
                            }
                            for note in notes {
                                // Update since_id to avoid re-processing
                                if since_id.as_ref().map_or(true, |id| &note.id > id) {
                                    since_id = Some(note.id.clone());
                                }

                                let astr_msg = parse_misskey_note(&note);
                                let handler_opt = handler.lock().unwrap().clone();
                                if let Some(ref handler) = handler_opt {
                                    handler.on_message(astr_msg).await;
                                }
                            }
                        }
                        Err(e) => {
                            warn!("[Misskey] Failed to parse mentions: {}", e);
                        }
                    }
                } else {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    error!("[Misskey] mentions API failed: {} - {}", status, body);
                }
            }
            Err(e) => {
                warn!("[Misskey] Polling request failed: {}", e);
                connected.store(false, Ordering::Relaxed);
            }
        }

        sleep(Duration::from_secs(5)).await;
    }

    connected.store(false, Ordering::Relaxed);
    info!("[Misskey] Polling loop stopped");
}

// ---------------------------------------------------------------------------
// Misskey Adapter
// ---------------------------------------------------------------------------

pub struct MisskeyAdapter {
    metadata: PlatformMetadata,
    instance_url: String,
    api_token: String,
    connected: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    handler: Arc<std::sync::Mutex<HandlerRef>>,
    poll_handle: Mutex<Option<JoinHandle<()>>>,
    http_client: reqwest::Client,
}

impl MisskeyAdapter {
    pub fn new(instance_url: String, api_token: String) -> Self {
        Self {
            metadata: PlatformMetadata {
                id: "misskey".to_string(),
                name: "Misskey".to_string(),
                platform_type: PlatformType::Misskey,
                enabled: true,
                extra: {
                    let mut map = HashMap::new();
                    map.insert(
                        "instance_url".to_string(),
                        serde_json::Value::String(instance_url.clone()),
                    );
                    map
                },
            },
            instance_url,
            api_token,
            connected: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
            handler: Arc::new(std::sync::Mutex::new(None)),
            poll_handle: Mutex::new(None),
            http_client: SharedHttpClient::new().client(),
        }
    }
}

#[async_trait]
impl PlatformAdapter for MisskeyAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!(
            "[Misskey] Initializing adapter for {}...",
            self.instance_url
        );
        // Verify token by calling /api/i
        let req = ICheckRequest {
            i: self.api_token.clone(),
        };
        let url = format!("{}/api/i", self.instance_url.trim_end_matches('/'));
        let response = self
            .http_client
            .post(&url)
            .json(&req)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| AstrBotError::Platform {
                adapter: "Misskey".to_string(),
                message: format!("auth check failed: {}", e),
            })?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "Misskey".to_string(),
                message: format!("auth check failed: {}", body),
            });
        }

        info!("[Misskey] Token valid, adapter ready");
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        info!("[Misskey] Starting adapter...");
        self.running.store(true, Ordering::Relaxed);

        let instance_url = self.instance_url.clone();
        let api_token = self.api_token.clone();
        let running = Arc::clone(&self.running);
        let connected = Arc::clone(&self.connected);
        let handler = Arc::clone(&self.handler);
        let client = self.http_client.clone();

        let handle = tokio::spawn(async move {
            run_polling_loop(instance_url, api_token, running, connected, handler, client).await;
        });

        let mut guard = self.poll_handle.lock().await;
        *guard = Some(handle);

        info!("[Misskey] Adapter started (polling mentions)");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("[Misskey] Stopping adapter...");
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
                adapter: "Misskey".to_string(),
                message: "adapter not running".to_string(),
            });
        }

        let text = chain_to_misskey_text(chain);
        if text.is_empty() {
            return Ok(());
        }

        let req = NotesCreateRequest {
            i: self.api_token.clone(),
            text,
            visibility: Some("specified".to_string()),
            reply_id: None,
        };

        let url = format!(
            "{}/api/notes/create",
            self.instance_url.trim_end_matches('/')
        );
        let response = self
            .http_client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| AstrBotError::Platform {
                adapter: "Misskey".to_string(),
                message: format!("HTTP request failed: {}", e),
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "Misskey".to_string(),
                message: format!("Misskey API error: {} - {}", status, body),
            });
        }

        info!("[Misskey] Note sent successfully");
        Ok(())
    }

    async fn reply_message(&self, original: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
        let text = chain_to_misskey_text(chain);
        if text.is_empty() {
            return Ok(());
        }

        let req = NotesCreateRequest {
            i: self.api_token.clone(),
            text,
            visibility: Some("specified".to_string()),
            reply_id: Some(original.message_id.clone()),
        };

        let url = format!(
            "{}/api/notes/create",
            self.instance_url.trim_end_matches('/')
        );
        let response = self
            .http_client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| AstrBotError::Platform {
                adapter: "Misskey".to_string(),
                message: format!("HTTP request failed: {}", e),
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "Misskey".to_string(),
                message: format!("Misskey API error: {} - {}", status, body),
            });
        }

        info!("[Misskey] Reply sent successfully");
        Ok(())
    }

    async fn health_check(&self) -> Result<bool> {
        Ok(self.connected.load(Ordering::Relaxed))
    }

    fn set_message_handler(&mut self, handler: Arc<dyn MessageHandler>) {
        let mut guard = self.handler.lock().unwrap();
        *guard = Some(handler);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_misskey_lifecycle() {
        let mut adapter =
            MisskeyAdapter::new("https://misskey.io".to_string(), "test-token".to_string());
        assert_eq!(adapter.metadata().platform_type, PlatformType::Misskey);
        // initialize will fail with test token, skip in unit test
        // adapter.initialize().await.unwrap();
        adapter.start().await.unwrap();
        assert!(!adapter.health_check().await.unwrap()); // not connected until poll succeeds
        adapter.stop().await.unwrap();
    }

    #[test]
    fn test_parse_misskey_note() {
        let note = MisskeyNote {
            id: "note1".to_string(),
            text: Some("hello world".to_string()),
            user: MisskeyUser {
                id: "user1".to_string(),
                name: Some("Alice".to_string()),
                username: "alice".to_string(),
            },
            reply: None,
            visibility: "public".to_string(),
            created_at: Some("2024-01-01T00:00:00Z".to_string()),
        };

        let msg = parse_misskey_note(&note);
        assert_eq!(msg.message_id, "note1");
        assert_eq!(msg.session_id, "user1");
        assert_eq!(msg.sender.nickname, Some("Alice".to_string()));
        assert_eq!(msg.chain.plain_text(), "hello world");
    }

    #[test]
    fn test_chain_to_misskey_text() {
        let mut chain = MessageChain::new();
        chain.0.push(MessageComponent::Plain {
            text: "hi ".to_string(),
        });
        chain.0.push(MessageComponent::At {
            target: "user1".to_string(),
            display: Some("Alice".to_string()),
        });

        let text = chain_to_misskey_text(&chain);
        assert_eq!(text, "hi @Alice");
    }
}
