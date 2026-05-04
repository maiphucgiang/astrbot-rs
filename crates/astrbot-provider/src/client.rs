use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::provider::{ChatConfig, ChatMessage, ChatResponse, ChatStreamChunk, Provider};
use futures_util::Stream;
use std::sync::Arc;
use tracing::{info, warn};

/// Unified LLM client that wraps a provider
/// Provides a high-level API for chat completions
pub struct LLMClient {
    provider: Box<dyn Provider>,
    default_config: ChatConfig,
}

impl LLMClient {
    /// Create a new LLM client
    pub fn new(provider: Box<dyn Provider>, default_config: Option<ChatConfig>) -> Self {
        Self {
            provider,
            default_config: default_config.unwrap_or_default(),
        }
    }

    /// Set the provider
    pub fn set_provider(&mut self, provider: Box<dyn Provider>) {
        self.provider = provider;
    }

    /// Get the provider name
    pub fn provider_name(&self) -> &str {
        self.provider.name()
    }

    /// Simple chat - send a single user message and get response
    pub async fn chat(
        &self,
        system: Option<String>,
        user: String,
        config: Option<ChatConfig>,
    ) -> Result<ChatResponse> {
        let mut messages = Vec::new();
        if let Some(sys) = system {
            messages.push(ChatMessage::system(sys));
        }
        messages.push(ChatMessage::user(user));

        let merged_config = self.merge_config(config);
        self.provider.chat(messages, merged_config).await
    }

    /// Multi-turn chat - send a conversation and get response
    pub async fn chat_multi(
        &self,
        messages: Vec<ChatMessage>,
        config: Option<ChatConfig>,
    ) -> Result<ChatResponse> {
        let merged_config = self.merge_config(config);
        self.provider.chat(messages, merged_config).await
    }

    /// Streaming chat - send messages and get a stream of response chunks
    pub async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        config: Option<ChatConfig>,
    ) -> Result<Box<dyn Stream<Item = Result<ChatStreamChunk>> + Send>> {
        let mut merged_config = self.merge_config(config);
        merged_config.stream = true;
        self.provider.chat_stream(messages, merged_config).await
    }

    /// Check if the provider is healthy
    pub async fn health_check(&self) -> Result<bool> {
        self.provider.health_check().await
    }

    /// Get supported models
    pub async fn models(&self) -> Result<Vec<String>> {
        self.provider.models().await
    }

    /// Merge user-provided config with default config
    fn merge_config(&self, user_config: Option<ChatConfig>) -> ChatConfig {
        let mut config = self.default_config.clone();
        if let Some(user) = user_config {
            if user.model.is_some() {
                config.model = user.model;
            }
            if user.temperature.is_some() {
                config.temperature = user.temperature;
            }
            if user.max_tokens.is_some() {
                config.max_tokens = user.max_tokens;
            }
            if user.top_p.is_some() {
                config.top_p = user.top_p;
            }
            if user.stream {
                config.stream = user.stream;
            }
            config.extra.extend(user.extra);
        }
        config
    }
}

/// ProviderManager - manages multiple LLM providers with fallback support
pub struct ProviderManager {
    providers: Vec<Arc<dyn Provider>>,
    active_provider: Option<usize>,
    fallback_chain: Vec<usize>,
}

impl ProviderManager {
    /// Create a new provider manager
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            active_provider: None,
            fallback_chain: Vec::new(),
        }
    }

    /// Register a provider
    pub fn register(&mut self, provider: Arc<dyn Provider>) {
        let idx = self.providers.len();
        self.providers.push(provider);
        if self.active_provider.is_none() {
            self.active_provider = Some(idx);
        }
        self.fallback_chain.push(idx);
    }

    /// Set the active provider by index
    pub fn set_active(&mut self, idx: usize) -> Result<()> {
        if idx >= self.providers.len() {
            return Err(AstrBotError::Validation(format!(
                "Invalid provider index: {} (total: {})",
                idx,
                self.providers.len()
            )));
        }
        self.active_provider = Some(idx);
        Ok(())
    }

    /// Set the active provider by ID
    pub fn set_active_by_id(&mut self, id: &str) -> Result<()> {
        for (idx, provider) in self.providers.iter().enumerate() {
            if provider.id() == id {
                self.active_provider = Some(idx);
                return Ok(());
            }
        }
        Err(AstrBotError::NotFound(format!(
            "Provider with id '{}' not found",
            id
        )))
    }

    /// Get the active provider
    pub fn active_provider(&self) -> Option<&dyn Provider> {
        self.active_provider
            .and_then(|idx| self.providers.get(idx))
            .map(|p| p.as_ref())
    }

    /// Get a provider by ID
    pub fn get(&self, id: &str) -> Option<&dyn Provider> {
        self.providers
            .iter()
            .find(|p| p.id() == id)
            .map(|p| p.as_ref())
    }

    /// List all providers
    pub fn list(&self) -> Vec<&dyn Provider> {
        self.providers.iter().map(|p| p.as_ref()).collect()
    }

    /// Remove a provider
    pub fn remove(&mut self, id: &str) {
        self.providers.retain(|p| p.id() != id);
        // Reset active provider if it was removed
        if let Some(active) = self.active_provider {
            if active >= self.providers.len() {
                self.active_provider = self.providers.first().map(|_| 0);
            }
        }
        // Rebuild fallback chain
        self.fallback_chain = (0..self.providers.len()).collect();
    }

    /// Chat with fallback - if active provider fails, try fallback providers
    pub async fn chat_with_fallback(
        &self,
        messages: Vec<ChatMessage>,
        config: ChatConfig,
    ) -> Result<ChatResponse> {
        let chain = self.build_fallback_chain();

        for (attempt, idx) in chain.iter().enumerate() {
            let provider = match self.providers.get(*idx) {
                Some(p) => p,
                None => continue,
            };

            info!(
                "Attempting chat with provider '{}' (attempt {})",
                provider.name(),
                attempt + 1
            );

            match provider.chat(messages.clone(), config.clone()).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    warn!(
                        "Provider '{}' failed: {}. Trying fallback...",
                        provider.name(),
                        e
                    );
                    continue;
                }
            }
        }

        Err(AstrBotError::Provider {
            provider: "all".to_string(),
            message: "All providers in fallback chain failed".to_string(),
        })
    }

    /// Health check all providers
    pub async fn health_check_all(&self) -> Vec<(String, bool)> {
        let mut results = Vec::new();
        for provider in &self.providers {
            let name = provider.name().to_string();
            let healthy = provider.health_check().await.unwrap_or(false);
            results.push((name, healthy));
        }
        results
    }

    fn build_fallback_chain(&self) -> Vec<usize> {
        let mut chain = Vec::new();
        // Active provider first
        if let Some(active) = self.active_provider {
            chain.push(active);
        }
        // Then the rest in fallback chain order
        for idx in &self.fallback_chain {
            if !chain.contains(idx) {
                chain.push(*idx);
            }
        }
        chain
    }
}

impl Default for ProviderManager {
    fn default() -> Self {
        Self::new()
    }
}
