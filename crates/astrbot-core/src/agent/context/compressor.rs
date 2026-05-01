use crate::provider::ChatMessage as Message;

pub struct ContextCompressor {
    max_full_messages: usize,
}

impl Default for ContextCompressor {
    fn default() -> Self {
        Self { max_full_messages: 4 }
    }
}

impl ContextCompressor {
    pub fn new(max_full_messages: usize) -> Self {
        Self { max_full_messages }
    }

    pub fn compress(&self, history: &mut Vec<Message>) {
        if history.len() <= self.max_full_messages {
            return;
        }
        let to_compress = history.len() - self.max_full_messages;
        let old: Vec<Message> = history.drain(..to_compress).collect();
        let roles: Vec<&str> = old.iter().map(|m| m.role.as_str()).collect();
        let unique_roles: Vec<&str> = {
            let mut seen = std::collections::HashSet::new();
            roles.into_iter().filter(|r| seen.insert(*r)).collect()
        };
        let summary = format!("[{} earlier messages summarized. Roles: {}]", old.len(), unique_roles.join(", "));
        let summary_msg = Message {
            role: "system".to_string(),
            content: summary,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };
        history.insert(0, summary_msg);
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
    fn no_compress_when_under_limit() {
        let compressor = ContextCompressor::new(4);
        let mut history = vec![msg("user", "hello"), msg("assistant", "hi"), msg("user", "how are you")];
        compressor.compress(&mut history);
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].role, "user");
    }

    #[test]
    fn compresses_oldest_messages() {
        let compressor = ContextCompressor::new(2);
        let mut history = vec![msg("user", "q1"), msg("assistant", "a1"), msg("user", "q2"), msg("assistant", "a2"), msg("user", "q3")];
        compressor.compress(&mut history);
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].role, "system");
        assert!(history[0].content.contains("3 earlier messages summarized"));
        assert!(history[0].content.contains("user, assistant"));
        assert_eq!(history[1].content, "a2");
        assert_eq!(history[2].content, "q3");
    }

    #[test]
    fn empty_history_does_not_panic() {
        let compressor = ContextCompressor::new(4);
        let mut history: Vec<Message> = vec![];
        compressor.compress(&mut history);
        assert!(history.is_empty());
    }

    #[test]
    fn summary_contains_unique_roles_only() {
        let compressor = ContextCompressor::new(1);
        let mut history = vec![msg("user", "u1"), msg("user", "u2"), msg("assistant", "a1"), msg("user", "u3")];
        compressor.compress(&mut history);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, "system");
        assert!(history[0].content.contains("user, assistant"));
        assert!(!history[0].content.contains("user, assistant, user"));
    }

    #[test]
    fn exact_limit_boundary() {
        let compressor = ContextCompressor::new(3);
        let mut history = vec![msg("user", "a"), msg("assistant", "b"), msg("user", "c")];
        compressor.compress(&mut history);
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].content, "a");
    }
}
