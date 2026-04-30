//! Unified mock testing utilities for AstrBot Rust.
//!
//! This module provides reusable mock implementations of core traits
//! for use across all workspace crates. These are **not** `#[cfg(test)]`-gated
//! so that dependent crates (e.g. `astrbot-platform`) can import them in
//! their own test suites.
//!
//! # Mock Types
//! - [`MockProvider`] — deterministic LLM provider with configurable responses
//! - [`MockPlatformAdapter`] — in-memory platform adapter using tokio mpsc channels
//! - [`MockMessageHandler`] — message handler that captures all received messages

use crate::errors::Result;
use crate::message::{
    AstrBotMessage, HandlerRef, MessageChain, MessageComponent, MessageHandler, MessageMember,
    MessageType,
};
use crate::platform::{MessageSource, PlatformMetadata, PlatformType};
use crate::provider::{
    ChatConfig, ChatMessage, ChatResponse, ChatStreamChunk, ModelInfo, Provider,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

// ───────────────────────────────────────────────
// MockProvider
// ───────────────────────────────────────────────

/// A fully configurable mock LLM provider for tests.
///
/// # Example
/// ```
/// # use astrbot_core::testing::MockProvider;
/// let provider = MockProvider::new("mock", "Mock")
///     .with_chat_response("Hello, test!");
/// ```
pub struct MockProvider {
    id: String,
    name: String,
    chat_response: String,
    models_list: Vec<String>,
    embedding_dim: usize,
    embeddings: Vec<Vec<f32>>,
    should_fail_chat: bool,
    should_fail_health: bool,
    call_count: AtomicUsize,
}

impl MockProvider {
    /// Create a new mock provider with the given id and name.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            chat_response: "Mock response".to_string(),
            models_list: vec!["mock-model".to_string()],
            embedding_dim: 3,
            embeddings: Vec::new(),
            should_fail_chat: false,
            should_fail_health: false,
            call_count: AtomicUsize::new(0),
        }
    }

    /// Set the deterministic text response for `chat()`.
    pub fn with_chat_response(mut self, text: impl Into<String>) -> Self {
        self.chat_response = text.into();
        self
    }

    /// Set the list of models returned by `models()`.
    pub fn with_models(mut self, models: Vec<String>) -> Self {
        self.models_list = models;
        self
    }

    /// Set the embedding dimension (default 3).
    pub fn with_embedding_dim(mut self, dim: usize) -> Self {
        self.embedding_dim = dim;
        self
    }

    /// Set pre-canned embeddings to return from `embedding()`.
    pub fn with_embeddings(mut self, embeddings: Vec<Vec<f32>>) -> Self {
        self.embeddings = embeddings;
        self
    }

    /// Make `chat()` always fail.
    pub fn with_chat_failure(mut self) -> Self {
        self.should_fail_chat = true;
        self
    }

    /// Make `health_check()` always return false.
    pub fn with_health_failure(mut self) -> Self {
        self.should_fail_health = true;
        self
    }

    /// Return how many times `chat()` was called.
    pub fn chat_count(&self) -> usize {
        self.call_count.load(Ordering::Relaxed)
    }

    /// Generate a deterministic embedding vector for a given text.
    fn generate_embedding(&self, text: &str) -> Vec<f32> {
        // Deterministic: hash the text into floats
        let mut vec = vec![0.0f32; self.embedding_dim];
        for (i, byte) in text.bytes().enumerate() {
            vec[i % self.embedding_dim] += byte as f32 / 255.0;
        }
        // Normalize
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            vec.iter_mut().for_each(|x| *x /= norm);
        }
        vec
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    async fn models(&self) -> Result<Vec<String>> {
        Ok(self.models_list.clone())
    }

    async fn chat(&self, _messages: Vec<ChatMessage>, _config: ChatConfig) -> Result<ChatResponse> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        if self.should_fail_chat {
            return Err(crate::errors::AstrBotError::Provider {
                provider: self.name.clone(),
                message: "Mock chat failure".to_string(),
            });
        }
        Ok(ChatResponse {
            content: self.chat_response.clone(),
            model: self
                .models_list
                .first()
                .cloned()
                .unwrap_or_else(|| "mock".to_string()),
            usage: None,
            reasoning: None,
            tool_calls: None,
        })
    }

    async fn chat_stream(
        &self,
        _messages: Vec<ChatMessage>,
        _config: ChatConfig,
    ) -> Result<Box<dyn futures_util::Stream<Item = Result<ChatStreamChunk>> + Send>> {
        Ok(Box::new(futures_util::stream::empty()))
    }

    async fn embedding(&self, texts: Vec<String>, _model: Option<String>) -> Result<Vec<Vec<f32>>> {
        if !self.embeddings.is_empty() {
            return Ok(self.embeddings.clone());
        }
        Ok(texts.iter().map(|t| self.generate_embedding(t)).collect())
    }

    async fn model_info(&self, model: &str) -> Result<ModelInfo> {
        Ok(ModelInfo {
            name: model.to_string(),
            context_length: 4096,
            supports_streaming: true,
            supports_vision: false,
            supports_function_calling: false,
        })
    }

    async fn health_check(&self) -> Result<bool> {
        Ok(!self.should_fail_health)
    }
}

