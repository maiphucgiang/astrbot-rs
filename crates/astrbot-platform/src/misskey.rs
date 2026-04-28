use async_trait::async_trait;
use astrbot_core::errors::Result;
use astrbot_core::message::{AstrBotMessage, MessageChain, MessageHandler, HandlerRef, MessageType};
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use crate::adapter::PlatformAdapter;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

/// Misskey platform adapter (skeleton)
pub struct MisskeyAdapter {
    metadata: PlatformMetadata,
    instance_url: String,
    api_token: String,
    running: AtomicBool,
    handler: Option<Arc<dyn MessageHandler>>,
}

impl MisskeyAdapter {
    pub fn new(id: String, instance_url: String, api_token: String) -> Self {
        let metadata = PlatformMetadata {
            id: id.clone(),
            name: format!("Misskey {}", id),
            platform_type: PlatformType::Misskey,
            enabled: true,
            extra: {
                let mut map = std::collections::HashMap::new();
                map.insert("instance_url".to_string(), serde_json::Value::String(instance_url.clone()));
                map
            },
        };
        Self {
            metadata,
            instance_url,
            api_token,
            running: AtomicBool::new(false),
            handler: None,
        }
    }
}

#[async_trait]
impl PlatformAdapter for MisskeyAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!("[Misskey] initialized {} on {}", self.metadata.id, self.instance_url);
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        self.running.store(true, Ordering::Relaxed);
        info!("[Misskey] started {}", self.metadata.id);
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);
        info!("[Misskey] stopped {}", self.metadata.id);
        Ok(())
    }

    async fn send_message(
        &self,
        _target: &MessageSource,
        chain: &MessageChain,
    ) -> Result<()> {
        let text = chain.plain_text();
        info!("[Misskey] send note (skeleton): {}", text);
        Ok(())
    }

    async fn reply_message(
        &self,
        original: &AstrBotMessage,
        chain: &MessageChain,
    ) -> Result<()> {
        let source = MessageSource {
            platform: original.platform,
            session_id: original.session_id.clone(),
            message_id: original.message_id.clone(),
            user_id: original.sender.user_id.clone(),
        };
        self.send_message(&source, chain).await
    }

    async fn health_check(&self) -> Result<bool> {
        Ok(self.running.load(Ordering::Relaxed))
    }

    fn set_message_handler(&mut self, handler: Arc<dyn MessageHandler>) {
        self.handler = Some(handler);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_misskey_lifecycle() {
        let mut adapter = MisskeyAdapter::new(
            "misskey-1".to_string(),
            "https://misskey.io".to_string(),
            "test-token".to_string(),
        );
        assert_eq!(adapter.metadata().platform_type, PlatformType::Misskey);
        adapter.initialize().await.unwrap();
        adapter.start().await.unwrap();
        assert!(adapter.health_check().await.unwrap());
        adapter.stop().await.unwrap();
    }
}
