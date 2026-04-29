use crate::adapter::PlatformAdapter;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::message::{
    AstrBotMessage, HandlerRef, MessageChain, MessageComponent, MessageHandler, MessageMember,
    MessageType,
};
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use async_trait::async_trait;
use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

// Type alias for HMAC-SHA256
type HmacSha256 = Hmac<Sha256>;

/// Constant-time hex string comparison to prevent timing attacks
fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

// ---------------------------------------------------------------------------
// Webhook shared state
// ---------------------------------------------------------------------------

/// Shared state for the Webhook adapter, including an optional secret token
#[derive(Debug, Clone)]
pub struct WebhookShared {
    /// Optional secret token for signature verification
    pub secret_token: Option<String>,
    /// Endpoint path (e.g., "/webhook/adapter1")
    pub endpoint: String,
    /// Sender for injecting payloads
    pub tx: tokio::sync::mpsc::Sender<WebhookPayload>,
}

/// A raw webhook payload delivered to the adapter
#[derive(Debug, Clone)]
pub struct WebhookPayload {
    pub headers: HashMap<String, String>,
    pub body: String,
    pub source_ip: Option<String>,
}

// ---------------------------------------------------------------------------
// Payload processing loop
// ---------------------------------------------------------------------------

async fn run_webhook_loop(
    mut rx: tokio::sync::mpsc::Receiver<WebhookPayload>,
    running: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    handler: Arc<std::sync::Mutex<HandlerRef>>,
    secret_token: Option<String>,
) {
    connected.store(true, Ordering::Relaxed);
    info!("[Webhook] Payload processing loop started");

    while running.load(Ordering::Relaxed) {
        match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Some(payload)) => {
                // Optional signature verification
                if let Some(ref secret) = secret_token {
                    let signature = payload
                        .headers
                        .get("x-signature")
                        .or_else(|| payload.headers.get("X-Signature"))
                        .or_else(|| payload.headers.get("x-hub-signature-256"))
                        .or_else(|| payload.headers.get("X-Hub-Signature-256"));

                    match signature {
                        Some(sig_header) => {
                            // Parse expected signature: "sha256=<hex>" or raw hex
                            let expected_hex =
                                sig_header.strip_prefix("sha256=").unwrap_or(sig_header);

                            // Compute HMAC-SHA256 of the body
                            let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
                                Ok(m) => m,
                                Err(e) => {
                                    warn!("[Webhook] Failed to create HMAC: {}", e);
                                    continue;
                                }
                            };
                            mac.update(payload.body.as_bytes());
                            let result = mac.finalize();
                            let computed_hex = hex::encode(result.into_bytes());

                            // Constant-time comparison to prevent timing attacks
                            if !constant_time_eq(expected_hex, &computed_hex) {
                                warn!("[Webhook] Signature mismatch, skipping");
                                continue;
                            }
                            info!("[Webhook] Signature verified");
                        }
                        None => {
                            warn!("[Webhook] Missing signature, skipping");
                            continue;
                        }
                    }
                }

                // Parse the webhook body as JSON
                let parsed: serde_json::Value = match serde_json::from_str(&payload.body) {
                    Ok(v) => v,
                    Err(_) => {
                        // Treat raw body as plain text message
                        serde_json::json!({"text": payload.body})
                    }
                };

                let text = parsed
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&payload.body)
                    .to_string();

                let user_id = parsed
                    .get("user_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("webhook_user")
                    .to_string();

                let session_id = parsed
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("webhook_default")
                    .to_string();

                let message_id = parsed
                    .get("message_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("webhook_msg")
                    .to_string();

                let sender = MessageMember {
                    user_id: user_id.clone(),
                    nickname: Some(user_id.clone()),
                    card: None,
                    role: None,
                    is_self: false,
                };

                let mut chain = MessageChain::new();
                if !text.is_empty() {
                    chain.0.push(MessageComponent::Plain { text });
                }

                let astr_msg = AstrBotMessage {
                    message_id,
                    timestamp: Utc::now(),
                    platform: PlatformType::Webhook,
                    session_id,
                    sender,
                    message_type: MessageType::Unknown,
                    chain,
                    raw_payload: Some(serde_json::Value::String(payload.body)),
                };

                let handler_opt = handler.lock().unwrap().clone();
                if let Some(ref h) = handler_opt {
                    h.on_message(astr_msg).await;
                }
            }
            Ok(None) => {
                info!("[Webhook] Payload channel closed");
                break;
            }
            Err(_) => {
                // timeout, continue loop
            }
        }
    }

    connected.store(false, Ordering::Relaxed);
    info!("[Webhook] Payload loop stopped");
}

