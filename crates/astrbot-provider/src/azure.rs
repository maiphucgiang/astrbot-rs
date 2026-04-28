//! Azure OpenAI provider — OpenAI-compatible API
//!
//! Azure OpenAI Service provides OpenAI models via Azure infrastructure.
//! Docs: https://learn.microsoft.com/en-us/azure/ai-services/openai/

use async_trait::async_trait;
use crate::openai::OpenAiProvider;
use astrbot_core::provider::{Provider, ChatMessage, ChatConfig, ChatResponse, ChatStreamChunk, ModelInfo};
use astrbot_core::errors::Result;
use futures_util::Stream;

/// Azure OpenAI provider wrapper
pub struct AzureOpenAiProvider {
    inner: OpenAiProvider,
}

impl AzureOpenAiProvider {
    pub fn new(id: String, api_key: String, endpoint: String, deployment: String) -> Self {
        // Azure OpenAI endpoint: https://{your-resource}.openai.azure.com/openai/deployments/{deployment}
        let base_url = format!("{}/openai/deployments/{}", endpoint.trim_end_matches('/'), deployment);
        Self {
            inner: OpenAiProvider::new(id, api_key, base_url, deployment),
        }
    }
}

#[async_trait]
impl Provider for AzureOpenAiProvider {
    fn id(&self) -> &str { self.inner.id() }
    fn name(&self) -> &str { self.inner.name() }

    async fn models(&self) -> Result<Vec<String>> { self.inner.models().await }

    async fn chat(&self, messages: Vec<ChatMessage>, config: ChatConfig) -> Result<ChatResponse> {
        self.inner.chat(messages, config).await
    }

    async fn chat_stream(&self, messages: Vec<ChatMessage>, config: ChatConfig,
    ) -> Result<Box<dyn Stream<Item = Result<ChatStreamChunk>> + Send>> {
        self.inner.chat_stream(messages, config).await
    }

    async fn embedding(&self, texts: Vec<String>, model: Option<String>) -> Result<Vec<Vec<f32>>> {
        self.inner.embedding(texts, model).await
    }

    async fn model_info(&self, model: &str) -> Result<ModelInfo> { self.inner.model_info(model).await }

    async fn health_check(&self) -> Result<bool> { self.inner.health_check().await }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_azure_provider_creation() {
        let p = AzureOpenAiProvider::new(
            "azure-1".to_string(),
            "sk-azure-test".to_string(),
            "https://my-resource.openai.azure.com".to_string(),
            "gpt-4o".to_string(),
        );
        assert_eq!(p.id(), "azure-1");
        assert_eq!(p.name(), "azure-1");
    }
}
