#[cfg(test)]
mod tests {
    use crate::client::{LLMClient, ProviderManager};
    use astrbot_core::errors::Result;
    use astrbot_core::provider::{
        ChatConfig, ChatMessage, ChatResponse, Provider, ProviderConfig,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Mock provider for testing
    struct MockProvider {
        id: String,
        name: String,
        chat_count: AtomicUsize,
        should_fail: bool,
    }

    impl MockProvider {
        fn new(id: &str, name: &str, should_fail: bool) -> Self {
            Self {
                id: id.to_string(),
                name: name.to_string(),
                chat_count: AtomicUsize::new(0),
                should_fail,
            }
        }
    }

    #[async_trait::async_trait]
    impl Provider for MockProvider {
        fn id(&self) -> &str {
            &self.id
        }

        fn name(&self) -> &str {
            &self.name
        }

        async fn models(&self) -> Result<Vec<String>> {
            Ok(vec!["mock-model".to_string()])
        }

        async fn chat(&self, _messages: Vec<ChatMessage>, _config: ChatConfig) -> Result<ChatResponse> {
            self.chat_count.fetch_add(1, Ordering::Relaxed);
            if self.should_fail {
                return Err(astrbot_core::errors::AstrBotError::Provider {
                    provider: self.name.clone(),
                    message: "Mock failure".to_string(),
                });
            }
            Ok(ChatResponse {
                content: "Mock response".to_string(),
                model: "mock-model".to_string(),
                usage: None,
                reasoning: None,
            })
        }

        async fn chat_stream(
            &self,
            _messages: Vec<ChatMessage>,
            _config: ChatConfig,
        ) -> Result<Box<dyn futures_util::Stream<Item = Result<astrbot_core::provider::ChatStreamChunk>> + Send>> {
            Ok(Box::new(futures_util::stream::empty()))
        }

        async fn embedding(&self, _texts: Vec<String>, _model: Option<String>) -> Result<Vec<Vec<f32>>> {
            Ok(vec![vec![0.1, 0.2, 0.3]])
        }

        async fn model_info(&self, _model: &str) -> Result<astrbot_core::provider::ModelInfo> {
            Ok(astrbot_core::provider::ModelInfo {
                name: "mock-model".to_string(),
                context_length: 4096,
                supports_streaming: true,
                supports_vision: false,
                supports_function_calling: false,
            })
        }

        async fn health_check(&self) -> Result<bool> {
            Ok(!self.should_fail)
        }
    }

    #[tokio::test]
    async fn test_llm_client_chat() {
        let provider = MockProvider::new("mock1", "Mock Provider", false);
        let client = LLMClient::new(Box::new(provider), None);

        let response = client.chat(
            Some("You are a test".to_string()),
            "Hello".to_string(),
            None,
        ).await.unwrap();

        assert_eq!(response.content, "Mock response");
    }

    #[tokio::test]
    async fn test_llm_client_health_check() {
        let provider = MockProvider::new("mock1", "Mock Provider", false);
        let client = LLMClient::new(Box::new(provider), None);

        assert!(client.health_check().await.unwrap());
    }

    #[tokio::test]
    async fn test_provider_manager_register() {
        let mut manager = ProviderManager::new();
        let provider = MockProvider::new("mock1", "Mock Provider", false);

        manager.register(Box::new(provider));
        assert_eq!(manager.list().len(), 1);
        assert_eq!(manager.active_provider().unwrap().name(), "Mock Provider");
    }

    #[tokio::test]
    async fn test_provider_manager_fallback() {
        let mut manager = ProviderManager::new();
        
        // Register a failing provider first
        let failing = MockProvider::new("fail", "Failing Provider", true);
        manager.register(Box::new(failing));
        
        // Register a working provider
        let working = MockProvider::new("ok", "Working Provider", false);
        manager.register(Box::new(working));

        // Set failing as active
        manager.set_active(0).unwrap();

        let messages = vec![ChatMessage::user("Hello")];
        let config = ChatConfig::default();

        // Should fallback to working provider
        let response = manager.chat_with_fallback(messages, config).await.unwrap();
        assert_eq!(response.content, "Mock response");
    }

    #[tokio::test]
    async fn test_provider_manager_set_active_by_id() {
        let mut manager = ProviderManager::new();
        let provider = MockProvider::new("mock1", "Mock Provider", false);
        
        manager.register(Box::new(provider));
        manager.set_active_by_id("mock1").unwrap();
        
        assert_eq!(manager.active_provider().unwrap().id(), "mock1");
    }

    #[tokio::test]
    async fn test_provider_manager_health_check_all() {
        let mut manager = ProviderManager::new();
        
        let healthy = MockProvider::new("h1", "Healthy", false);
        let unhealthy = MockProvider::new("h2", "Unhealthy", true);
        
        manager.register(Box::new(healthy));
        manager.register(Box::new(unhealthy));

        let results = manager.health_check_all().await;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], ("Healthy".to_string(), true));
        assert_eq!(results[1], ("Unhealthy".to_string(), false));
    }

    #[tokio::test]
    async fn test_provider_manager_remove() {
        let mut manager = ProviderManager::new();
        let provider = MockProvider::new("mock1", "Mock Provider", false);
        
        manager.register(Box::new(provider));
        assert_eq!(manager.list().len(), 1);
        
        manager.remove("mock1");
        assert_eq!(manager.list().len(), 0);
        assert!(manager.active_provider().is_none());
    }

    #[tokio::test]
    async fn test_provider_config_deserialization() {
        let config_json = serde_json::json!({
            "id": "openai",
            "name": "OpenAI",
            "provider_type": "openai",
            "enabled": true,
            "api_key": "sk-test",
            "base_url": "https://api.openai.com",
            "default_model": "gpt-4",
            "models": ["gpt-4", "gpt-3.5-turbo"],
            "extra": {
                "organization": "test-org"
            }
        });

        let config: ProviderConfig = serde_json::from_value(config_json).unwrap();
        assert_eq!(config.id, "openai");
        assert_eq!(config.name, "OpenAI");
        assert_eq!(config.default_model, "gpt-4");
        assert_eq!(config.models.len(), 2);
        assert!(config.extra.contains_key("organization"));
    }

    #[tokio::test]
    async fn test_mock_provider_health_check() {
        let mock = MockProvider::new("mock", "Mock", false);
        assert!(mock.health_check().await.unwrap());
    }

    #[tokio::test]
    async fn test_mock_provider_chat() {
        let mock = MockProvider::new("mock", "Mock", false);
        let response = mock.chat(
            vec![ChatMessage::user("Hi")],
            ChatConfig::default(),
        ).await.unwrap();
        assert_eq!(response.content, "Mock response");
    }

    #[tokio::test]
    async fn test_mock_provider_models() {
        let mock = MockProvider::new("mock", "Mock", false);
        let models = mock.models().await.unwrap();
        assert!(!models.is_empty());
    }
}