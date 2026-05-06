use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub mod pipeline_bridge;

/// Platform adapter types supported by AstrBot
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlatformType {
    /// QQ Official Bot (Webhook)
    #[default]
    QqOfficial,
    /// QQ via Aiocqhttp (OneBot 11)
    Aiocqhttp,
    /// Telegram
    Telegram,
    /// Discord
    Discord,
    /// Feishu (Lark)
    Feishu,
    /// WeCom (Enterprise WeChat)
    Wecom,
    /// WeChat Official Account
    Weixin,
    /// DingTalk
    Dingtalk,
    /// Line
    Line,
    /// Mattermost
    Mattermost,
    /// Slack
    Slack,
    /// WebChat (built-in web interface)
    Webchat,
    /// Generic Webhook
    Webhook,
    /// Custom adapter
    Custom,
    /// Satori protocol
    Satori,
    /// WeChat Personal (personal account)
    WechatPersonal,
    /// WeCom Bot (webhook robot)
    WecomBot,
    /// Misskey
    Misskey,
    /// Matrix
    Matrix,
}

/// Unique identifier for a message source
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct MessageSource {
    /// Platform type
    pub platform: PlatformType,
    /// Unique session identifier (chat/group ID)
    pub session_id: String,
    /// Unique message identifier
    pub message_id: String,
    /// User identifier
    pub user_id: String,
}

/// Metadata about a platform adapter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformMetadata {
    /// Unique adapter ID
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Platform type
    pub platform_type: PlatformType,
    /// Whether the adapter is enabled
    pub enabled: bool,
    /// Additional configuration
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}