// ───────────────────────────────────────────────
// MockPlatformAdapter
// ───────────────────────────────────────────────

/// An in-memory platform adapter backed by tokio mpsc channels.
///
/// # Usage
/// ```no_run
/// # use astrbot_core::testing::MockPlatformAdapter;
/// #[tokio::main]
/// async fn main() {
/// let (mut adapter, shared) = MockPlatformAdapter::new("mock", "Mock Platform");
/// adapter.initialize().await.unwrap();
/// adapter.start().await.unwrap();
///
/// // Inject an incoming message
/// shared.inject_message("user1", "session1", "hello").await.unwrap();
///
/// // Collect outgoing replies
/// let replies = adapter.drain_outgoing().await;
/// }
/// ```
pub struct MockPlatformAdapter {
    metadata: PlatformMetadata,
    incoming_tx: tokio::sync::mpsc::Sender<MockIncomingMessage>,
    incoming_rx: Mutex<Option<tokio::sync::mpsc::Receiver<MockIncomingMessage>>>,
    outgoing: Mutex<Vec<(String, MessageChain)>>,
    running: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    handler: Arc<Mutex<HandlerRef>>,
    loop_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// Shared handle for injecting messages into a [`MockPlatformAdapter`].
#[derive(Clone)]
pub struct MockAdapterShared {
    tx: tokio::sync::mpsc::Sender<MockIncomingMessage>,
}

/// Internal struct for an incoming message.
#[derive(Debug, Clone)]
pub struct MockIncomingMessage {
    pub user_id: String,
    pub session_id: String,
    pub text: String,
    pub message_id: String,
}

impl MockPlatformAdapter {
    /// Create a new mock adapter. Returns `(adapter, shared)`.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> (Self, MockAdapterShared) {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let shared = MockAdapterShared { tx: tx.clone() };
        let adapter = Self {
            metadata: PlatformMetadata {
                id: id.into(),
                name: name.into(),
                platform_type: PlatformType::Custom,
                enabled: true,
                extra: HashMap::new(),
            },
            incoming_tx: tx,
            incoming_rx: Mutex::new(Some(rx)),
            outgoing: Mutex::new(Vec::new()),
            running: Arc::new(AtomicBool::new(false)),
            connected: Arc::new(AtomicBool::new(false)),
            handler: Arc::new(Mutex::new(None)),
            loop_handle: Mutex::new(None),
        };
        (adapter, shared)
    }

    pub fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    pub async fn initialize(&mut self) -> Result<()> {
        Ok(())
    }

