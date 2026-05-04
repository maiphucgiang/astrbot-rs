use astrbot_core::pipeline::{
    ContentSafetyCheckStage, PipelineContext, PipelineScheduler, PreProcessStage,
    ProcessStage, RateLimitStage, RespondStage, ResultDecorateStage, SendFn,
    SessionStatusCheckStage, StageRegistry, WakingCheckStage, WhitelistCheckStage,
};
use astrbot_core::platform::MessageSource;
use astrbot_plugin::manager::PluginManager;
use astrbot_provider::client::ProviderManager;
use std::sync::Arc;
use tracing::info;

/// Bot runtime — holds all components and manages lifecycle
pub struct BotRuntime {
    pub provider_manager: ProviderManager,
    pub providers: Vec<Arc<dyn astrbot_core::provider::Provider>>,
    pub plugin_manager: PluginManager,
    pub pipeline: Option<Arc<PipelineScheduler>>,
}

impl BotRuntime {
    pub fn new(plugin_dir: std::path::PathBuf) -> Self {
        Self {
            provider_manager: ProviderManager::new(),
            providers: vec![],
            plugin_manager: PluginManager::new(plugin_dir),
            pipeline: None,
        }
    }

    /// Register an OpenAI-compatible provider from config.
    pub fn register_openai_provider(
        &mut self,
        id: &str,
        api_key: &str,
        base_url: Option<&str>,
        model: &str,
    ) {
        let url = base_url.unwrap_or("https://api.openai.com").to_string();
        let provider = astrbot_provider::openai::OpenAiProvider::new(
            id.to_string(),
            api_key.to_string(),
            url,
            model.to_string(),
        );
        let arc = Arc::new(provider);
        self.providers.push(arc.clone());
        self.provider_manager.register(arc);
        info!("[Runtime] Registered OpenAI provider: {} (model: {})", id, model);
    }

    /// Build the 9-stage pipeline with provider and sender bindings.
    pub async fn build_pipeline(
        &mut self,
        sender: astrbot_core::pipeline::SendFn,
    ) -> anyhow::Result<()> {
        let ctx = Arc::new(PipelineContext::new());
        let mut registry = StageRegistry::new();

        // Bind the first registered provider to ProcessStage
        let process_stage = if let Some(p) = self.providers.first().cloned() {
            ProcessStage::new().with_provider(p)
        } else {
            ProcessStage::new()
        };

        let respond_stage = RespondStage::new().with_sender(sender);

        registry.register("WakingCheckStage", Box::new(WakingCheckStage::default()));
        registry.register("WhitelistCheckStage", Box::new(WhitelistCheckStage::default()));
        registry.register("SessionStatusCheckStage", Box::new(SessionStatusCheckStage::default()));
        registry.register("RateLimitStage", Box::new(RateLimitStage::default()));
        registry.register("ContentSafetyCheckStage", Box::new(ContentSafetyCheckStage::default()));
        registry.register("PreProcessStage", Box::new(PreProcessStage::default()));
        registry.register("ProcessStage", Box::new(process_stage));
        registry.register("ResultDecorateStage", Box::new(ResultDecorateStage::default()));
        registry.register("RespondStage", Box::new(respond_stage));

        registry.initialize_all(&ctx).await?;
        self.pipeline = Some(Arc::new(PipelineScheduler::new(ctx, registry)));
        info!("[Runtime] Pipeline built with 9 stages");
        Ok(())
    }

    /// Graceful shutdown
    pub async fn stop_all(&mut self) -> anyhow::Result<()> {
        info!("[Runtime] Stopping all components...");
        Ok(())
    }
}

impl Default for BotRuntime {
    fn default() -> Self {
        Self::new(std::path::PathBuf::from("plugins"))
    }
}
