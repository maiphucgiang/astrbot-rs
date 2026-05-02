use crate::agent::context::{TokenCounter, ContextCompressor, ContextTruncator};
use crate::provider::ChatMessage as Message;

pub struct ContextManager {
    counter: TokenCounter,
    compressor: ContextCompressor,
    truncator: ContextTruncator,
}

impl Default for ContextManager {
    fn default() -> Self {
        Self {
            counter: TokenCounter::new(),
            compressor: ContextCompressor::new(4),
            truncator: ContextTruncator::new(4096, 1),
        }
    }
}

impl ContextManager {
    pub fn new() -> Self { Self::default() }
    pub fn with_budget(max_tokens: usize) -> Self {
        Self {
            counter: TokenCounter::new(),
            compressor: ContextCompressor::new(4),
            truncator: ContextTruncator::new(max_tokens, 1),
        }
    }
    pub fn prepare_context(&self, history: &[Message], max_tokens: usize) -> Vec<Message> {
        let mut working = history.to_vec();
        self.compressor.compress(&mut working);
        let truncator = ContextTruncator::new(max_tokens, 1);
        truncator.truncate(&working)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prepare_context_under_budget() {
        let cm = ContextManager::with_budget(10000);
        let history = vec![
            Message { role: "user".to_string(), content: "hello".to_string() },
            Message { role: "assistant".to_string(), content: "hi".to_string() },
        ];
        let result = cm.prepare_context(&history, 10000);
        assert_eq!(result.len(), 3); // compressor adds summary system msg
    }

    #[test]
    fn test_prepare_context_compress_then_truncate() {
        let cm = ContextManager::with_budget(50);
        let history: Vec<Message> = (0..20)
            .map(|i| Message { role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(), content: "msg".to_string() })
            .collect();
        let result = cm.prepare_context(&history, 50);
        assert!(result.len() <= 10); // compressed or truncated
    }

    #[test]
    fn test_prepare_context_empty() {
        let cm = ContextManager::new();
        let result = cm.prepare_context(&[], 100);
        assert!(result.is_empty());
    }

    #[test]
    fn test_with_budget_chain() {
        let cm = ContextManager::with_budget(2048);
        let history = vec![
            Message { role: "system".to_string(), content: "You are a bot".to_string() },
            Message { role: "user".to_string(), content: "test".to_string() },
        ];
        let result = cm.prepare_context(&history, 2048);
        assert!(!result.is_empty());
    }

    #[test]
    fn test_prepare_context_reserves_recent() {
        let cm = ContextManager::with_budget(100);
        let history: Vec<Message> = (0..10)
            .map(|i| Message { role: "user".to_string(), content: format!("message {}", i) })
            .collect();
        let result = cm.prepare_context(&history, 100);
        // Should keep at least recent messages after truncation
        assert!(!result.is_empty());
    }

    #[test]
    fn test_context_manager_new_default() {
        let cm = ContextManager::new();
        let history = vec![Message { role: "user".to_string(), content: "hello".to_string() }];
        let result = cm.prepare_context(&history, 4096);
        assert!(!result.is_empty());
    }
}
