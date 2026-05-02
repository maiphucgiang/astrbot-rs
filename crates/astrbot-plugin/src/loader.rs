use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::plugin::{PluginConfig, PluginContext, PluginMetadata, Star};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Descriptor for a discovered plugin on disk — not yet loaded into memory.
#[derive(Debug, Clone)]
pub struct StarDescriptor {
    pub metadata: PluginMetadata,
    pub path: PathBuf,
    pub name: String,
}

/// Plugin loader — scans disk, reads metadata, prepares for instantiation.
pub struct PluginLoader {
    plugin_dir: PathBuf,
}

impl PluginLoader {
    pub fn new(plugin_dir: PathBuf) -> Self {
        Self { plugin_dir }
    }

    pub fn plugin_dir(&self) -> &PathBuf {
        &self.plugin_dir
    }

    /// Scan plugin_dir and return all valid plugin descriptors.
    pub async fn scan(&self) -> Result<Vec<StarDescriptor>> {
        let mut descriptors = Vec::new();
        let mut entries = tokio::fs::read_dir(&self.plugin_dir)
            .await
            .map_err(|e| AstrBotError::Plugin {
                plugin: "loader".to_string(),
                message: format!("Failed to read plugin dir: {}", e),
            })?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| AstrBotError::Plugin {
                plugin: "loader".to_string(),
                message: format!("Failed to read dir entry: {}", e),
            })?
        {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let meta_path = path.join("metadata.json");
            if !meta_path.exists() {
                warn!("[PluginLoader] skipping {} — no metadata.json", path.display());
                continue;
            }

            let content = tokio::fs::read_to_string(&meta_path)
                .await
                .map_err(|e| AstrBotError::Plugin {
                    plugin: "loader".to_string(),
                    message: format!("Failed to read metadata: {}", e),
                })?;

            let metadata: PluginMetadata = serde_json::from_str(&content)
                .map_err(|e| AstrBotError::Serialization(format!("Invalid metadata: {}", e)))?;

            let name = metadata.name.clone();
            descriptors.push(StarDescriptor {
                metadata,
                path,
                name,
            });
        }

        info!("[PluginLoader] scanned {} plugins", descriptors.len());
        Ok(descriptors)
    }

    /// Read metadata for a specific installed plugin.
    pub async fn read_metadata(&self, name: &str) -> Result<PluginMetadata> {
        let path = self.plugin_dir.join(name).join("metadata.json");
        if !path.exists() {
            return Err(AstrBotError::NotFound(format!(
                "Plugin '{}' metadata not found",
                name
            )));
        }
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| AstrBotError::Plugin {
                plugin: "loader".to_string(),
                message: format!("Failed to read metadata: {}", e),
            })?;
        serde_json::from_str(&content)
            .map_err(|e| AstrBotError::Serialization(format!("Invalid metadata: {}", e)))
    }

    /// Instantiate a Star from descriptor.
    /// For now this is a skeleton — dynamic plugin loading (dlopen / wasm)
    /// will be implemented later. Returns a Star with metadata but no plugin impl.
    pub async fn instantiate(&self, desc: &StarDescriptor) -> Result<Star> {
        let mut star = Star::new(desc.metadata.clone());
        info!(
            "[PluginLoader] instantiated skeleton for '{}'",
            desc.name
        );
        Ok(star)
    }

    /// Hot reload: unload + re-scan + re-instantiate.
    pub async fn reload(&self, name: &str) -> Result<StarDescriptor> {
        let path = self.plugin_dir.join(name);
        if !path.exists() {
            return Err(AstrBotError::NotFound(format!(
                "Plugin '{}' not found for reload",
                name
            )));
        }
        let meta_path = path.join("metadata.json");
        let content = tokio::fs::read_to_string(&meta_path)
            .await
            .map_err(|e| AstrBotError::Plugin {
                plugin: "loader".to_string(),
                message: format!("Failed to read metadata on reload: {}", e),
            })?;
        let metadata: PluginMetadata = serde_json::from_str(&content)
            .map_err(|e| AstrBotError::Serialization(format!("Invalid metadata on reload: {}", e)))?;
        Ok(StarDescriptor {
            metadata,
            path,
            name: name.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_scan_finds_plugins() {
        let tmp = std::env::temp_dir().join(format!("astrbot_loader_scan_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        tokio::fs::create_dir_all(&tmp).await.unwrap();

        let p1 = tmp.join("plugin_a");
        tokio::fs::create_dir(&p1).await.unwrap();
        tokio::fs::write(
            p1.join("metadata.json"),
            r#"{"name":"plugin_a","author":"a","description":"","version":"1.0.0","platforms":[],"reserved":false}"#,
        )
        .await
        .unwrap();

        let p2 = tmp.join("plugin_b");
        tokio::fs::create_dir(&p2).await.unwrap();
        tokio::fs::write(
            p2.join("metadata.json"),
            r#"{"name":"plugin_b","author":"b","description":"","version":"2.0.0","platforms":[],"reserved":false}"#,
        )
        .await
        .unwrap();

        let loader = PluginLoader::new(tmp.clone());
        let found = loader.scan().await.unwrap();
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|d| d.name == "plugin_a"));
        assert!(found.iter().any(|d| d.name == "plugin_b"));

        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    #[tokio::test]
    async fn test_scan_skips_invalid() {
        let tmp = std::env::temp_dir().join(format!("astrbot_loader_skip_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        tokio::fs::create_dir_all(&tmp).await.unwrap();

        let p1 = tmp.join("good");
        tokio::fs::create_dir(&p1).await.unwrap();
        tokio::fs::write(
            p1.join("metadata.json"),
            r#"{"name":"good","author":"x","description":"","version":"1.0.0","platforms":[],"reserved":false}"#,
        )
        .await
        .unwrap();

        let _ = tmp.join("bad");
        tokio::fs::create_dir(tmp.join("bad")).await.unwrap();

        let loader = PluginLoader::new(tmp.clone());
        let found = loader.scan().await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "good");

        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    #[tokio::test]
    async fn test_instantiate_returns_star() {
        let tmp = std::env::temp_dir().join(format!("astrbot_loader_inst_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        tokio::fs::create_dir_all(&tmp).await.unwrap();

        let p = tmp.join("my_plugin");
        tokio::fs::create_dir(&p).await.unwrap();
        tokio::fs::write(
            p.join("metadata.json"),
            r#"{"name":"my_plugin","author":"me","description":"","version":"1.0.0","platforms":[],"reserved":false}"#,
        )
        .await
        .unwrap();

        let loader = PluginLoader::new(tmp.clone());
        let desc = loader.scan().await.unwrap().into_iter().next().unwrap();
        let star = loader.instantiate(&desc).await.unwrap();
        assert_eq!(star.metadata.name, "my_plugin");
        assert!(!star.activated);

        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }
}
