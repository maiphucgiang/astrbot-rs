use crate::errors::{AstrBotError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub mod env_override;
pub mod hot_reload;

pub use env_override::*;
pub use hot_reload::*;

/// Core AstrBot configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AstrBotConfig {
    /// Bot nickname
    pub nickname: String,
    /// Command prefixes (e.g., ["/", "!"])
    pub prefixes: Vec<String>,
    /// Admin user IDs
    pub admins: Vec<String>,
    /// Platform adapter configurations
    pub platforms: Vec<PlatformConfig>,
    /// LLM provider configurations
    pub providers: Vec<ProviderConfig>,
    /// Plugin configurations
    pub plugins: HashMap<String, serde_json::Value>,
    /// WebUI configuration
    pub webui: WebUiConfig,
    /// Log level
    pub log_level: String,
    /// Database URL
    pub database_url: String,
    /// Additional settings
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Default for AstrBotConfig {
    fn default() -> Self {
        Self {
            nickname: "AstrBot".to_string(),
            prefixes: vec!["/".to_string()],
            admins: Vec::new(),
            platforms: Vec::new(),
            providers: Vec::new(),
            plugins: HashMap::new(),
            webui: WebUiConfig::default(),
            log_level: "info".to_string(),
            database_url: "sqlite:data/data.db".to_string(),
            extra: HashMap::new(),
        }
    }
}

impl AstrBotConfig {
    /// Load configuration from a file (JSON, YAML, or TOML)
    pub async fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| AstrBotError::Config(format!("failed to read config file: {}", e)))?;

        let config: Self = if path.extension().and_then(|s| s.to_str()) == Some("json") {
            serde_json::from_str(&content)
                .map_err(|e| AstrBotError::Config(format!("invalid JSON: {}", e)))?
        } else if path.extension().and_then(|s| s.to_str()) == Some("yaml")
            || path.extension().and_then(|s| s.to_str()) == Some("yml")
        {
            serde_yaml::from_str(&content)
                .map_err(|e| AstrBotError::Config(format!("invalid YAML: {}", e)))?
        } else {
            toml::from_str(&content)
                .map_err(|e| AstrBotError::Config(format!("invalid TOML: {}", e)))?
        };

        Ok(config)
    }

    /// Save configuration to a file
    pub async fn to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path = path.as_ref();
        let content = if path.extension().and_then(|s| s.to_str()) == Some("json") {
            serde_json::to_string_pretty(self)
                .map_err(|e| AstrBotError::Serialization(e.to_string()))?
        } else if path.extension().and_then(|s| s.to_str()) == Some("yaml")
            || path.extension().and_then(|s| s.to_str()) == Some("yml")
        {
            serde_yaml::to_string(self).map_err(|e| AstrBotError::Serialization(e.to_string()))?
        } else {
            toml::to_string_pretty(self).map_err(|e| AstrBotError::Serialization(e.to_string()))?
        };

        tokio::fs::write(path, content)
            .await
            .map_err(|e| AstrBotError::Config(format!("failed to write config: {}", e)))?;

        Ok(())
    }

    /// Check if a user is an admin
    pub fn is_admin(&self, user_id: &str) -> bool {
        self.admins.contains(&user_id.to_string())
    }

    /// Get plugin config by name
    pub fn plugin_config(&self, name: &str) -> Option<&serde_json::Value> {
        self.plugins.get(name)
    }
}

/// Platform adapter configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlatformConfig {
    /// Unique adapter ID
    pub id: String,
    /// Platform type
    pub platform_type: String,
    /// Whether enabled
    pub enabled: bool,
    /// Platform-specific configuration
    #[serde(flatten)]
    pub config: HashMap<String, serde_json::Value>,
}

/// LLM provider configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    /// Unique provider ID
    pub id: String,
    /// Provider type (openai, anthropic, gemini, etc.)
    pub provider_type: String,
    /// API key or credentials
    pub api_key: Option<String>,
    /// Base URL (for custom endpoints)
    pub base_url: Option<String>,
    /// Default model name
    pub model: String,
    /// Whether enabled
    pub enabled: bool,
    /// Additional provider-specific settings
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// WebUI configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WebUiConfig {
    /// Whether WebUI is enabled
    pub enabled: bool,
    /// Host to bind
    pub host: String,
    /// Port to bind
    pub port: u16,
    /// JWT secret
    pub jwt_secret: String,
    /// TLS certificate path (optional)
    pub tls_cert: Option<String>,
    /// TLS private key path (optional)
    pub tls_key: Option<String>,
}

impl Default for WebUiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            host: "0.0.0.0".to_string(),
            port: 6185,
            jwt_secret: "change-me-in-production".to_string(),
            tls_cert: None,
            tls_key: None,
        }
    }
}
