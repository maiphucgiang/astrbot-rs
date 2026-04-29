//! Plugin marketplace — registry of available plugins from remote sources
//!
//! Provides: remote index fetch, local disk cache, search/filter,
//! and install/uninstall delegation to PluginInstaller.

use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::plugin::PluginMetadata;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tracing::{info, warn};

use crate::installer::PluginInstaller;

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

/// Cached index with timestamp
#[derive(Debug, Default, Serialize, Deserialize)]
struct CachedIndex {
    fetched_at: u64, // epoch seconds
    plugins: HashMap<String, MarketPlugin>,
}

/// Marketplace index — holds all available plugins + cache + installer
pub struct PluginMarketplace {
    plugins: HashMap<String, MarketPlugin>,
    cache_path: Option<PathBuf>,
    cache_ttl: Duration,
    installer: Option<PluginInstaller>,
}

impl PluginMarketplace {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            cache_path: None,
            cache_ttl: Duration::from_secs(300), // 5 min default
            installer: None,
        }
    }

    /// Set disk cache path and TTL
    pub fn with_cache(mut self, path: PathBuf, ttl_secs: u64) -> Self {
        self.cache_path = Some(path);
        self.cache_ttl = Duration::from_secs(ttl_secs);
        self
    }

    /// Attach installer for install/uninstall operations
    pub fn with_installer(mut self, installer: PluginInstaller) -> Self {
        self.installer = Some(installer);
        self
    }

    /// Load from disk cache if present and not stale
    pub async fn load_cache(&mut self) -> Result<bool> {
        let path = match &self.cache_path {
            Some(p) => p,
            None => return Ok(false),
        };
        if !path.exists() {
            return Ok(false);
        }
        let data = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| AstrBotError::Plugin {
                plugin: "market".to_string(),
                message: format!("Failed to read cache: {}", e),
            })?;
        let cached: CachedIndex = serde_json::from_str(&data)
            .map_err(|e| AstrBotError::Serialization(format!("Cache JSON parse failed: {}", e)))?;
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if now.saturating_sub(cached.fetched_at) > self.cache_ttl.as_secs() {
            info!("[PluginMarketplace] cache stale, refetch needed");
            return Ok(false);
        }
        self.plugins = cached.plugins;
        info!(
            "[PluginMarketplace] loaded {} plugins from cache",
            self.plugins.len()
        );
        Ok(true)
    }

    /// Save current index to disk cache
    async fn save_cache(&self) -> Result<()> {
        let path = match &self.cache_path {
            Some(p) => p,
            None => return Ok(()),
        };
        let cached = CachedIndex {
            fetched_at: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            plugins: self.plugins.clone(),
        };
        let json = serde_json::to_string_pretty(&cached)
            .map_err(|e| AstrBotError::Serialization(format!("Cache JSON export failed: {}", e)))?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        tokio::fs::write(path, json)
            .await
            .map_err(|e| AstrBotError::Plugin {
                plugin: "market".to_string(),
                message: format!("Failed to write cache: {}", e),
            })?;
        Ok(())
    }

    /// Fetch remote index from URL (HTTP GET)
    pub async fn fetch_remote(&mut self, url: &str) -> Result<u64> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| AstrBotError::Internal(format!("HTTP client build failed: {}", e)))?;

        let resp =
            client.get(url).send().await.map_err(|e| {
                AstrBotError::Internal(format!("Failed to fetch marketplace: {}", e))
            })?;

        if !resp.status().is_success() {
            return Err(AstrBotError::Internal(format!(
                "Marketplace HTTP {}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            )));
        }

        let json = resp.text().await.map_err(|e| {
            AstrBotError::Internal(format!("Failed to read marketplace body: {}", e))
        })?;

        let fetched: HashMap<String, MarketPlugin> = serde_json::from_str(&json).map_err(|e| {
            AstrBotError::Serialization(format!("Marketplace JSON parse failed: {}", e))
        })?;

        let count = fetched.len() as u64;
        self.plugins = fetched;
        self.save_cache().await?;
        info!("[PluginMarketplace] fetched {} plugins from {}", count, url);
        Ok(count)
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

    /// Load marketplace from JSON string (manual import)
    pub fn load_json(json: &str) -> Result<Self> {
        let plugins: HashMap<String, MarketPlugin> = serde_json::from_str(json).map_err(|e| {
            AstrBotError::Serialization(format!("Marketplace JSON parse failed: {}", e))
        })?;
        Ok(Self {
            plugins,
            cache_path: None,
            cache_ttl: Duration::from_secs(300),
            installer: None,
        })
    }

    /// Export marketplace to JSON string
    pub fn export_json(&self) -> Result<String> {
        serde_json::to_string(&self.plugins).map_err(|e| {
            AstrBotError::Serialization(format!("Marketplace JSON export failed: {}", e))
        })
    }

    /// Install a marketplace plugin by ID (delegates to PluginInstaller)
    pub async fn install(&self, id: &str, source_path: &std::path::Path) -> Result<PluginMetadata> {
        let installer = self
            .installer
            .as_ref()
            .ok_or_else(|| AstrBotError::Internal("PluginInstaller not attached".to_string()))?;
        let plugin = self.plugins.get(id).ok_or_else(|| {
            AstrBotError::NotFound(format!("Plugin '{}' not found in marketplace", id))
        })?;
        installer.install_from_path(source_path, &plugin.id).await
    }

    /// Uninstall a plugin by ID (delegates to PluginInstaller)
    pub async fn uninstall(&self, id: &str) -> Result<()> {
        let installer = self
            .installer
            .as_ref()
            .ok_or_else(|| AstrBotError::Internal("PluginInstaller not attached".to_string()))?;
        installer.uninstall(id).await
    }

    /// Check if a plugin is installed
    pub async fn is_installed(&self, id: &str) -> Result<bool> {
        let installer = self
            .installer
            .as_ref()
            .ok_or_else(|| AstrBotError::Internal("PluginInstaller not attached".to_string()))?;
        Ok(installer.is_installed(id).await)
    }

    /// Refresh: load cache if fresh, else fetch remote
    pub async fn refresh(&mut self, remote_url: Option<&str>) -> Result<u64> {
        if self.load_cache().await? {
            return Ok(self.plugins.len() as u64);
        }
        match remote_url {
            Some(url) => self.fetch_remote(url).await,
            None => Ok(0),
        }
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

    #[tokio::test]
    async fn test_marketplace_cache() {
        let tmp = std::env::temp_dir().join("astrbot_test_market_cache");
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        tokio::fs::create_dir_all(&tmp).await.unwrap();

        let cache = tmp.join("market.json");
        let mut market = PluginMarketplace::new().with_cache(cache.clone(), 60);
        market.add(MarketPlugin {
            id: "test".to_string(),
            name: "Test".to_string(),
            description: "desc".to_string(),
            version: "1.0.0".to_string(),
            author: "dev".to_string(),
            download_url: "https://example.com/test.zip".to_string(),
            tags: vec![],
            installs: 0,
            rating: None,
        });
        market.save_cache().await.unwrap();
        assert!(cache.exists());

        let mut market2 = PluginMarketplace::new().with_cache(cache, 60);
        let loaded = market2.load_cache().await.unwrap();
        assert!(loaded);
        assert_eq!(market2.list(None).len(), 1);

        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }
}
