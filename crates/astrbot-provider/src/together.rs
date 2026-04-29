//! Together AI provider — OpenAI-compatible API
//!
//! Together AI offers fast inference for open-source models.
//! Docs: https://docs.together.ai/docs/openai-api-compatibility

use crate::openai::OpenAiProvider;
use astrbot_core::errors::Result;
use astrbot_core::provider::{
    ChatConfig, ChatMessage, ChatResponse, ChatStreamChunk, ModelInfo, Provider,
};
use async_trait::async_trait;
use futures_util::Stream;

/// Together AI provider wrapper
pub struct TogetherProvider {
    inner: OpenAiProvider,
}

impl TogetherProvider {
    pub fn new(id: String, api_key: String, model: String) -> Self {
        let base_url = "https://api.together.xyz/v1".to_string();
        Self {
            inner: OpenAiProvider::new(id, api_key, base_url, model),
        }
    }
}

#[async_trait]
impl Provider for TogetherProvider {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn models(&self) -> Result<Vec<String>> {
        self.inner.models().await
    }

    async fn chat(&self, messages: Vec<ChatMessage>, config: ChatConfig) -> Result<ChatResponse> {
        self.inner.chat(messages, config).await
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        config: ChatConfig,
    ) -> Result<Box<dyn Stream<Item = Result<ChatStreamChunk>> + Send>> {
        self.inner.chat_stream(messages, config).await
    }

    async fn embedding(&self, texts: Vec<String>, model: Option<String>) -> Result<Vec<Vec<f32>>> {
        self.inner.embedding(texts, model).await
    }

    async fn model_info(&self, model: &str) -> Result<ModelInfo> {
        self.inner.model_info(model).await
    }

    async fn health_check(&self) -> Result<bool> {
        self.inner.health_check().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_together_provider_creation() {
        let p = TogetherProvider::new(
            "together-1".to_string(),
            "sk-together-test".to_string(),
            "meta-llama/Llama-3.1-70B-Instruct-Turbo".to_string(),
        );
        assert_eq!(p.id(), "together-1");
        assert_eq!(p.name(), "together-1");
    }
}
