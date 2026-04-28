//! Proactive messaging — allows the bot to initiate conversations
//!
//! Provides APIs for scheduled / trigger-based proactive message sending.

use crate::errors::{AstrBotError, Result};
use crate::message::MessageChain;
use std::collections::HashMap;
use tokio::sync::Mutex;
use tracing::info;

/// A proactive conversation target
#[derive(Debug, Clone)]
pub struct ProactiveTarget {
    pub platform: String,
    pub channel_id: String,
    pub user_id: Option<String>,
}

/// Proactive message scheduler
#[derive(Default)]
pub struct ProactiveScheduler {
    targets: Mutex<HashMap<String, ProactiveTarget>>,
}

impl ProactiveScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a target that can receive proactive messages
    pub async fn register_target(
        &self,
        id: String,
        target: ProactiveTarget,
    ) {
        let mut targets = self.targets.lock().await;
        targets.insert(id, target);
    }

    /// Remove a registered target
    pub async fn unregister_target(&self, id: &str) -> Option<ProactiveTarget> {
        let mut targets = self.targets.lock().await;
        targets.remove(id)
    }

    /// List all registered targets
    pub async fn list_targets(&self) -> Vec<(String, ProactiveTarget)> {
        let targets = self.targets.lock().await;
        targets.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// Send a proactive message to a target (skeleton)
    pub async fn send_message(
        &self,
        target_id: &str,
        _message: MessageChain,
    ) -> Result<()> {
        let targets = self.targets.lock().await;
        let _target = targets
            .get(target_id)
            .ok_or_else(|| AstrBotError::NotFound(format!("Proactive target '{}' not found", target_id)))?;

        info!("[Proactive] send_message skeleton — target={}", target_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_and_unregister() {
        let scheduler = ProactiveScheduler::new();
        let target = ProactiveTarget {
            platform: "qq".to_string(),
            channel_id: "12345".to_string(),
            user_id: Some("user1".to_string()),
        };
        scheduler.register_target("t1".to_string(), target.clone()).await;
        assert_eq!(scheduler.list_targets().await.len(), 1);

        let removed = scheduler.unregister_target("t1").await;
        assert!(removed.is_some());
        assert_eq!(scheduler.list_targets().await.len(), 0);
    }

    #[tokio::test]
    async fn test_send_message_not_found() {
        let scheduler = ProactiveScheduler::new();
        let result = scheduler
            .send_message("missing", MessageChain::new())
            .await;
        assert!(result.is_err());
    }
}
