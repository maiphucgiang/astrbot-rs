use async_trait::async_trait;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::platform::MessageSource;

#[async_trait]
pub trait PlatformAdapter: Send + Sync {
    fn name(&self) -> &str;

    async fn send_voice(
        &self,
        _target: &MessageSource,
        _data: Vec<u8>,
        _format: &str,
    ) -> Result<()> {
        Err(AstrBotError::NotImplemented(
            "send_voice not supported by this adapter".to_string(),
        ))
    }
}
