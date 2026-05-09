use crate::provider::ChatMessage as Message;

pub struct TokenCounter {
    cjk_chars_per_token: f32,
    ascii_chars_per_token: f32,
}

impl Default for TokenCounter {
    fn default() -> Self {
        Self {
            cjk_chars_per_token: 1.0,
            ascii_chars_per_token: 4.0,
        }
    }
}

impl TokenCounter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_ratios(cjk: f32, ascii: f32) -> Self {
        Self {
            cjk_chars_per_token: cjk,
            ascii_chars_per_token: ascii,
        }
    }

    pub fn count_text(&self, text: &str) -> usize {
        let mut cjk_count = 0usize;
        let mut ascii_count = 0usize;
        for ch in text.chars() {
            if is_cjk(ch) {
                cjk_count += 1;
            } else if ch.is_ascii_alphanumeric() {
                ascii_count += 1;
            }
        }
        let cjk_tokens = (cjk_count as f32 / self.cjk_chars_per_token).ceil() as usize;
        let ascii_tokens = (ascii_count as f32 / self.ascii_chars_per_token).ceil() as usize;
        let overhead = if text.is_empty() { 0 } else { 4 };
        cjk_tokens + ascii_tokens + overhead
    }

    pub fn count_message(&self, msg: &Message) -> usize {
        let role_tokens = self.count_text(&msg.role);
        let content_tokens = self.count_text(&msg.content);
        role_tokens + content_tokens + 3
    }

    pub fn count_history(&self, history: &[Message]) -> usize {
        history.iter().map(|m| self.count_message(m)).sum()
    }
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch,
        '\u{4e00}'..='\u{9fff}'
        | '\u{3400}'..='\u{4dbf}'
        | '\u{3040}'..='\u{309f}'
        | '\u{30a0}'..='\u{30ff}'
        | '\u{ac00}'..='\u{d7af}'
        | '\u{ff00}'..='\u{ffef}'
    )
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
    fn count_text_ascii() {
        let counter = TokenCounter::new();
        assert_eq!(counter.count_text("hello world"), 7);
    }

    #[test]
    fn count_text_cjk() {
        let counter = TokenCounter::new();
        assert_eq!(counter.count_text("你好世界"), 8);
    }

    #[test]
    fn count_mixed_text() {
        let counter = TokenCounter::new();
        assert_eq!(counter.count_text("hello 你好 world 世界"), 11);
    }

    #[test]
    fn count_empty_text() {
        let counter = TokenCounter::new();
        assert_eq!(counter.count_text(""), 0);
    }

    #[test]
    fn count_history() {
        let counter = TokenCounter::new();
        let history = vec![msg("user", "hello"), msg("assistant", "你好")];
        assert_eq!(counter.count_history(&history), 30);
    }

    #[test]
    fn custom_ratios() {
        let counter = TokenCounter::with_ratios(2.0, 8.0);
        assert_eq!(counter.count_text("hello"), 5);
        assert_eq!(counter.count_text("你好"), 5);
    }
}
