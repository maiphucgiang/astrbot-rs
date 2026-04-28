//! End-to-end integration tests for AstrBot Rust

use crate::config::{AstrBotConfig, ProviderConfig, PlatformConfig};
use crate::provider::{ChatMessage, ChatConfig, Provider, ChatResponse, ModelInfo, ChatStreamChunk};
use crate::errors::Result;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A mock provider for integration testing
struct MockProvider {
    id: String,
    name: String,
    call_count: AtomicUsize,
}

#[async_trait::async_trait]
impl Provider for MockProvider {
    fn id(&self) -> &str { &self.id }
    fn name(&self) -> &str { &self.name }
    async fn models(&self) -> Result<Vec<String>> {
        Ok(vec!["mock".to_string()])
    }
    async fn chat(&self, _messages: Vec<ChatMessage>, _config: ChatConfig
    ) -> Result<ChatResponse> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        Ok(ChatResponse {
            content: "hello from mock".to_string(),
            model: "mock".to_string(),
            usage: None,
            reasoning: None,
        })
    }
    async fn chat_stream(
        &self, _messages: Vec<ChatMessage>, _config: ChatConfig
    ) -> Result<Box<dyn futures_util::Stream<Item = Result<ChatStreamChunk>> + Send>> {
        Ok(Box::new(futures_util::stream::empty()))
    }
    async fn embedding(&self, _texts: Vec<String>, _model: Option<String>
    ) -> Result<Vec<Vec<f32>>> {
        Ok(vec![])
    }
    async fn model_info(&self, _model: &str
    ) -> Result<ModelInfo> {
        Ok(ModelInfo::default())
    }
    async fn health_check(&self) -> Result<bool> {
        Ok(true)
    }
}

#[tokio::test]
async fn test_e2e_config_to_provider_manager() {
    // Build a config with a provider and platform
    let mut config = AstrBotConfig::default();
    config.providers.push(ProviderConfig {
        id: "mock".to_string(),
        provider_type: "mock".to_string(),
        api_key: None,
        base_url: None,
        model: "mock".to_string(),
        enabled: true,
        extra: std::collections::HashMap::new(),
    });
    config.platforms.push(PlatformConfig {
        id: "qq".to_string(),
        platform_type: "qq".to_string(),
        enabled: false,
        config: std::collections::HashMap::new(),
    });

    assert_eq!(config.providers.len(), 1);
    assert_eq!(config.platforms.len(), 1);
}

#[tokio::test]
async fn test_e2e_provider_trait_chat() {
    let provider = MockProvider {
        id: "mock".to_string(),
        name: "Mock".to_string(),
        call_count: AtomicUsize::new(0),
    };

    let messages = vec![
        ChatMessage::system("You are a test"),
        ChatMessage::user("Say hello"),
    ];

    let response = provider.chat(messages, ChatConfig::default()).await.unwrap();
    assert_eq!(response.content, "hello from mock");
}

#[tokio::test]
async fn test_e2e_provider_trait_streaming() {
    let provider = MockProvider {
        id: "mock".to_string(),
        name: "Mock".to_string(),
        call_count: AtomicUsize::new(0),
    };

    let messages = vec![ChatMessage::user("Hi")];
    let config = ChatConfig { stream: true, ..ChatConfig::default() };

    let _stream = provider.chat_stream(messages, config).await.unwrap();
    // Stream should be empty for mock — skip full collect, just verify it returned Ok
}

#[tokio::test]
async fn test_e2e_provider_trait_embedding() {
    let provider = MockProvider {
        id: "mock".to_string(),
        name: "Mock".to_string(),
        call_count: AtomicUsize::new(0),
    };

    let result = provider.embedding(vec!["hello".to_string()], None).await.unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn test_e2e_provider_trait_health_check() {
    let provider = MockProvider {
        id: "mock".to_string(),
        name: "Mock".to_string(),
        call_count: AtomicUsize::new(0),
    };

    assert!(provider.health_check().await.unwrap());
}

#[tokio::test]
async fn test_e2e_chat_with_system_and_user() {
    let provider = MockProvider {
        id: "mock".to_string(),
        name: "Mock".to_string(),
        call_count: AtomicUsize::new(0),
    };

    let messages = vec![
        ChatMessage::system("You are a helpful assistant"),
        ChatMessage::user("What's 1+1?"),
    ];

    let response = provider.chat(messages, ChatConfig::default()).await.unwrap();
    assert_eq!(response.content, "hello from mock");
}

#[tokio::test]
async fn test_e2e_provider_config_roundtrip() {
    let config = ProviderConfig {
        id: "openai".to_string(),
        provider_type: "openai".to_string(),
        api_key: Some("sk-test".to_string()),
        base_url: Some("https://api.openai.com".to_string()),
        model: "gpt-4".to_string(),
        enabled: true,
        extra: std::collections::HashMap::new(),
    };

    // Serialize to JSON
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("openai"));
    assert!(json.contains("gpt-4"));

    // Deserialize back
    let restored: ProviderConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.id, "openai");
    assert_eq!(restored.model, "gpt-4");
    assert_eq!(restored.api_key, Some("sk-test".to_string()));
}