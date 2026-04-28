//! Plugin marketplace — registry of available plugins from remote sources
//!
//! Skeleton: stores plugin listings fetched from a remote index.

use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::plugin::PluginMetadata;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A plugin available in the marketplace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketPlugin {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub author: String,
    pub download_url: String,
    pub tags: Vec<String>,
    pub installs: u64,
    pub rating: Option<f32>,
}

/// Marketplace index — holds all available plugins
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PluginMarketplace {
    plugins: HashMap<String, MarketPlugin>,
}

impl PluginMarketplace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a plugin to the marketplace
    pub fn add(&mut self, plugin: MarketPlugin) {
        self.plugins.insert(plugin.id.clone(), plugin);
    }

    /// Remove a plugin from the marketplace
    pub fn remove(&mut self, id: &str) -> Option<MarketPlugin> {
        self.plugins.remove(id)
    }

    /// Get a plugin by ID
    pub fn get(&self, id: &str) -> Option<&MarketPlugin> {
        self.plugins.get(id)
    }

    /// List all plugins with optional tag filter
    pub fn list(&self, tag_filter: Option<&str>) -> Vec<&MarketPlugin> {
        self.plugins
            .values()
            .filter(|p| {
                if let Some(tag) = tag_filter {
                    p.tags.iter().any(|t| t.eq_ignore_ascii_case(tag))
                } else {
                    true
                }
            })
            .collect()
    }

    /// Search plugins by keyword (name or description)
    pub fn search(&self, keyword: &str) -> Vec<&MarketPlugin> {
        let kw = keyword.to_lowercase();
        self.plugins
            .values()
            .filter(|p| {
                p.name.to_lowercase().contains(&kw)
                    || p.description.to_lowercase().contains(&kw)
                    || p.tags.iter().any(|t| t.to_lowercase().contains(&kw))
            })
            .collect()
    }

    /// Load marketplace from JSON string
    pub fn load_json(json: &str) -> Result<Self> {
        serde_json::from_str(json)
            .map_err(|e| AstrBotError::Serialization(format!("Marketplace JSON parse failed: {}", e)))
    }

    /// Export marketplace to JSON string
    pub fn export_json(&self) -> Result<String> {
        serde_json::to_string(self)
            .map_err(|e| AstrBotError::Serialization(format!("Marketplace JSON export failed: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_marketplace_add_and_list() {
        let mut market = PluginMarketplace::new();
        market.add(MarketPlugin {
            id: "weather".to_string(),
            name: "Weather".to_string(),
            description: "Get weather info".to_string(),
            version: "1.0.0".to_string(),
            author: "dev".to_string(),
            download_url: "https://example.com/weather.zip".to_string(),
            tags: vec!["utility".to_string()],
            installs: 42,
            rating: Some(4.5),
        });
        assert_eq!(market.list(None).len(), 1);
        assert_eq!(market.list(Some("utility")).len(), 1);
        assert_eq!(market.list(Some("game")).len(), 0);
    }

    #[test]
    fn test_marketplace_search() {
        let mut market = PluginMarketplace::new();
        market.add(MarketPlugin {
            id: "weather".to_string(),
            name: "Weather".to_string(),
            description: "Get weather info".to_string(),
            version: "1.0.0".to_string(),
            author: "dev".to_string(),
            download_url: "https://example.com/weather.zip".to_string(),
            tags: vec!["utility".to_string()],
            installs: 42,
            rating: Some(4.5),
        });
        assert_eq!(market.search("weather").len(), 1);
        assert_eq!(market.search("game").len(), 0);
    }

    #[test]
    fn test_marketplace_json_roundtrip() {
        let mut market = PluginMarketplace::new();
        market.add(MarketPlugin {
            id: "weather".to_string(),
            name: "Weather".to_string(),
            description: "Get weather info".to_string(),
            version: "1.0.0".to_string(),
            author: "dev".to_string(),
            download_url: "https://example.com/weather.zip".to_string(),
            tags: vec!["utility".to_string()],
            installs: 42,
            rating: Some(4.5),
        });
        let json = market.export_json().unwrap();
        let market2 = PluginMarketplace::load_json(&json).unwrap();
        assert_eq!(market2.list(None).len(), 1);
    }
}
