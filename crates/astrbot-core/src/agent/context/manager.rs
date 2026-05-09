use crate::agent::context::compressor::ContextCompressor;
use crate::agent::context::token_counter::TokenCounter;
use crate::agent::context::truncator::ContextTruncator;
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
            truncator: ContextTruncator::new(4096),
        }
    }
}

impl ContextManager {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_budget(max_tokens: usize) -> Self {
        Self {
            counter: TokenCounter::new(),
            compressor: ContextCompressor::new(4),
            truncator: ContextTruncator::new(max_tokens),
        }
    }
    pub fn prepare_context(&self, history: &[Message], max_tokens: usize) -> Vec<Message> {
        let mut working = history.to_vec();
        self.compressor.compress(&mut working);
        let truncator = ContextTruncator::new(max_tokens);
        truncator.truncate(&working)
    }
}
