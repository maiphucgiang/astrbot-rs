use crate::platform::{MessageSource, PlatformType};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Type of message event
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    /// Private / direct message
    Private,
    /// Group message
    Group,
    /// Channel message
    Channel,
    /// Unknown / other
    Unknown,
}

/// Member information in a chat
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageMember {
    /// User ID
    pub user_id: String,
    /// Nickname
    pub nickname: Option<String>,
    /// Group card name (group-specific nickname)
    pub card: Option<String>,
    /// Role in the group (owner, admin, member)
    pub role: Option<String>,
    /// Whether the user is the bot itself
    pub is_self: bool,
}

impl Default for MessageMember {
    fn default() -> Self {
        Self {
            user_id: String::new(),
            nickname: None,
            card: None,
            role: None,
            is_self: false,
        }
    }
}

/// A component within a message chain
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageComponent {
    /// Plain text
    Plain { text: String },
    /// Mention / at someone
    At {
        target: String,
        display: Option<String>,
    },
    /// Image
    Image {
        url: Option<String>,
        file_id: Option<String>,
        base64: Option<String>,
    },
    /// Voice / audio
    Voice {
        url: Option<String>,
        file_id: Option<String>,
        base64: Option<String>,
    },
    /// File
    File {
        name: String,
        url: Option<String>,
        file_id: Option<String>,
    },
    /// Reply / quote
    Reply {
        message_id: String,
        chain: Option<Vec<MessageComponent>>,
    },
    /// Forward / share
    Forward {
        message_id: String,
        summary: Option<String>,
    },
    /// JSON payload (rich message)
    Json { data: serde_json::Value },
    /// XML payload
    Xml { data: String },
}

/// A chain of message components
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MessageChain(pub Vec<MessageComponent>);

impl MessageChain {
    /// Get all components as a slice
    pub fn components(&self) -> &[MessageComponent] {
        &self.0
    }

    /// Get all components as a mutable slice
    pub fn components_mut(&mut self) -> &mut [MessageComponent] {
        &mut self.0
    }

    /// Create an empty message chain
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a plain text component
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.0.push(MessageComponent::Plain { text: text.into() });
        self
    }

    /// Add an @ mention
    pub fn at(mut self, target: impl Into<String>) -> Self {
        self.0.push(MessageComponent::At {
            target: target.into(),
            display: None,
        });
        self
    }

    /// Add an image
    pub fn image_url(mut self, url: impl Into<String>) -> Self {
        self.0.push(MessageComponent::Image {
            url: Some(url.into()),
            file_id: None,
            base64: None,
        });
        self
    }

    /// Add a reply / quote reference
    pub fn reply(
        mut self,
        message_id: impl Into<String>,
        chain: Option<Vec<MessageComponent>>,
    ) -> Self {
        self.0.push(MessageComponent::Reply {
            message_id: message_id.into(),
            chain,
        });
        self
    }

    /// Get plain text concatenation
    pub fn plain_text(&self) -> String {
        self.0
            .iter()
            .filter_map(|c| match c {
                MessageComponent::Plain { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .concat()
            .to_string()
    }

    /// Check if chain contains a specific component type
    pub fn contains(&self, component_type: &str) -> bool {
        self.0.iter().any(|c| match (c, component_type) {
            (MessageComponent::Plain { .. }, "plain") => true,
            (MessageComponent::At { .. }, "at") => true,
            (MessageComponent::Image { .. }, "image") => true,
            (MessageComponent::Voice { .. }, "voice") => true,
            (MessageComponent::File { .. }, "file") => true,
            (MessageComponent::Reply { .. }, "reply") => true,
            _ => false,
        })
    }

    /// Check if this is a command (starts with a prefix character)
    pub fn is_command(&self, prefixes: &[char]) -> bool {
        let text = self.plain_text();
        !text.is_empty() && prefixes.iter().any(|p| text.starts_with(*p))
    }

    /// Extract command name and args if this is a command
    pub fn parse_command(&self, prefixes: &[char]) -> Option<(String, Vec<String>)> {
        let text = self.plain_text();
        let stripped = prefixes
            .iter()
            .find_map(|p| text.strip_prefix(*p).map(|s| s.trim()))?;
        if stripped.is_empty() {
            return None;
        }
        let mut parts = stripped.split_whitespace().map(String::from);
        let cmd = parts.next()?;
        let args = parts.collect();
        Some((cmd, args))
    }
}

/// A raw incoming message from any platform
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AstrBotMessage {
    /// Unique message ID from the platform
    pub message_id: String,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// Source platform
    pub platform: PlatformType,
    /// Session / chat ID
    pub session_id: String,
    /// Sender information
    pub sender: MessageMember,
    /// Message type
    pub message_type: MessageType,
    /// Message content
    pub chain: MessageChain,
    /// Raw platform-specific payload (for debugging / advanced use)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_payload: Option<serde_json::Value>,
}

impl Default for AstrBotMessage {
    fn default() -> Self {
        Self {
            message_id: String::new(),
            timestamp: Utc::now(),
            platform: PlatformType::Custom,
            session_id: String::new(),
            sender: MessageMember::default(),
            message_type: MessageType::Unknown,
            chain: MessageChain::default(),
            raw_payload: None,
        }
    }
}

/// Handler for incoming messages from platform adapters
#[async_trait::async_trait]
pub trait MessageHandler: Send + Sync {
    async fn on_message(&self, message: AstrBotMessage);
}

/// Type alias for shared message handler
pub type HandlerRef = Option<std::sync::Arc<dyn MessageHandler>>;

/// Result of processing a message event
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageEventResult {
    /// Reply to the message
    Reply { chain: MessageChain },
    /// No reply needed
    Nothing,
    /// Forward to another session
    Forward {
        target: MessageSource,
        chain: MessageChain,
    },
}

impl MessageEventResult {
    /// Create a reply result
    pub fn reply(chain: MessageChain) -> Self {
        Self::Reply { chain }
    }

    /// Create a text-only reply
    pub fn reply_text(text: impl Into<String>) -> Self {
        Self::Reply {
            chain: MessageChain::new().text(text),
        }
    }

    /// Create a nothing result
    pub fn nothing() -> Self {
        Self::Nothing
    }
}

pub mod quote;
