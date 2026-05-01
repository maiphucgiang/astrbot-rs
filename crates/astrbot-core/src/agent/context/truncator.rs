use crate::agent::context::token_counter::TokenCounter;
use crate::provider::ChatMessage as Message;

pub struct ContextTruncator {
    token_counter: TokenCounter,
    max_tokens: usize,
    preserve_recent: usize,
}

impl ContextTruncator {
    pub fn new(max_tokens: usize) -> Self {
        Self {
            token_counter: TokenCounter::new(),
            max_tokens,
            preserve_recent: 1,
        }
    }

    pub fn with_preserve(max_tokens: usize, preserve_recent: usize) -> Self {
        Self {
            token_counter: TokenCounter::new(),
            max_tokens,
            preserve_recent: preserve_recent.max(1),
        }
    }

    pub fn truncate(&self, history: &[Message]) -> Vec<Message> {
        let mut result = history.to_vec();
        let mut current = self.token_counter.count_history(&result);
        let min_keep = self.preserve_recent.min(result.len());
        while current > self.max_tokens && result.len() > min_keep {
            let removed = result.remove(0);
            current -= self.token_counter.count_message(&removed);
        }
        result
    }

    pub fn fits(&self, history: &[Message]) -> bool {
        self.token_counter.count_history(history) <= self.max_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> Message {
        Message {
            role: role.to_string(),
            content: content.to_string(),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[test]
    fn no_truncation_when_under_budget() {
        let truncator = ContextTruncator::new(1000);
        let history = vec![msg("user", "hello"), msg("assistant", "world")];
        let out = truncator.truncate(&history);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].content, "hello");
    }

    #[test]
    fn drops_oldest_first() {
        let truncator = ContextTruncator::new(20);
        let history = vec![
            msg("user", "first message here"),
            msg("assistant", "second message here"),
            msg("user", "third message here"),
        ];
        let out = truncator.truncate(&history);
        assert!(!out.is_empty());
        assert_ne!(out.first().map(|m| m.content.as_str()), Some("first message here"));
    }

    #[test]
    fn preserves_recent_floor() {
        let truncator = ContextTruncator::with_preserve(5, 2);
        let history = vec![
            msg("user", "a very long first message with many tokens"),
            msg("assistant", "a very long second message with many tokens"),
            msg("user", "a very long third message with many tokens"),
        ];
        let out = truncator.truncate(&history);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].role, "assistant");
        assert_eq!(out[1].role, "user");
    }

    #[test]
    fn empty_history() {
        let truncator = ContextTruncator::new(10);
        let out = truncator.truncate(&[]);
        assert!(out.is_empty());
    }

    #[test]
    fn single_message_exceeds_budget() {
        let truncator = ContextTruncator::new(1);
        let history = vec![msg("user", "way too long for one token")];
        let out = truncator.truncate(&history);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn fits_method() {
        let truncator = ContextTruncator::new(1000);
        let small = vec![msg("user", "hi")];
        let big = vec![msg("user", "this is an enormously long message with lots of characters that surely exceeds any tiny budget we might set later")];
        assert!(truncator.fits(&small));
        assert!(truncator.fits(&big));
        let tiny = ContextTruncator::new(1);
        assert!(!tiny.fits(&big));
    }
}
