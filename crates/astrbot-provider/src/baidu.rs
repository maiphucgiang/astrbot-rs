//! Baidu Qianfan (ERNIE) provider — OpenAI-compatible API
//!
//! Docs: https://cloud.baidu.com/doc/WENXINWORKSHOP/s/ilkkdek78

use crate::openai::OpenAiProvider;
use astrbot_core::errors::Result;
use astrbot_core::provider::{
    ChatConfig, ChatMessage, ChatResponse, ChatStreamChunk, ModelInfo, Provider,
};
use async_trait::async_trait;
use futures_util::Stream;

/// Baidu Qianfan (ERNIE) provider wrapper
pub struct BaiduProvider {
    inner: OpenAiProvider,
}

impl BaiduProvider {
    pub fn new(id: String, api_key: String, model: String) -> Self {
        let base_url = "https://qianfan.baidubce.com/compatible-mode/v1".to_string();
        Self {
            inner: OpenAiProvider::new(id, api_key, base_url, model),
        }
    }
}

#[async_trait]
impl Provider for BaiduProvider {
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
    use astrbot_core::provider::{ChatConfig, ChatMessage};

    #[tokio::test]
    async fn test_baidu_provider_creation() {
        let provider = BaiduProvider::new(
            "baidu".to_string(),
            "test-key".to_string(),
            "ernie-4.0-turbo-8k".to_string(),
        );
        assert_eq!(provider.id(), "baidu");
        assert_eq!(provider.name(), "baidu");
    }

    #[tokio::test]
    async fn test_baidu_provider_models() {
        let provider = BaiduProvider::new(
            "baidu".to_string(),
            "test-key".to_string(),
            "ernie-4.0-turbo-8k".to_string(),
        );
        let models = provider.models().await.unwrap();
        assert_eq!(models, vec!["ernie-4.0-turbo-8k"]);
    }

    #[tokio::test]
    async fn test_baidu_provider_chat_test_key() {
        let provider = BaiduProvider::new(
            "baidu".to_string(),
            "test-key".to_string(),
            "ernie-4.0-turbo-8k".to_string(),
        );
        let messages = vec![
            ChatMessage::system("You are a helpful assistant."),
            ChatMessage::user("Hello"),
        ];
        let response = provider
            .chat(messages, ChatConfig::default())
            .await
            .unwrap();
        assert!(response.content.contains("Mock") || !response.content.is_empty());
    }

    #[tokio::test]
    async fn test_baidu_provider_health_check() {
        let provider = BaiduProvider::new(
            "baidu".to_string(),
            "test-key".to_string(),
            "ernie-4.0-turbo-8k".to_string(),
        );
        assert!(provider.health_check().await.unwrap());
    }
}