    pub async fn start(&mut self) -> Result<()> {
        self.running.store(true, Ordering::Relaxed);
        self.connected.store(true, Ordering::Relaxed);

        let rx = {
            let mut guard = self.incoming_rx.lock().await;
            guard
                .take()
                .ok_or_else(|| crate::errors::AstrBotError::Platform {
                    adapter: self.metadata.name.clone(),
                    message: "receiver already taken".to_string(),
                })?
        };

        let running = Arc::clone(&self.running);
        let connected = Arc::clone(&self.connected);
        let handler = Arc::clone(&self.handler);

        let handle = tokio::spawn(async move {
            run_mock_loop(rx, running, connected, handler).await;
        });

        let mut guard = self.loop_handle.lock().await;
        *guard = Some(handle);

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);
        self.connected.store(false, Ordering::Relaxed);

        let mut guard = self.loop_handle.lock().await;
        if let Some(handle) = guard.take() {
            let _ = handle.await;
        }
        Ok(())
    }

    pub async fn send_message(&self, target: &MessageSource, chain: &MessageChain) -> Result<()> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(crate::errors::AstrBotError::Platform {
                adapter: self.metadata.name.clone(),
                message: "adapter not running".to_string(),
            });
        }
        let mut queue = self.outgoing.lock().await;
        queue.push((target.session_id.clone(), chain.clone()));
        Ok(())
    }

    pub async fn reply_message(
        &self,
        original: &AstrBotMessage,
        chain: &MessageChain,
    ) -> Result<()> {
        let mut queue = self.outgoing.lock().await;
        queue.push((original.session_id.clone(), chain.clone()));
        Ok(())
    }

    pub async fn health_check(&self) -> Result<bool> {
        Ok(self.running.load(Ordering::Relaxed) && self.connected.load(Ordering::Relaxed))
    }

    pub async fn set_message_handler(&mut self, handler: Arc<dyn MessageHandler>) {
        let mut h = self.handler.lock().await;
        *h = Some(handler);
    }

    /// Pop the next outgoing message (FIFO).
    pub async fn pop_outgoing(&self) -> Option<(String, MessageChain)> {
        let mut queue = self.outgoing.lock().await;
        if queue.is_empty() {
            None
        } else {
            Some(queue.remove(0))
        }
    }

    /// Drain all outgoing messages.
    pub async fn drain_outgoing(&self) -> Vec<(String, MessageChain)> {
        let mut queue = self.outgoing.lock().await;
        std::mem::take(&mut *queue)
    }
}

impl MockAdapterShared {
    /// Inject a text message into the adapter.
    pub async fn inject_message(
        &self,
        user_id: impl Into<String>,
        session_id: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<()> {
        let msg = MockIncomingMessage {
            user_id: user_id.into(),
            session_id: session_id.into(),
            text: text.into(),
            message_id: uuid::Uuid::new_v4().to_string(),
        };
        self.tx
            .send(msg)
            .await
            .map_err(|_| crate::errors::AstrBotError::Platform {
                adapter: "Mock".to_string(),
                message: "channel closed".to_string(),
            })
    }
}

async fn run_mock_loop(
    mut rx: tokio::sync::mpsc::Receiver<MockIncomingMessage>,
    running: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    handler: Arc<Mutex<HandlerRef>>,
) {
    connected.store(true, std::sync::atomic::Ordering::Relaxed);

    while running.load(Ordering::Relaxed) {
        match tokio::time::timeout(tokio::time::Duration::from_millis(100), rx.recv()).await {
            Ok(Some(msg)) => {
                let sender = MessageMember {
                    user_id: msg.user_id.clone(),
                    nickname: None,
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
                    timestamp: chrono::Utc::now(),
                    platform: PlatformType::Custom,
                    session_id: msg.session_id,
                    sender,
                    message_type: MessageType::Private,
                    chain,
                    raw_payload: None,
                };

                let h = handler.lock().await.clone();
                if let Some(ref handler) = h {
                    handler.on_message(astr_msg).await;
                }
            }
            Ok(None) => break,
            Err(_) => {} // timeout, continue
        }
    }

    connected.store(false, Ordering::Relaxed);
}

// ───────────────────────────────────────────────
// MockMessageHandler
// ───────────────────────────────────────────────

/// A message handler that simply records every message it receives.
pub struct MockMessageHandler {
    messages: Mutex<Vec<AstrBotMessage>>,
}

impl MockMessageHandler {
    pub fn new() -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
        }
    }

    pub async fn drain_messages(&self) -> Vec<AstrBotMessage> {
        let mut guard = self.messages.lock().await;
        std::mem::take(&mut *guard)
    }

    pub async fn last_message(&self) -> Option<AstrBotMessage> {
        let guard = self.messages.lock().await;
        guard.last().cloned()
    }
}

