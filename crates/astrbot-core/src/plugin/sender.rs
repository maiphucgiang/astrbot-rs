use crate::errors::Result;
use crate::message::MessageChain;
use crate::platform::MessageSource;
use async_trait::async_trait;

/// Trait for sending messages from plugins
#[async_trait]
pub trait MessageSender: Send + Sync {
    /// Send a message chain to a target
    async fn send(&self, source: &MessageSource, chain: MessageChain) -> Result<()>;

    /// Send plain text (convenience)
    async fn send_text(&self, source: &MessageSource, text: &str) -> Result<()> {
        self.send(source, MessageChain::new().text(text)).await
    }
}

/// Wrapper type alias for the sender arc
pub type PluginMessageSender = std::sync::Arc<dyn MessageSender>;
