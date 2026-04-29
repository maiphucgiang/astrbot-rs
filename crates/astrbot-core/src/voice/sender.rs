//! Voice message sender — skeleton for sending voice/audio via platform adapters

use crate::errors::Result;
use crate::message::{MessageChain, MessageComponent};
use crate::platform::MessageSource;

/// Trait for adapters that support voice message sending
#[async_trait::async_trait]
pub trait VoiceSender: Send + Sync {
    /// Upload voice file and return a URL or file ID usable by the platform
    async fn upload_voice(
        &self,
        source: &MessageSource,
        data: Vec<u8>,
        format: &str,
    ) -> Result<String>;

    /// Build a voice MessageComponent from uploaded data
    fn voice_component(url_or_id: String) -> MessageComponent {
        MessageComponent::Voice {
            url: Some(url_or_id),
            file_id: None,
            base64: None,
        }
    }
}

/// Helper to detect if a chain contains voice content
pub fn contains_voice(chain: &MessageChain) -> bool {
    chain
        .0
        .iter()
        .any(|c| matches!(c, MessageComponent::Voice { .. }))
}

/// Helper to extract voice URLs from a chain
pub fn extract_voice_urls(chain: &MessageChain) -> Vec<String> {
    chain
        .0
        .iter()
        .filter_map(|c| match c {
            MessageComponent::Voice { url: Some(u), .. } => Some(u.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::MessageChain;

    #[test]
    fn test_contains_voice() {
        let mut chain = MessageChain::new().text("Hello");
        chain.0.push(MessageComponent::Voice {
            url: Some("https://example.com/voice.ogg".to_string()),
            file_id: None,
            base64: None,
        });
        assert!(contains_voice(&chain));
    }

    #[test]
    fn test_extract_voice_urls() {
        let mut chain = MessageChain::new().text("Hello");
        chain.0.push(MessageComponent::Voice {
            url: Some("https://example.com/voice.ogg".to_string()),
            file_id: None,
            base64: None,
        });
        let urls = extract_voice_urls(&chain);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0], "https://example.com/voice.ogg");
    }
}
