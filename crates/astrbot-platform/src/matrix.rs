use async_trait::async_trait;
use astrbot_core::errors::Result;
use astrbot_core::message::{AstrBotMessage, MessageChain, MessageHandler, HandlerRef, MessageType};
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use crate::adapter::PlatformAdapter;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

/// Matrix platform adapter (skeleton)
pub struct MatrixAdapter {
    metadata: PlatformMetadata,
    homeserver: String,
    access_token: String,
    room_id: Option<String>,
    running: AtomicBool,
    handler: Option<Arc<dyn MessageHandler>>,
}

impl MatrixAdapter {
    pub fn new(
        id: String,
        homeserver: String,
        access_token: String,
        room_id: Option<String>,
    ) -> Self {
        let metadata = PlatformMetadata {
            id: id.clone(),
            name: format!("Matrix {}", id),
            platform_type: PlatformType::Matrix,
            enabled: true,
            extra: {
                let mut map = std::collections::HashMap::new();
                map.insert("homeserver".to_string(), serde_json::Value::String(homeserver.clone()));
                if let Some(ref rid) = room_id {
                    map.insert("room_id".to_string(), serde_json::Value::String(rid.clone()));
                }
                map
            },
        };
        Self {
            metadata,
            homeserver,
            access_token,
            room_id,
            running: AtomicBool::new(false),
            handler: None,
        }
    }
}

#[async_trait]
impl PlatformAdapter for MatrixAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!("[Matrix] initialized {} on {}", self.metadata.id, self.homeserver);
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        self.running.store(true, Ordering::Relaxed);
        info!("[Matrix] started {}", self.metadata.id);
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);
        info!("[Matrix] stopped {}", self.metadata.id);
        Ok(())
    }

    async fn send_message(
        &self,
        _target: &MessageSource,
        chain: &MessageChain,
    ) -> Result<()> {
        let text = chain.plain_text();
        info!("[Matrix] send message (skeleton): {}", text);
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
    async fn test_matrix_lifecycle() {
        let mut adapter = MatrixAdapter::new(
            "matrix-1".to_string(),
            "https://matrix.org".to_string(),
            "syt_test".to_string(),
            Some("!room:matrix.org".to_string()),
        );
        assert_eq!(adapter.metadata().platform_type, PlatformType::Matrix);
        adapter.initialize().await.unwrap();
        adapter.start().await.unwrap();
        assert!(adapter.health_check().await.unwrap());
        adapter.stop().await.unwrap();
    }
}
