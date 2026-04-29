//! End-to-end pipeline test using the mock testing framework.
//!
//! This demonstrates the full flow:
//! fake message → MockPlatformAdapter → MessageHandler →
//! Provider.chat() → reply → MockPlatformAdapter outgoing queue

use crate::message::{MessageChain, MessageEventResult, MessageHandler};
use crate::platform::{MessageSource, PlatformType};
use crate::provider::{ChatConfig, ChatMessage, Provider};
use crate::testing::{MockAdapterShared, MockMessageHandler, MockPlatformAdapter, MockProvider};
use std::sync::Arc;

/// A simple message handler that invokes a provider and produces a reply.
struct EchoHandler {
    provider: Arc<dyn Provider>,
    system_prompt: String,
}

impl EchoHandler {
    fn new(provider: Arc<dyn Provider>, system_prompt: impl Into<String>) -> Self {
        Self {
            provider,
            system_prompt: system_prompt.into(),
        }
    }
}

#[async_trait::async_trait]
impl MessageHandler for EchoHandler {
    async fn on_message(&self, message: crate::message::AstrBotMessage) {
        let text = message.chain.plain_text();
        let messages = vec![
            ChatMessage::system(&self.system_prompt),
            ChatMessage::user(&text),
        ];

        let config = ChatConfig::default();
        match self.provider.chat(messages, config).await {
            Ok(response) => {
                let chain = MessageChain::new().text(response.content);
                // In a real pipeline this would route back to the adapter.
                // Here we just log / verify through the test.
                tracing::debug!("Handler produced reply: {}", chain.plain_text());
            }
            Err(e) => {
                tracing::error!("Handler error: {}", e);
            }
        }
    }
}

/// A handler that sends replies back through a MockPlatformAdapter.
struct ReplyViaAdapterHandler {
    provider: Arc<dyn Provider>,
    adapter: Arc<MockPlatformAdapter>,
    system_prompt: String,
}

impl ReplyViaAdapterHandler {
    fn new(
        provider: Arc<dyn Provider>,
        adapter: Arc<MockPlatformAdapter>,
        system_prompt: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            adapter,
            system_prompt: system_prompt.into(),
        }
    }
}

