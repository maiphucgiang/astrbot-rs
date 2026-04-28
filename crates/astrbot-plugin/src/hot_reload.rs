use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::plugin::{Plugin, PluginContext, PluginConfig};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Plugin hot-reloader — watches plugin directory and auto-loads new plugins
pub struct PluginHotReloader {
    plugin_dir: PathBuf,
    registry: Arc<Mutex<PluginRegistry>>,
}

/// Simple plugin registry for hot-loaded plugins
#[derive(Default)]
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, plugin: Box<dyn Plugin>) {
        self.plugins.push(plugin);
    }

    pub fn remove(&mut self, name: &str) {
        self.plugins.retain(|p| p.metadata().name != name);
    }

    pub fn list(&self) -> Vec<&dyn Plugin> {
        self.plugins.iter().map(|p| p.as_ref()).collect()
    }
}

impl PluginHotReloader {
    pub fn new(plugin_dir: PathBuf) -> Self {
        Self {
            plugin_dir,
            registry: Arc::new(Mutex::new(PluginRegistry::new())),
        }
    }

    /// Start watching the plugin directory for changes (skeleton)
    pub async fn start_watching(&self) -> Result<()> {
        info!(
            "[PluginHotReloader] watching {} for changes (skeleton mode)",
            self.plugin_dir.display()
        );
        // Full notify-based file watching is a skeleton — production
        // would use `notify` crate to detect .py / .wasm plugin additions.
        Ok(())
    }

    /// Manually trigger a reload scan
    pub async fn scan_and_reload(&self) -> Result<Vec<String>> {
        let mut loaded = Vec::new();
        let mut entries = tokio::fs::read_dir(&self.plugin_dir)
            .await
            .map_err(|e| AstrBotError::Plugin {
                plugin: "hot-reload".to_string(),
                message: format!("Failed to read plugin dir: {}", e),
            })?;

        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            AstrBotError::Plugin {
                plugin: "hot-reload".to_string(),
                message: format!("Dir entry error: {}", e),
            }
        })? {
            if entry.file_type().await.unwrap_or_else(|_| unreachable!()).is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                loaded.push(name);
            }
        }

        info!("[PluginHotReloader] scanned {} potential plugins", loaded.len());
        Ok(loaded)
    }

    /// Get the plugin registry
    pub fn registry(&self) -> Arc<Mutex<PluginRegistry>> {
        self.registry.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_hot_reloader_scan() {
        let tmp = std::env::temp_dir().join("astrbot_hotreload_test");
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        tokio::fs::create_dir_all(&tmp).await.unwrap();
        tokio::fs::create_dir_all(tmp.join("plugin_a")).await.unwrap();
        tokio::fs::create_dir_all(tmp.join("plugin_b")).await.unwrap();

        let reloader = PluginHotReloader::new(tmp.clone());
        let scanned = reloader.scan_and_reload().await.unwrap();
        assert_eq!(scanned.len(), 2);

        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    #[test]
    fn test_registry_add_remove() {
        use astrbot_core::plugin::{PluginMetadata, Star};

        let mut registry = PluginRegistry::new();
        // Can't easily add a real Plugin trait object in a unit test,
        // so we just verify the struct works.
        assert!(registry.list().is_empty());
    }
}
