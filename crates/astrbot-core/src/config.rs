use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotConfig {
    pub bot_name: String,
    pub admin_id: Option<String>,
    pub log_level: String,
    pub enabled_plugins: Vec<String>,
    pub enabled_adapters: Vec<String>,
    pub enabled_providers: Vec<String>,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            bot_name: "AstrBot".to_string(),
            admin_id: None,
            log_level: "info".to_string(),
            enabled_plugins: vec![],
            enabled_adapters: vec![],
            enabled_providers: vec![],
        }
    }
}
