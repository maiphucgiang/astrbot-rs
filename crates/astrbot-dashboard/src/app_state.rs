use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use astrbot_core::config::AstrBotConfig;
use astrbot_plugin::PluginManager;
use astrbot_provider::client::ProviderManager;
use astrbot_persona::PersonaManager;
use crate::sse::SseBroadcaster;
use crate::log_stream::LogBroadcaster;

#[derive(Clone)]
pub struct AppState {
    pub version: String,
    pub start_time: std::time::Instant,
    pub config: Arc<RwLock<AstrBotConfig>>,
    pub config_path: String,
    pub persona_manager: Arc<std::sync::Mutex<PersonaManager>>,
    pub sse_broadcaster: Option<Arc<SseBroadcaster>>,
    pub plugin_manager: Option<Arc<RwLock<PluginManager>>>,
    pub provider_manager: Option<Arc<RwLock<ProviderManager>>>,
    pub pipeline: Option<Arc<astrbot_core::pipeline::PipelineScheduler>>,
    pub db: Option<Arc<astrbot_core::db::Database>>,
    pub jwt_secret: Option<String>,
    pub admin_password: Option<String>,
    pub log_broadcaster: Option<Arc<LogBroadcaster>>,
    pub persona_registry: Option<Arc<astrbot_core::persona::PersonaRegistry>>,
    pub tool_registry: Option<Arc<astrbot_core::tools::ToolRegistry>>,
    pub backup_manager: Option<Arc<astrbot_core::backup::BackupManager>>,
    pub agent_registry: Option<Arc<RwLock<astrbot_core::agent::AgentRegistry>>>,
    pub mcp_registry: Option<Arc<astrbot_core::mcp::McpServerRegistry>>,
    pub webhook_manager: Option<Arc<astrbot_core::webhook::WebhookManager>>,
    pub safety_engine: Option<Arc<astrbot_core::safety::SafetyEngine>>,
    pub metrics_collector: Option<Arc<Mutex<astrbot_core::metrics::MetricsCollector>>>,
}

impl AppState {
    pub fn new(version: impl Into<String>) -> Self {
        let persona_mgr = PersonaManager::new(Some("data/personas.db".to_string()));
        Self {
            version: version.into(),
            start_time: std::time::Instant::now(),
            config: Arc::new(RwLock::new(AstrBotConfig::default())),
            config_path: "data/config.json".to_string(),
            persona_manager: Arc::new(std::sync::Mutex::new(persona_mgr)),
            sse_broadcaster: Some(Arc::new(SseBroadcaster::default())),
            plugin_manager: None,
            provider_manager: None,
            pipeline: None,
            db: None,
            jwt_secret: None,
            admin_password: None,
            log_broadcaster: None,
            persona_registry: None,
            tool_registry: None,
            backup_manager: None,
            agent_registry: None,
            mcp_registry: None,
            webhook_manager: None,
            safety_engine: None,
            metrics_collector: None,
        }
    }

    pub fn with_config(mut self, config: AstrBotConfig) -> Self {
        self.config = Arc::new(RwLock::new(config));
        self
    }

    pub fn with_db(mut self, db: Arc<astrbot_core::db::Database>) -> Self {
        self.db = Some(db);
        self
    }

    pub fn with_jwt(mut self, secret: String, password: String) -> Self {
        self.jwt_secret = Some(secret);
        self.admin_password = Some(password);
        self
    }

    pub fn with_log_broadcaster(mut self, broadcaster: Arc<LogBroadcaster>) -> Self {
        self.log_broadcaster = Some(broadcaster);
        self
    }

    pub fn with_plugin_manager(mut self, pm: Arc<RwLock<PluginManager>>) -> Self {
        self.plugin_manager = Some(pm);
        self
    }

    pub fn with_provider_manager(mut self, pm: Arc<RwLock<ProviderManager>>) -> Self {
        self.provider_manager = Some(pm);
        self
    }

    pub fn with_pipeline(mut self, p: Arc<astrbot_core::pipeline::PipelineScheduler>) -> Self {
        self.pipeline = Some(p);
        self
    }

    pub fn with_persona_registry(mut self, pr: Arc<astrbot_core::persona::PersonaRegistry>) -> Self {
        self.persona_registry = Some(pr);
        self
    }

    pub fn with_tool_registry(mut self, tr: Arc<astrbot_core::tools::ToolRegistry>) -> Self {
        self.tool_registry = Some(tr);
        self
    }

    pub fn with_backup_manager(mut self, bm: Arc<astrbot_core::backup::BackupManager>) -> Self {
        self.backup_manager = Some(bm);
        self
    }

    pub fn with_agent_registry(mut self, ar: Arc<RwLock<astrbot_core::agent::AgentRegistry>>) -> Self {
        self.agent_registry = Some(ar);
        self
    }

    pub fn with_mcp_registry(mut self, mr: Arc<astrbot_core::mcp::McpServerRegistry>) -> Self {
        self.mcp_registry = Some(mr);
        self
    }

    pub fn with_webhook_manager(mut self, wm: Arc<astrbot_core::webhook::WebhookManager>) -> Self {
        self.webhook_manager = Some(wm);
        self
    }

    pub fn with_safety_engine(mut self, se: Arc<astrbot_core::safety::SafetyEngine>) -> Self {
        self.safety_engine = Some(se);
        self
    }

    pub fn with_metrics_collector(mut self, mc: Arc<Mutex<astrbot_core::metrics::MetricsCollector>>) -> Self {
        self.metrics_collector = Some(mc);
        self
    }

    pub async fn load_config(&self) -> anyhow::Result<()> {
        let path = &self.config_path;
        if tokio::fs::try_exists(path).await.unwrap_or(false) {
            let config = AstrBotConfig::from_file(path).await?;
            let mut cfg = self.config.write().await;
            *cfg = config;
        }
        Ok(())
    }

    pub async fn save_config(&self) -> anyhow::Result<()> {
        let cfg = self.config.read().await;
        AstrBotConfig::to_file(&*cfg, &self.config_path).await?;
        Ok(())
    }
}
