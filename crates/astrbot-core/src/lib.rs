pub mod event;
pub mod message;
pub mod config;
pub mod metrics;
pub mod session;
pub mod types;

pub use event::*;
pub use message::*;
pub use config::*;
pub use types::*;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 核心消息类型，所有平台适配器统一转换为该类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AstrMessage {
    pub id: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub platform: String,
    pub sender: SenderInfo,
    pub content: MessageContent,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SenderInfo {
    pub id: String,
    pub name: Option<String>,
    pub avatar: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum MessageContent {
    Text(String),
    Image { url: String, mime_type: Option<String> },
    File { name: String, url: String, size: Option<u64> },
    Voice { url: String, duration: Option<u64> },
    Video { url: String, duration: Option<u64> },
    At { target: String },
    Reply { message_id: String },
}

impl AstrMessage {
    pub fn new_text(platform: &str, sender_id: &str, text: &str, session_id: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now(),
            platform: platform.to_string(),
            sender: SenderInfo {
                id: sender_id.to_string(),
                name: None,
                avatar: None,
            },
            content: MessageContent::Text(text.to_string()),
            session_id: session_id.to_string(),
        }
    }
}
