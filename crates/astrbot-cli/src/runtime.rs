use astrbot_core::pipeline::{
    ContentSafetyCheckStage, PipelineContext, PipelineScheduler, PreProcessStage,
    ProcessStage, RateLimitStage, RespondStage, ResultDecorateStage, SendFn,
    SessionStatusCheckStage, StageRegistry, WakingCheckStage, WhitelistCheckStage,
};
use astrbot_core::platform::MessageSource;
use astrbot_core::message::{AstrBotMessage, MessageChain, MessageMember, MessageType};
use astrbot_core::platform::PlatformType;
use astrbot_plugin::manager::PluginManager;
use astrbot_provider::client::ProviderManager;
use std::sync::Arc;
use tracing::{error, info, warn};
use tokio::io::AsyncBufReadExt;

/// ConsoleSender — prints AI replies to stdout
pub struct ConsoleSender;

impl ConsoleSender {
    pub fn as_send_fn() -> SendFn {
        Arc::new(|_src: MessageSource, chain: MessageChain| {
            Box::pin(async move {
                println!("[AI] {}", chain.plain_text());
                Ok(())
            })
        })
    }
}

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

    /// Register an OpenAI-compatible provider from config
    pub fn register_openai_provider(
        &mut self,
        id: &str,
        api_key: &str,
        url: &str,
        model: &str,
    ) {
        let provider = astrbot_provider::openai::OpenAiProvider::new(
            id.to_string(),
            api_key.to_string(),
            url.to_string(),
            model.to_string(),
        );
        let arc = Arc::new(provider);
        self.providers.push(arc.clone());
        self.provider_manager.register(arc);
        info!("[Runtime] Registered OpenAI provider: {} (model: {})", id, model);
    }

    /// Build the 9-stage pipeline with provider and sender bindings
    pub async fn build_pipeline(&mut self, sender: SendFn) -> anyhow::Result<()> {
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

    /// Start the bot — build pipeline, then run interactive console loop
    pub async fn start(&mut self) -> anyhow::Result<()> {
        let sender = ConsoleSender::as_send_fn();
        self.build_pipeline(sender).await?;
        let pipeline = self.pipeline.clone().unwrap();

        info!("[Runtime] Bot started. Type messages and press Enter. Ctrl+C to exit.");

        let stdin = tokio::io::stdin();
        let mut reader = tokio::io::BufReader::new(stdin);
        let mut line = String::new();

        loop {
            line.clear();
            tokio::select! {
                result = reader.read_line(&mut line) => {
                    match result {
                        Ok(0) => {
                            info!("[Runtime] EOF received, shutting down...");
                            break;
                        }
                        Ok(_) => {
                            let trimmed = line.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            let message = AstrBotMessage {
                                message_id: format!("console-{}", chrono::Utc::now().timestamp_millis()),
                                timestamp: chrono::Utc::now(),
                                platform: PlatformType::Custom,
                                session_id: "console-session".to_string(),
                                sender: MessageMember {
                                    user_id: "console-user".to_string(),
                                    nickname: Some("User".to_string()),
                                    card: None,
                                    role: None,
                                    is_self: false,
                                },
                                message_type: MessageType::Private,
                                chain: MessageChain::new().text(trimmed.to_string()),
                                raw_payload: None,
                            };
                            let mut event = astrbot_core::pipeline::PipelineEvent::new(message);
                            if let Err(e) = pipeline.execute(&mut event).await {
                                error!("[Runtime] Pipeline error: {}", e);
                            }
                        }
                        Err(e) => {
                            error!("[Runtime] stdin read error: {}", e);
                            break;
                        }
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    info!("[Runtime] Ctrl+C received, shutting down...");
                    break;
                }
            }
        }

        self.stop_all().await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_console_sender_prints() {
        let sender = ConsoleSender::as_send_fn();
        let source = MessageSource {
            platform: PlatformType::Custom,
            session_id: "test".to_string(),
            message_id: "msg-1".to_string(),
            user_id: "user-1".to_string(),
        };
        let chain = MessageChain::new().text("hello");
        let result = sender(source, chain).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_runtime_default_build_pipeline() {
        let mut runtime = BotRuntime::default();
        let sender = ConsoleSender::as_send_fn();
        let result = runtime.build_pipeline(sender).await;
        assert!(result.is_ok());
        assert!(runtime.pipeline.is_some());
    }
}
