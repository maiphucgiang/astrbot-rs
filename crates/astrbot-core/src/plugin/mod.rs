use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::errors::Result;
use crate::event::{Event, EventResult};
use crate::message::{MessageChain, MessageEventResult};
use crate::platform::MessageSource;

mod sender;
pub use sender::{MessageSender, PluginMessageSender};

/// Plugin lifecycle states
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PluginLifecycle {
    Installed,
    Loaded,
    Initialized,
    Running,
    Stopped,
    Unloaded,
    Error,
}

/// Plugin metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMetadata {
    /// Plugin name (unique identifier)
    pub name: String,
    /// Plugin author
    pub author: String,
    /// Description
    pub description: String,
    /// Version (semver)
    pub version: String,
    /// Repository URL
    pub repository: Option<String>,
    /// Minimum AstrBot version required
    pub min_astrbot_version: Option<String>,
    /// Supported platforms
    pub platforms: Vec<String>,
    /// Whether this is a reserved/built-in plugin
    pub reserved: bool,
    /// Logo path
    pub logo: Option<String>,
}

/// Plugin configuration schema entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfigSchema {
    pub key: String,
    /// Type: string, int, float, bool, list, dict
    pub value_type: String,
    pub default: Option<serde_json::Value>,
    pub description: String,
    pub required: bool,
}

/// Plugin configuration (runtime values)
pub type PluginConfig = HashMap<String, serde_json::Value>;

/// Context passed to plugins for interacting with the bot
#[derive(Clone)]
pub struct PluginContext {
    /// Bot's self identifier (platform-specific)
    pub bot_id: String,
    /// Platform type
    pub platform: String,
    /// Shared config store
    pub config: Arc<dashmap::DashMap<String, serde_json::Value>>,
    /// Message sender for plugins to proactively send messages
    message_sender: Option<Arc<dyn MessageSender>>,
}

impl PluginContext {
    pub fn new(bot_id: String, platform: String) -> Self {
        Self {
            bot_id,
            platform,
            config: Arc::new(dashmap::DashMap::new()),
            message_sender: None,
        }
    }

    /// Set the message sender (called by the framework during initialization)
    pub fn set_message_sender(&mut self, sender: Arc<dyn MessageSender>) {
        self.message_sender = Some(sender);
    }

    /// Send a message to a target (plugin → platform)
    pub async fn send_message(&self, source: &MessageSource, chain: MessageChain) -> Result<()> {
        if let Some(ref sender) = self.message_sender {
            sender.send(source, chain).await
        } else {
            Err(crate::errors::AstrBotError::Internal(
                "Message sender not available in PluginContext".to_string(),
            ))
        }
    }

    /// Send plain text to a target (convenience)
    pub async fn send_text(&self, source: &MessageSource, text: impl Into<String>) -> Result<()> {
        self.send_message(source, MessageChain::new().text(text))
            .await
    }

    /// Get a config value
    pub fn get_config(&self, key: &str) -> Option<serde_json::Value> {
        self.config.get(key).map(|v| v.value().clone())
    }

    /// Set a config value
    pub fn set_config(&self, key: String, value: serde_json::Value) {
        self.config.insert(key, value);
    }
}

/// Core trait for all AstrBot plugins (Stars)
#[async_trait]
pub trait Plugin: Send + Sync {
    /// Get plugin metadata reference
    fn metadata(&self) -> &PluginMetadata;

    /// Get plugin metadata (convenience clone)
    fn get_metadata(&self) -> PluginMetadata {
        self.metadata().clone()
    }

    /// Handle an incoming message (default delegates to on_event)
    async fn on_message(
        &self,
        message: &crate::message::AstrBotMessage,
        source: &MessageSource,
    ) -> Result<MessageEventResult> {
        let event = crate::event::MessageEvent {
            source: source.clone(),
            message: message.clone(),
        };
        let result = self.on_event(&event).await?;
        Ok(result.into_message_result())
    }

    /// Get configuration schema
    fn config_schema(&self) -> Vec<PluginConfigSchema> {
        vec![]
    }

    /// Initialize the plugin with configuration
    async fn initialize(&mut self, config: PluginConfig, ctx: PluginContext) -> Result<()>;

    /// Start the plugin (called after initialize)
    async fn start(&mut self) -> Result<()>;

    /// Stop the plugin (graceful shutdown)
    async fn stop(&mut self) -> Result<()>;

    /// Handle an incoming event
    async fn on_event(&self, event: &dyn Event) -> Result<EventResult>;

    /// Check if this plugin can handle the given event
    fn can_handle(&self, event: &dyn Event) -> bool;

    /// Register commands this plugin provides (return map of command name → description)
    fn commands(&self) -> Vec<(String, String, bool)> {
        vec![]
    }

    /// Handle a command invoked by the user
    async fn on_command(
        &self,
        _command: &str,
        _args: &[String],
        _source: &MessageSource,
        _user_id: &str,
    ) -> Result<MessageEventResult> {
        Err(crate::errors::AstrBotError::NotFound(format!(
            "Command not handled by this plugin"
        )))
    }
}

/// A registered plugin instance (Star)
pub struct Star {
    pub metadata: PluginMetadata,
    pub config: PluginConfig,
    pub activated: bool,
    pub lifecycle: PluginLifecycle,
    pub instance_id: String,
    /// The actual plugin implementation
    pub plugin: Option<Box<dyn Plugin>>,
}

impl Star {
    pub fn new(metadata: PluginMetadata) -> Self {
        Self {
            metadata,
            config: HashMap::new(),
            activated: false,
            lifecycle: PluginLifecycle::Installed,
            instance_id: String::new(),
            plugin: None,
        }
    }
}
