use async_trait::async_trait;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::message::{AstrBotMessage, MessageChain, MessageHandler, HandlerRef, MessageType};
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use crate::adapter::PlatformAdapter;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

/// WeCom Bot (webhook robot) adapter
pub struct WecomBotAdapter {
    metadata: PlatformMetadata,
    webhook_url: String,
    secret: Option<String>,
    running: AtomicBool,
    handler: Option<Arc<dyn MessageHandler>>,
}

impl WecomBotAdapter {
    pub fn new(id: String, webhook_url: String, secret: Option<String>) -> Self {
        let metadata = PlatformMetadata {
            id: id.clone(),
            name: format!("WeCom Bot {}", id),
            platform_type: PlatformType::WecomBot,
            enabled: true,
            extra: {
                let mut map = std::collections::HashMap::new();
                map.insert("webhook_url".to_string(), serde_json::Value::String(webhook_url.clone()));
                if let Some(ref s) = secret {
                    map.insert("secret".to_string(), serde_json::Value::String(s.clone()));
                }
                map
            },
        };
        Self {
            metadata,
            webhook_url,
            secret,
            running: AtomicBool::new(false),
            handler: None,
        }
    }
}

#[async_trait]
impl PlatformAdapter for WecomBotAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!("[WecomBot] initialized {}", self.metadata.id);
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        self.running.store(true, Ordering::Relaxed);
        info!("[WecomBot] started {}", self.metadata.id);
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);
        info!("[WecomBot] stopped {}", self.metadata.id);
        Ok(())
    }

    async fn send_message(&self, _target: &MessageSource, chain: &MessageChain) -> Result<()> {
        let text = chain.plain_text();
        info!("[WecomBot] send to webhook (skeleton): {}", text);
        // Skeleton: in production, POST to webhook_url with text payload
        Ok(())
    }

    async fn reply_message(&self, _original: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
        self.send_message(&MessageSource {
            platform: PlatformType::WecomBot,
            session_id: "default".to_string(),
            message_id: "0".to_string(),
            user_id: "0".to_string(),
        }, chain).await
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
    async fn test_wecombot_lifecycle() {
        let mut adapter = WecomBotAdapter::new(
            "wecom-bot-1".to_string(),
            "https://qyapi.weixin.qq.com/cgi-bin/webhook/send".to_string(),
            Some("secret123".to_string()),
        );
        assert_eq!(adapter.metadata().platform_type, PlatformType::WecomBot);
        adapter.initialize().await.unwrap();
        adapter.start().await.unwrap();
        assert!(adapter.health_check().await.unwrap());
        adapter.stop().await.unwrap();
    }
}