#[async_trait]
impl MessageHandler for MockMessageHandler {
    async fn on_message(&self, message: AstrBotMessage) {
        let mut guard = self.messages.lock().await;
        guard.push(message);
    }
}

impl Default for MockMessageHandler {
    fn default() -> Self {
        Self::new()
    }
}

// ───────────────────────────────────────────────
// Tests for the mocks themselves
// ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_provider_chat() {
        let provider = MockProvider::new("m1", "Mock").with_chat_response("Hello!");
        let resp = provider
            .chat(vec![ChatMessage::user("Hi")], ChatConfig::default())
            .await
            .unwrap();
        assert_eq!(resp.content, "Hello!");
        assert_eq!(provider.chat_count(), 1);
    }

    #[tokio::test]
    async fn test_mock_provider_embedding() {
        let provider = MockProvider::new("m1", "Mock").with_embedding_dim(4);
        let emb = provider
            .embedding(vec!["test".to_string()], None)
            .await
            .unwrap();
        assert_eq!(emb.len(), 1);
        assert_eq!(emb[0].len(), 4);
    }

    #[tokio::test]
    async fn test_mock_provider_failure() {
        let provider = MockProvider::new("m1", "Mock").with_chat_failure();
        let result = provider
            .chat(vec![ChatMessage::user("Hi")], ChatConfig::default())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_adapter_lifecycle() {
        let (mut adapter, _shared) = MockPlatformAdapter::new("mock", "Mock");
        assert!(!adapter.health_check().await.unwrap());
        adapter.initialize().await.unwrap();
        adapter.start().await.unwrap();
        assert!(adapter.health_check().await.unwrap());
        adapter.stop().await.unwrap();
        assert!(!adapter.health_check().await.unwrap());
    }

    #[tokio::test]
    async fn test_mock_adapter_send_and_receive() {
        let (mut adapter, shared) = MockPlatformAdapter::new("mock", "Mock");
        let handler = Arc::new(MockMessageHandler::new());
        adapter.set_message_handler(handler.clone()).await;

        adapter.initialize().await.unwrap();
        adapter.start().await.unwrap();

        shared
            .inject_message("u1", "s1", "hello bot")
            .await
            .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let msgs = handler.drain_messages().await;
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].chain.plain_text(), "hello bot");

        // Send a reply
        let source = MessageSource {
            platform: PlatformType::Custom,
            session_id: "s1".to_string(),
            message_id: "1".to_string(),
            user_id: "u1".to_string(),
        };
        let chain = MessageChain::new().text("Reply");
        adapter.send_message(&source, &chain).await.unwrap();

        let out = adapter.drain_outgoing().await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "s1");
        assert_eq!(out[0].1.plain_text(), "Reply");

        adapter.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_mock_adapter_send_not_running() {
        let (adapter, _shared) = MockPlatformAdapter::new("mock", "Mock");
        let source = MessageSource {
            platform: PlatformType::Custom,
            session_id: "s1".to_string(),
            message_id: "1".to_string(),
            user_id: "u1".to_string(),
        };
        let result = adapter
            .send_message(&source, &MessageChain::new().text("x"))
            .await;
        assert!(result.is_err());
    }
}
