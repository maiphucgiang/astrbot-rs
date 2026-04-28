//! Message quoting / reply utilities
//!
//! Provides cross-platform message reference handling:
//! - Extract quote info from raw platform payloads
//! - Format quoted messages for display
//! - Build reply chains with context

use super::{AstrBotMessage, MessageChain, MessageComponent, MessageMember};
use crate::platform::PlatformType;
use serde::{Deserialize, Serialize};

/// Information about a quoted / referenced message
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuoteInfo {
    /// ID of the quoted message
    pub message_id: String,
    /// ID of the user who sent the quoted message
    pub sender_id: String,
    /// Display name of the sender
    pub sender_name: Option<String>,
    /// The quoted message content (truncated summary)
    pub summary: String,
    /// Original platform type
    pub platform: PlatformType,
    /// Timestamp of the quoted message (if available)
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

/// Utility for extracting and formatting message quotes
pub struct MessageQuoter;

impl MessageQuoter {
    /// Extract quote info from an AstrBotMessage's Reply component
    pub fn extract_from_chain(chain: &MessageChain) -> Option<QuoteInfo> {
        chain.0.iter().find_map(|c| match c {
            MessageComponent::Reply { message_id, chain: quoted_chain } => {
                let summary = quoted_chain.as_ref()
                    .map(|qc| qc.iter()
                        .filter_map(|c| match c {
                            MessageComponent::Plain { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .concat()
                        .chars()
                        .take(100)
                        .collect::<String>())
                    .unwrap_or_default();

                Some(QuoteInfo {
                    message_id: message_id.clone(),
                    sender_id: String::new(), // Unknown from Reply component alone
                    sender_name: None,
                    summary,
                    platform: PlatformType::Custom,
                    timestamp: None,
                })
            }
            _ => None,
        })
    }

    /// Build a human-readable quote prefix for replies
    /// Example: "> [Alice] hello world...\n"
    pub fn format_quote_prefix(quote: &QuoteInfo) -> String {
        let sender = quote.sender_name.as_ref()
            .or_else(|| {
                if quote.sender_id.is_empty() {
                    None
                } else {
                    Some(&quote.sender_id)
                }
            })
            .map(|s| s.as_str())
            .unwrap_or("Unknown");

        let truncated = if quote.summary.len() > 80 {
            format!("{}...", &quote.summary[..80])
        } else {
            quote.summary.clone()
        };

        format!("> [{}] {}\n", sender, truncated)
    }

    /// Build a reply chain that includes a quote reference
    pub fn build_reply_with_quote(
        quote: &QuoteInfo,
        reply_text: impl Into<String>,
    ) -> MessageChain {
        let prefix = Self::format_quote_prefix(quote);
        MessageChain::new()
            .reply(&quote.message_id, None)
            .text(format!("{}{}", prefix, reply_text.into()))
    }

    /// Extract quote from raw platform JSON (platform-specific heuristics)
    pub fn extract_from_raw(platform: PlatformType, raw: &serde_json::Value) -> Option<QuoteInfo> {
        match platform {
            PlatformType::QqOfficial | PlatformType::Aiocqhttp => {
                // QQ: reply message has "reply" field with message_id
                raw.get("reply").and_then(|r| {
                    let msg_id = r.get("message_id")?.as_str()?.to_string();
                    let sender_id = r.get("sender")?.get("user_id")?.as_str()?.to_string();
                    let sender_name = r.get("sender")?.get("nickname")?.as_str().map(String::from);
                    let content = r.get("message")?.as_array().and_then(|arr| {
                        Some(arr.iter()
                            .filter_map(|c| c.get("data")?.get("text")?.as_str())
                            .collect::<Vec<_>>()
                            .concat())
                    }).unwrap_or_default();

                    Some(QuoteInfo {
                        message_id: msg_id,
                        sender_id,
                        sender_name,
                        summary: content.chars().take(100).collect(),
                        platform: PlatformType::QqOfficial,
                        timestamp: None,
                    })
                })
            }
            PlatformType::Telegram => {
                // Telegram: reply_to_message
                raw.get("reply_to_message").and_then(|r| {
                    let msg_id = r.get("message_id")?.as_i64()?.to_string();
                    let sender = r.get("from")?;
                    let sender_id = sender.get("id")?.as_i64()?.to_string();
                    let sender_name = sender.get("username")?
                        .as_str()
                        .map(String::from)
                        .or_else(|| {
                            let first = sender.get("first_name")?.as_str()?;
                            let last = sender.get("last_name").and_then(|l| l.as_str());
                            Some(match last {
                                Some(l) => format!("{} {}", first, l),
                                None => first.to_string(),
                            })
                        });
                    let text = r.get("text")?.as_str()?.to_string();

                    Some(QuoteInfo {
                        message_id: msg_id,
                        sender_id,
                        sender_name,
                        summary: text.chars().take(100).collect(),
                        platform: PlatformType::Telegram,
                        timestamp: None,
                    })
                })
            }
            PlatformType::Discord => {
                // Discord: referenced_message
                raw.get("referenced_message").and_then(|r| {
                    let msg_id = r.get("id")?.as_str()?.to_string();
                    let author = r.get("author")?;
                    let sender_id = author.get("id")?.as_str()?.to_string();
                    let sender_name = author.get("username")?.as_str().map(String::from);
                    let content = r.get("content")?.as_str()?.to_string();

                    Some(QuoteInfo {
                        message_id: msg_id,
                        sender_id,
                        sender_name,
                        summary: content.chars().take(100).collect(),
                        platform: PlatformType::Discord,
                        timestamp: None,
                    })
                })
            }
            PlatformType::Slack => {
                // Slack: thread_ts indicates reply in thread
                raw.get("thread_ts").and_then(|ts| {
                    let msg_id = ts.as_str()?.to_string();
                    let user_id = raw.get("user")?.as_str()?.to_string();
                    let text = raw.get("text")?.as_str()?.to_string();

                    Some(QuoteInfo {
                        message_id: msg_id.clone(),
                        sender_id: user_id,
                        sender_name: None,
                        summary: text.chars().take(100).collect(),
                        platform: PlatformType::Slack,
                        timestamp: None,
                    })
                })
            }
            PlatformType::Feishu => {
                // Lark: parent_id in message body
                raw.get("event").and_then(|e| {
                    let msg = e.get("message")?;
                    let parent_id = msg.get("parent_id")?.as_str()?;
                    let sender_id = msg.get("sender")?.get("sender_id")?.get("open_id")?.as_str()?;
                    let content = msg.get("content")?.as_str().and_then(|s| {
                        serde_json::from_str::<serde_json::Value>(s)
                            .ok()
                            .and_then(|v| v.get("text")?.as_str().map(String::from))
                    }).unwrap_or_default();

                    Some(QuoteInfo {
                        message_id: parent_id.to_string(),
                        sender_id: sender_id.to_string(),
                        sender_name: None,
                        summary: content.chars().take(100).collect(),
                        platform: PlatformType::Feishu,
                        timestamp: None,
                    })
                })
            }
            _ => None,
        }
    }

    /// Parse quote from a message and enrich the chain with it
    pub fn enrich_message_with_quote(
        message: &mut AstrBotMessage,
    ) {
        if let Some(raw) = &message.raw_payload {
            if let Some(quote) = Self::extract_from_raw(message.platform, raw) {
                // Insert Reply component at the beginning of the chain
                let reply = MessageComponent::Reply {
                    message_id: quote.message_id.clone(),
                    chain: Some(vec![MessageComponent::Plain { text: quote.summary.clone() }]),
                };
                message.chain.0.insert(0, reply);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_from_chain() {
        let chain = MessageChain::new()
            .reply("msg123", None)
            .text("hello");

        let quote = MessageQuoter::extract_from_chain(&chain).unwrap();
        assert_eq!(quote.message_id, "msg123");
    }

    #[test]
    fn test_extract_from_chain_with_quoted_content() {
        let chain = MessageChain::new()
            .reply("msg456", Some(vec![
                MessageComponent::Plain { text: "quoted text".to_string() },
            ]))
            .text("response");

        let quote = MessageQuoter::extract_from_chain(&chain).unwrap();
        assert_eq!(quote.message_id, "msg456");
        assert_eq!(quote.summary, "quoted text");
    }

    #[test]
    fn test_format_quote_prefix() {
        let quote = QuoteInfo {
            message_id: "m1".to_string(),
            sender_id: "u1".to_string(),
            sender_name: Some("Alice".to_string()),
            summary: "This is a long message that should be truncated".to_string(),
            platform: PlatformType::Custom,
            timestamp: None,
        };

        let prefix = MessageQuoter::format_quote_prefix(&quote);
        assert!(prefix.contains("Alice"));
        assert!(prefix.starts_with("> "));
    }

    #[test]
    fn test_build_reply_with_quote() {
        let quote = QuoteInfo {
            message_id: "m1".to_string(),
            sender_id: "u1".to_string(),
            sender_name: Some("Bob".to_string()),
            summary: "original".to_string(),
            platform: PlatformType::Custom,
            timestamp: None,
        };

        let chain = MessageQuoter::build_reply_with_quote(&quote, "I agree");
        assert!(chain.contains("reply"));
        assert_eq!(chain.plain_text(), "> [Bob] original\nI agree");
    }

    #[test]
    fn test_extract_from_raw_telegram() {
        let raw = serde_json::json!({
            "reply_to_message": {
                "message_id": 42,
                "from": {
                    "id": 123,
                    "username": "alice",
                    "first_name": "Alice"
                },
                "text": "Hello world"
            }
        });

        let quote = MessageQuoter::extract_from_raw(PlatformType::Telegram, &raw).unwrap();
        assert_eq!(quote.message_id, "42");
        assert_eq!(quote.sender_id, "123");
        assert_eq!(quote.sender_name, Some("alice".to_string()));
        assert_eq!(quote.summary, "Hello world");
    }

    #[test]
    fn test_extract_from_raw_discord() {
        let raw = serde_json::json!({
            "referenced_message": {
                "id": "987654321",
                "author": {
                    "id": "123456",
                    "username": "bob"
                },
                "content": "Check this out"
            }
        });

        let quote = MessageQuoter::extract_from_raw(PlatformType::Discord, &raw).unwrap();
        assert_eq!(quote.message_id, "987654321");
        assert_eq!(quote.sender_id, "123456");
        assert_eq!(quote.sender_name, Some("bob".to_string()));
    }

    #[test]
    fn test_extract_from_raw_no_quote() {
        let raw = serde_json::json!({ "text": "just a message" });
        let quote = MessageQuoter::extract_from_raw(PlatformType::Telegram, &raw);
        assert!(quote.is_none());
    }

    #[test]
    fn test_enrich_message_with_quote() {
        let mut msg = AstrBotMessage {
            message_id: "m2".to_string(),
            timestamp: chrono::Utc::now(),
            platform: PlatformType::Telegram,
            session_id: "s1".to_string(),
            sender: MessageMember {
                user_id: "u2".to_string(),
                nickname: None,
                card: None,
                role: None,
                is_self: false,
            },
            message_type: super::super::MessageType::Group,
            chain: MessageChain::new().text("hi"),
            raw_payload: Some(serde_json::json!({
                "reply_to_message": {
                    "message_id": 99,
                    "from": { "id": 1, "username": "origin" },
                    "text": "previous"
                }
            })),
        };

        MessageQuoter::enrich_message_with_quote(&mut msg);
        assert!(msg.chain.contains("reply"));
    }
}