// ---------------------------------------------------------------------------
// Webhook Adapter
// ---------------------------------------------------------------------------

pub struct WebhookAdapter {
    metadata: PlatformMetadata,
    shared: WebhookShared,
    rx: Mutex<Option<tokio::sync::mpsc::Receiver<WebhookPayload>>>,
    connected: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    handler: Arc<std::sync::Mutex<HandlerRef>>,
    loop_handle: Mutex<Option<JoinHandle<()>>>,
    /// In-memory outgoing queue for testing
    outgoing: Mutex<Vec<(String, String)>>,
}

impl WebhookAdapter {
    pub fn new(
        endpoint: impl Into<String>,
        secret_token: Option<String>,
        buffer: usize,
    ) -> (Self, WebhookShared) {
        let (tx, rx) = tokio::sync::mpsc::channel(buffer);
        let shared = WebhookShared {
            secret_token: secret_token.clone(),
            endpoint: endpoint.into(),
            tx: tx.clone(),
        };
        let shared_clone = shared.clone();
        let adapter = Self {
            metadata: PlatformMetadata {
                id: "webhook".to_string(),
                name: "Webhook".to_string(),
                platform_type: PlatformType::Webhook,
                enabled: true,
                extra: HashMap::new(),
            },
            shared,
            rx: Mutex::new(Some(rx)),
            connected: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
            handler: Arc::new(std::sync::Mutex::new(None)),
            loop_handle: Mutex::new(None),
            outgoing: Mutex::new(Vec::new()),
        };
        (adapter, shared_clone)
    }

    /// Inject a webhook payload from outside (e.g., HTTP endpoint) into the adapter
    pub async fn inject_payload(&self, payload: WebhookPayload) -> Result<()> {
        // Re-create sender on demand since we don't hold it
        Err(AstrBotError::Platform {
            adapter: "Webhook".to_string(),
            message: "use the WebhookShared sender directly".to_string(),
        })
    }

    /// Pop the next outgoing message (for testing)
    pub async fn pop_outgoing(&self) -> Option<(String, String)> {
        let mut queue = self.outgoing.lock().await;
        queue.pop()
    }
}

#[async_trait]
impl PlatformAdapter for WebhookAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!(
            "[Webhook] Initializing adapter (endpoint={})...",
            self.shared.endpoint
        );
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        info!("[Webhook] Starting adapter...");
        self.running.store(true, Ordering::Relaxed);

        let rx = {
            let mut guard = self.rx.lock().await;
            guard.take().ok_or_else(|| AstrBotError::Platform {
                adapter: "Webhook".to_string(),
                message: "receiver already taken".to_string(),
            })?
        };

        let running = Arc::clone(&self.running);
        let connected = Arc::clone(&self.connected);
        let handler = Arc::clone(&self.handler);
        let secret = self.shared.secret_token.clone();

        let handle = tokio::spawn(async move {
            run_webhook_loop(rx, running, connected, handler, secret).await;
        });

        let mut guard = self.loop_handle.lock().await;
        *guard = Some(handle);

        info!("[Webhook] Adapter started");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("[Webhook] Stopping adapter...");
        self.running.store(false, Ordering::Relaxed);
        self.connected.store(false, Ordering::Relaxed);

        let mut guard = self.loop_handle.lock().await;
        if let Some(handle) = guard.take() {
            let _ = handle.await;
        }
        Ok(())
    }

    async fn send_message(&self, target: &MessageSource, chain: &MessageChain) -> Result<()> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(AstrBotError::Platform {
                adapter: "Webhook".to_string(),
                message: "adapter not running".to_string(),
            });
        }

        let text = chain.plain_text();
        let mut queue = self.outgoing.lock().await;
        queue.push((target.session_id.clone(), text));

        info!("[Webhook] Message queued for session {}", target.session_id);
        Ok(())
    }

    async fn reply_message(&self, original: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
        let text = chain.plain_text();
        let mut queue = self.outgoing.lock().await;
        queue.push((original.session_id.clone(), text));
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