#[async_trait::async_trait]
impl MessageHandler for ReplyViaAdapterHandler {
    async fn on_message(&self, message: crate::message::AstrBotMessage) {
        let text = message.chain.plain_text();
        let messages = vec![
            ChatMessage::system(&self.system_prompt),
            ChatMessage::user(&text),
        ];

        let config = ChatConfig::default();
        let reply_text = match self.provider.chat(messages, config).await {
            Ok(response) => response.content,
            Err(_) => "Sorry, I couldn't process that.".to_string(),
        };

        let source = MessageSource {
            platform: PlatformType::Custom,
            session_id: message.session_id.clone(),
            message_id: message.message_id.clone(),
            user_id: message.sender.user_id.clone(),
        };
        let chain = MessageChain::new().text(reply_text);

        // Send reply back through the adapter
        let _ = self.adapter.send_message(&source, &chain).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{
        MockAdapterShared, MockMessageHandler, MockPlatformAdapter, MockProvider,
    };

    #[tokio::test]
    async fn test_e2e_fake_message_to_provider_reply() {
        // Setup: create mock provider with a canned response
        let provider =
            Arc::new(MockProvider::new("mock", "Mock").with_chat_response("Hello from mock LLM!"));

        // Setup: create mock adapter
        let (mut adapter, shared) = MockPlatformAdapter::new("mock", "Mock Platform");

        // Setup: handler that replies back through the adapter
        // Note: we wrap in Arc first, then use get_mut to set handler before any clones
        let mut adapter = Arc::new(adapter);
        let handler = Arc::new(ReplyViaAdapterHandler::new(
            provider.clone(),
            adapter.clone(),
            "You are a helpful test assistant.",
        ));

        // Give the adapter our handler (safe because no other refs yet)
        if let Some(a) = Arc::get_mut(&mut adapter) {
            a.set_message_handler(handler);
        }
        adapter.initialize().await.unwrap();
        adapter.start().await.unwrap();

        // Give the adapter our handler
        adapter.set_message_handler(handler);
        adapter.initialize().await.unwrap();
        adapter.start().await.unwrap();

        // Act: inject a fake user message
        shared
            .inject_message("user1", "session_abc", "Say hello")
            .await
            .unwrap();

        // Wait for the async loop to process
        tokio::time::sleep(tokio::time::Duration::from_millis(80)).await;

        // Assert: the adapter should have an outgoing reply
        let outgoing = adapter.drain_outgoing().await;
        assert_eq!(outgoing.len(), 1, "Should have exactly one outgoing reply");
        assert_eq!(
            outgoing[0].0, "session_abc",
            "Reply should target the correct session"
        );
        assert_eq!(
            outgoing[0].1.plain_text(),
            "Hello from mock LLM!",
            "Reply content should match provider response"
        );

        // Assert: provider was actually called
        assert_eq!(
            provider.chat_count(),
            1,
            "Provider should have been called once"
        );

        adapter.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_e2e_multi_message_conversation() {
        let provider =
            Arc::new(MockProvider::new("mock", "Mock").with_chat_response("Acknowledged."));
        let (mut adapter, shared) = MockPlatformAdapter::new("mock", "Mock Platform");
        let adapter = Arc::new(adapter);

        let handler = Arc::new(ReplyViaAdapterHandler::new(
            provider.clone(),
            adapter.clone(),
            "You are a test bot.",
        ));
        adapter.set_message_handler(handler);
        adapter.initialize().await.unwrap();
        adapter.start().await.unwrap();

        // Inject three messages
        shared.inject_message("u1", "s1", "Msg 1").await.unwrap();
        shared.inject_message("u2", "s2", "Msg 2").await.unwrap();
        shared.inject_message("u1", "s1", "Msg 3").await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let outgoing = adapter.drain_outgoing().await;
        assert_eq!(outgoing.len(), 3, "Should have three replies");
        assert_eq!(
            provider.chat_count(),
            3,
            "Provider should have been called three times"
        );

        // Verify session routing
        let s1_replies: Vec<_> = outgoing.iter().filter(|(sid, _)| sid == "s1").collect();
        let s2_replies: Vec<_> = outgoing.iter().filter(|(sid, _)| sid == "s2").collect();
        assert_eq!(s1_replies.len(), 2);
        assert_eq!(s2_replies.len(), 1);

        adapter.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_e2e_provider_failure_fallback() {
        // Provider that always fails
        let provider = Arc::new(MockProvider::new("mock", "Mock").with_chat_failure());
        let (mut adapter, shared) = MockPlatformAdapter::new("mock", "Mock Platform");
        let adapter = Arc::new(adapter);

        let handler = Arc::new(ReplyViaAdapterHandler::new(
            provider.clone(),
            adapter.clone(),
            "You are a test bot.",
        ));
        adapter.set_message_handler(handler);
        adapter.initialize().await.unwrap();
        adapter.start().await.unwrap();

        shared
            .inject_message("u1", "s1", "trigger failure")
            .await
            .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(80)).await;

        // Even with provider failure, handler should still produce a fallback reply
        let outgoing = adapter.drain_outgoing().await;
        assert_eq!(outgoing.len(), 1);
        assert_eq!(
            outgoing[0].1.plain_text(),
            "Sorry, I couldn't process that."
        );

        adapter.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_e2e_handler_records_messages() {
        let (mut adapter, shared) = MockPlatformAdapter::new("mock", "Mock Platform");
        let handler = Arc::new(MockMessageHandler::new());
        adapter.set_message_handler(handler.clone());
        adapter.initialize().await.unwrap();
        adapter.start().await.unwrap();

        shared.inject_message("u1", "s1", "hello").await.unwrap();
        shared.inject_message("u2", "s1", "world").await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(80)).await;

        let msgs = handler.drain_messages().await;
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].chain.plain_text(), "hello");
        assert_eq!(msgs[1].chain.plain_text(), "world");

        adapter.stop().await.unwrap();
    }
}
