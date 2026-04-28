use async_trait::async_trait;
use astrbot_core::errors::Result;
use astrbot_core::message::{AstrBotMessage, MessageChain, MessageHandler, HandlerRef, MessageType};
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use crate::adapter::PlatformAdapter;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

/// WeChat Personal account adapter (skeleton)
pub struct WechatPersonalAdapter {
    metadata: PlatformMetadata,
    bridge_endpoint: String,
    running: AtomicBool,
    handler: Option<Arc<dyn MessageHandler>>,
}

impl WechatPersonalAdapter {
    pub fn new(id: String, bridge_endpoint: String) -> Self {
        let metadata = PlatformMetadata {
            id: id.clone(),
            name: format!("WeChat Personal {}", id),
            platform_type: PlatformType::WechatPersonal,
            enabled: true,
            extra: {
                let mut map = std::collections::HashMap::new();
                map.insert("bridge_endpoint".to_string(), serde_json::Value::String(bridge_endpoint.clone()));
                map
            },
        };
        Self {
            metadata,
            bridge_endpoint,
            running: AtomicBool::new(false),
            handler: None,
        }
    }
}

#[async_trait]
impl PlatformAdapter for WechatPersonalAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!("[WechatPersonal] initialized {}", self.metadata.id);
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        self.running.store(true, Ordering::Relaxed);
        info!("[WechatPersonal] started {}", self.metadata.id);
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);
        info!("[WechatPersonal] stopped {}", self.metadata.id);
        Ok(())
    }

    async fn send_message(&self, target: &MessageSource, chain: &MessageChain) -> Result<()> {
        let text = chain.plain_text();
        info!("[WechatPersonal] send (skeleton): {}", text);
        Ok(())
    }

    async fn reply_message(&self, original: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
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
    async fn test_wechat_personal_lifecycle() {
        let mut adapter = WechatPersonalAdapter::new(
            "wx-personal-1".to_string(),
            "http://localhost:8080".to_string(),
        );
        assert_eq!(adapter.metadata().platform_type, PlatformType::WechatPersonal);
        adapter.initialize().await.unwrap();
        adapter.start().await.unwrap();
        assert!(adapter.health_check().await.unwrap());
        adapter.stop().await.unwrap();
    }
}
