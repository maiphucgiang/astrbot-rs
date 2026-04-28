use async_trait::async_trait;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::message::{AstrBotMessage, MessageChain, MessageComponent, MessageMember, MessageType, HandlerRef, MessageHandler};
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use crate::adapter::PlatformAdapter;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};
use chrono::Utc;

// ---------------------------------------------------------------------------
// Webchat shared state
// ---------------------------------------------------------------------------

/// Shared state for the Webchat adapter, including a channel for incoming messages
#[derive(Debug, Clone)]
pub struct WebchatShared {
    /// Sender for injecting incoming messages into the adapter
    pub tx: tokio::sync::mpsc::Sender<WebchatIncomingMessage>,
    /// Bot user ID
    pub bot_id: String,
}

/// An incoming message delivered to the Webchat adapter
#[derive(Debug, Clone)]
pub struct WebchatIncomingMessage {
    pub session_id: String,
    pub user_id: String,
    pub username: String,
    pub text: String,
    pub message_id: String,
}

// ---------------------------------------------------------------------------
// Message queue / processing loop
// ---------------------------------------------------------------------------

async fn run_message_loop(
    mut rx: tokio::sync::mpsc::Receiver<WebchatIncomingMessage>,
    running: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    handler: Arc<std::sync::Mutex<HandlerRef>>,
) {
    connected.store(true, Ordering::Relaxed);
    info!("[Webchat] Message loop started");

    while running.load(Ordering::Relaxed) {
        match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Some(msg)) => {
                let sender = MessageMember {
                    user_id: msg.user_id.clone(),
                    nickname: Some(msg.username.clone()),
                    card: None,
                    role: None,
                    is_self: false,
                };

                let mut chain = MessageChain::new();
                if !msg.text.is_empty() {
                    chain.0.push(MessageComponent::Plain { text: msg.text });
                }

                let astr_msg = AstrBotMessage {
                    message_id: msg.message_id,
                    timestamp: Utc::now(),
                    platform: PlatformType::Webchat,
                    session_id: msg.session_id,
                    sender,
                    message_type: MessageType::Private,
                    chain,
                    raw_payload: None,
                };

                let handler_opt = handler.lock().unwrap().clone();
                if let Some(ref h) = handler_opt {
                    h.on_message(astr_msg).await;
                }
            }
            Ok(None) => {
                info!("[Webchat] Message channel closed");
                break;
            }
            Err(_) => {
                // timeout, continue loop
            }
        }
    }

    connected.store(false, Ordering::Relaxed);
    info!("[Webchat] Message loop stopped");
}

// ---------------------------------------------------------------------------
// Webchat Adapter
// ---------------------------------------------------------------------------

pub struct WebchatAdapter {
    metadata: PlatformMetadata,
    shared: WebchatShared,
    rx: Mutex<Option<tokio::sync::mpsc::Receiver<WebchatIncomingMessage>>>,
    connected: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    handler: Arc<std::sync::Mutex<HandlerRef>>,
    loop_handle: Mutex<Option<JoinHandle<()>>>,
    /// In-memory outgoing queue for testing / echo
    outgoing: Mutex<Vec<(String, String)>>,
}

impl WebchatAdapter {
    pub fn new(buffer: usize) -> (Self, WebchatShared) {
        let (tx, rx) = tokio::sync::mpsc::channel(buffer);
        let bot_id = "webchat_bot".to_string();
        let shared = WebchatShared { tx: tx.clone(), bot_id: bot_id.clone() };
        let shared_clone = shared.clone();
        let adapter = Self {
            metadata: PlatformMetadata {
                id: "webchat".to_string(),
                name: "WebChat".to_string(),
                platform_type: PlatformType::Webchat,
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

    /// Inject a message from outside (e.g., HTTP endpoint) into the adapter
    pub async fn inject_message(&self, msg: WebchatIncomingMessage) -> Result<()> {
        self.shared.tx.send(msg).await.map_err(|_| AstrBotError::Platform {
            adapter: "WebChat".to_string(),
            message: "channel closed".to_string(),
        })
    }

    /// Pop the next outgoing message (for testing)
    pub async fn pop_outgoing(&self) -> Option<(String, String)> {
        let mut queue = self.outgoing.lock().await;
        queue.pop()
    }
}

#[async_trait]
impl PlatformAdapter for WebchatAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!("[Webchat] Initializing adapter...");
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        info!("[Webchat] Starting adapter...");
        self.running.store(true, Ordering::Relaxed);

        let rx = {
            let mut guard = self.rx.lock().await;
            guard.take().ok_or_else(|| AstrBotError::Platform {
                adapter: "WebChat".to_string(),
                message: "receiver already taken".to_string(),
            })?
        };

        let running = Arc::clone(&self.running);
        let connected = Arc::clone(&self.connected);
        let handler = Arc::clone(&self.handler);

        let handle = tokio::spawn(async move {
            run_message_loop(rx, running, connected, handler).await;
        });

        let mut guard = self.loop_handle.lock().await;
        *guard = Some(handle);

        info!("[Webchat] Adapter started");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("[Webchat] Stopping adapter...");
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
                adapter: "WebChat".to_string(),
                message: "adapter not running".to_string(),
            });
        }

        let text = chain.plain_text();
        let mut queue = self.outgoing.lock().await;
        queue.push((target.session_id.clone(), text));

        info!("[Webchat] Message queued for session {}", target.session_id);
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
