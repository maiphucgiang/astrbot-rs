use async_trait::async_trait;
use astrbot_core::errors::Result;
use astrbot_core::message::{AstrBotMessage, MessageChain, MessageHandler};
use astrbot_core::platform::{MessageSource, PlatformMetadata};
use std::sync::Arc;

#[async_trait]
pub trait PlatformAdapter: Send + Sync {
    fn metadata(&self) -> &PlatformMetadata;

    async fn initialize(&mut self) -> Result<()> {
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        Ok(())
    }

    async fn send_message(&self, _target: &MessageSource, _chain: &MessageChain) -> Result<()> {
        Ok(())
    }

    async fn reply_message(&self, _original: &AstrBotMessage, _chain: &MessageChain) -> Result<()> {
        Ok(())
    }

    async fn health_check(&self) -> Result<bool> {
        Ok(true)
    }

    fn set_message_handler(&mut self, _handler: Arc<dyn MessageHandler>) {}

    async fn send_voice(
        &self,
        _target: &MessageSource,
        _data: Vec<u8>,
        _format: &str,
    ) -> Result<()> {
        Err(astrbot_core::errors::AstrBotError::NotImplemented(
            "send_voice not supported by this adapter".to_string(),
        ))
    }
}
