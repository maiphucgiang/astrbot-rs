use astrbot_core::errors::Result;
use astrbot_core::message::MessageChain;
use astrbot_core::pipeline::ReplySender;
use astrbot_core::platform::MessageSource;
use astrbot_platform::{qq::QQAdapter, telegram::TelegramAdapter, PlatformAdapter};
use astrbot_provider::openai::OpenAiProvider;
use astrbot_provider::client::ProviderManager;
use std::sync::Arc;
use tracing::info;

/// AdapterReplySender — bridges Pipeline replies back to platform adapters
pub struct AdapterReplySender {
    adapter: Arc<dyn PlatformAdapter>,
}

impl AdapterReplySender {
    pub fn new(adapter: Arc<dyn PlatformAdapter>) -> Self {
        Self { adapter }
    }
}

#[async_trait::async_trait]
impl ReplySender for AdapterReplySender {
    async fn send_reply(
        &self,
        source: &MessageSource,
        chain: &MessageChain,
    ) -> Result<()> {
        self.adapter.send_message(source, chain).await
    }
}

/// Bot runtime — holds all components and manages lifecycle
pub struct BotRuntime {
    pub provider_manager: ProviderManager,
    pub adapters: Vec<Arc<dyn PlatformAdapter>>,
}

impl BotRuntime {
    pub fn new() -> Self {
        Self {
            provider_manager: ProviderManager::new(),
            adapters: Vec::new(),
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

    /// Create, initialize, start, and register a QQ adapter
    pub async fn add_qq_adapter(
        &mut self,
        handler: Arc<dyn astrbot_core::message::MessageHandler>,
        ws_host: &str,
        ws_port: u16,
        http_url: &str,
        access_token: Option<&str>,
    ) -> Result<Arc<dyn PlatformAdapter>> {
        let mut adapter = QQAdapter::new(
            ws_host.to_string(),
            ws_port,
            http_url.to_string(),
            access_token.map(|s| s.to_string()),
        );
        adapter.initialize().await?;
        adapter.set_message_handler(handler);
        adapter.start().await?;
        let adapter = Arc::new(adapter);
        self.adapters.push(adapter.clone());
        info!("[Runtime] QQ adapter started on {}:{}", ws_host, ws_port);
        Ok(adapter)
    }

    /// Create, initialize, start, and register a Telegram adapter
    pub async fn add_telegram_adapter(
        &mut self,
        handler: Arc<dyn astrbot_core::message::MessageHandler>,
        bot_token: &str,
        api_base: Option<&str>,
    ) -> Result<Arc<dyn PlatformAdapter>> {
        let mut adapter = TelegramAdapter::new(
            bot_token.to_string(),
            api_base.map(|s| s.to_string()),
            None,
        );
        adapter.initialize().await?;
        adapter.set_message_handler(handler);
        adapter.start().await?;
        let adapter = Arc::new(adapter);
        self.adapters.push(adapter.clone());
        info!("[Runtime] Telegram adapter started (token: {}...)", &bot_token[..bot_token.len().min(8)]);
        Ok(adapter)
    }

    /// Stop all adapters
    pub async fn stop_all(&mut self) -> Result<()> {
        for adapter in &mut self.adapters {
            info!("[Runtime] Stopping adapter: {}", adapter.metadata().name);
        }
        Ok(())
    }

    /// Health check all components
    pub async fn health_check(&self) -> Vec<(String, bool)> {
        let mut results = self.provider_manager.health_check_all().await;
        for adapter in &self.adapters {
            let name = adapter.metadata().name.clone();
            let healthy = adapter.health_check().await.unwrap_or(false);
            results.push((format!("adapter:{}", name), healthy));
        }
        results
    }
}

impl Default for BotRuntime {
    fn default() -> Self {
        Self::new()
    }
}