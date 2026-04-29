//! Proactive messaging — allows the bot to initiate conversations
//!
//! Provides APIs for scheduled / trigger-based proactive message sending.

use crate::errors::{AstrBotError, Result};
use crate::message::MessageChain;
use crate::platform::{MessageSource, PlatformType};
use crate::plugin::MessageSender;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info};

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
    sender: Mutex<Option<Arc<dyn MessageSender>>>,
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

    /// Set the message sender for proactive delivery
    pub async fn set_sender(&self, sender: Arc<dyn MessageSender>) {
        let mut s = self.sender.lock().await;
        *s = Some(sender);
    }

    /// Send a proactive message to a target
    pub async fn send_message(
        &self,
        target_id: &str,
        message: MessageChain,
    ) -> Result<()> {
        let targets = self.targets.lock().await;
        let target = targets
            .get(target_id)
            .ok_or_else(|| AstrBotError::NotFound(format!("Proactive target '{}' not found", target_id)))?;

        let sender = self.sender.lock().await;
        let sender = sender
            .as_ref()
            .ok_or_else(|| AstrBotError::Internal("Message sender not set in ProactiveScheduler".to_string()))?;

        // Build MessageSource from ProactiveTarget
        let platform = match target.platform.as_str() {
            "qq" => PlatformType::QqOfficial,
            "aiocqhttp" => PlatformType::Aiocqhttp,
            "telegram" => PlatformType::Telegram,
            "discord" => PlatformType::Discord,
            "feishu" | "lark" => PlatformType::Feishu,
            "wecom" => PlatformType::Wecom,
            "weixin" => PlatformType::Weixin,
            "dingtalk" => PlatformType::Dingtalk,
            "line" => PlatformType::Line,
            "mattermost" => PlatformType::Mattermost,
            "slack" => PlatformType::Slack,
            "webchat" => PlatformType::Webchat,
            "webhook" => PlatformType::Webhook,
            "satori" => PlatformType::Satori,
            "wechat_personal" => PlatformType::WechatPersonal,
            "wecom_bot" => PlatformType::WecomBot,
            "misskey" => PlatformType::Misskey,
            "matrix" => PlatformType::Matrix,
            _ => PlatformType::Custom,
        };

        let source = MessageSource {
            platform,
            session_id: target.channel_id.clone(),
            message_id: String::new(), // proactive messages have no original message id
            user_id: target.user_id.clone().unwrap_or_default(),
        };

        sender.send(&source, message).await?;
        info!("[Proactive] message sent to target={}", target_id);
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
