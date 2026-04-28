use astrbot_core::errors::Result;
use astrbot_core::message::MessageChain;
use astrbot_core::pipeline::ReplySender;
use astrbot_core::platform::MessageSource;
use astrbot_provider::openai::OpenAiProvider;
use astrbot_provider::client::ProviderManager;
use std::sync::Arc;
use tracing::info;

/// Bot runtime — holds all components and manages lifecycle
pub struct BotRuntime {
    pub provider_manager: ProviderManager,
}

impl BotRuntime {
    pub fn new() -> Self {
        Self {
            provider_manager: ProviderManager::new(),
        }
    }

    /// Register an OpenAI-compatible provider from config
    pub fn register_openai_provider(
        &mut self,
        id: &str,
        api_key: &str,
        base_url: Option<&str>,
        model: &str,
    ) {
        let url = base_url.unwrap_or("https://api.openai.com").to_string();
        let provider = OpenAiProvider::new(
            id.to_string(),
            api_key.to_string(),
            url,
            model.to_string(),
        );
        self.provider_manager.register(Box::new(provider));
        info!("[Runtime] Registered OpenAI provider: {} (model: {})", id, model);
    }

    /// Graceful shutdown
    pub async fn stop_all(&mut self) -> Result<()> {
        info!("[Runtime] Stopping all components...");
        Ok(())
    }
}

impl Default for BotRuntime {
    fn default() -> Self {
        Self::new()
    }
}
