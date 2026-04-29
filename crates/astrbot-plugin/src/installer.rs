use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::plugin::PluginMetadata;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Plugin installer — handles installation / uninstallation of plugins
pub struct PluginInstaller {
    /// Directory where plugins are installed
    plugin_dir: PathBuf,
}

impl PluginInstaller {
    pub fn new(plugin_dir: PathBuf) -> Self {
        Self { plugin_dir }
    }

    /// Install a plugin from a local directory path
    pub async fn install_from_path(
        &self,
        source: &Path,
        target_name: &str,
    ) -> Result<PluginMetadata> {
        if !source.exists() {
            return Err(AstrBotError::NotFound(format!(
                "Source path does not exist: {}",
                source.display()
            )));
        }

        let target = self.plugin_dir.join(target_name);
        if target.exists() {
            return Err(AstrBotError::Validation(format!(
                "Plugin '{}' already installed",
                target_name
            )));
        }

        // Copy source directory to plugin_dir
        tokio::fs::create_dir_all(&self.plugin_dir)
            .await
            .map_err(|e| AstrBotError::Plugin {
                plugin: "installer".to_string(),
                message: format!("Failed to create plugin dir: {}", e),
            })?;

        copy_dir_recursive(source, &target)
            .await
            .map_err(|e| AstrBotError::Plugin {
                plugin: "installer".to_string(),
                message: format!("Failed to copy plugin: {}", e),
            })?;

        // Read metadata
        let meta_path = target.join("metadata.json");
        let meta: PluginMetadata = if meta_path.exists() {
            let content =
                tokio::fs::read_to_string(&meta_path)
                    .await
                    .map_err(|e| AstrBotError::Plugin {
                        plugin: "installer".to_string(),
                        message: format!("Failed to read metadata: {}", e),
                    })?;
            serde_json::from_str(&content)
                .map_err(|e| AstrBotError::Serialization(format!("Invalid metadata JSON: {}", e)))?
        } else {
            PluginMetadata {
                name: target_name.to_string(),
                author: "unknown".to_string(),
                description: "".to_string(),
                version: "0.1.0".to_string(),
                repository: None,
                min_astrbot_version: None,
                platforms: vec![],
                reserved: false,
                logo: None,
            }
        };

        info!(
            "[PluginInstaller] installed '{}' v{} from {}",
            meta.name,
            meta.version,
            source.display()
        );
        Ok(meta)
    }

    /// Install a plugin from a Git URL (skeleton — clone not implemented)
    pub async fn install_from_git(&self, _url: &str, _target_name: &str) -> Result<PluginMetadata> {
        warn!("[PluginInstaller] install_from_git is a skeleton — not yet implemented");
        Err(AstrBotError::NotImplemented(
            "Git-based plugin installation is not yet implemented".to_string(),
        ))
    }

    /// Uninstall a plugin by name
    pub async fn uninstall(&self, name: &str) -> Result<()> {
        let target = self.plugin_dir.join(name);
        if !target.exists() {
            return Err(AstrBotError::NotFound(format!(
                "Plugin '{}' is not installed",
                name
            )));
        }

        tokio::fs::remove_dir_all(&target)
            .await
            .map_err(|e| AstrBotError::Plugin {
                plugin: "installer".to_string(),
                message: format!("Failed to uninstall plugin: {}", e),
            })?;

        info!("[PluginInstaller] uninstalled '{}'", name);
        Ok(())
    }

    pub async fn list_installed(&self) -> Result<Vec<String>> {
        let mut plugins = Vec::new();
        let mut entries =
            tokio::fs::read_dir(&self.plugin_dir)
                .await
                .map_err(|e| AstrBotError::Plugin {
                    plugin: "installer".to_string(),
                    message: format!("Failed to read plugin dir: {}", e),
                })?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| AstrBotError::Plugin {
                plugin: "installer".to_string(),
                message: format!("Failed to read dir entry: {}", e),
            })?
        {
            if entry
                .file_type()
                .await
                .unwrap_or_else(|_| unreachable!())
                .is_dir()
            {
                plugins.push(entry.file_name().to_string_lossy().to_string());
            }
        }
        Ok(plugins)
    }

    /// Check if a plugin is installed
    pub async fn is_installed(&self, name: &str) -> bool {
        self.plugin_dir.join(name).exists()
    }
}

/// Recursively copy a directory
async fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dst).await?;
    let mut entries = tokio::fs::read_dir(src).await?;
    while let Some(entry) = entries.next_entry().await? {
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if entry.file_type().await?.is_dir() {
            Box::pin(copy_dir_recursive(&src_path, &dst_path)).await?;
        } else {
            tokio::fs::copy(&src_path, &dst_path).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_installer_install_and_uninstall() {
        let tmp = std::env::temp_dir().join("astrbot_test_plugins");
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        let installer = PluginInstaller::new(tmp.clone());

        // Create a fake plugin source
        let src = tmp.join("source_plugin");
        tokio::fs::create_dir_all(&src).await.unwrap();
        tokio::fs::write(src.join("main.rs"), "// plugin code")
            .await
            .unwrap();

        // Install
        let meta = installer
            .install_from_path(&src, "test_plugin")
            .await
            .unwrap();
        assert_eq!(meta.name, "test_plugin");
        assert!(installer.is_installed("test_plugin").await);

        // List
        let list = installer.list_installed().await.unwrap();
        assert!(list.contains(&"test_plugin".to_string()));

        // Uninstall
        installer.uninstall("test_plugin").await.unwrap();
        assert!(!installer.is_installed("test_plugin").await);

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    #[tokio::test]
    async fn test_installer_duplicate_fails() {
        let tmp = std::env::temp_dir().join("astrbot_test_dup");
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        let installer = PluginInstaller::new(tmp.clone());

        let src = tmp.join("src");
        tokio::fs::create_dir_all(&src).await.unwrap();
        installer.install_from_path(&src, "dup").await.unwrap();

        let result = installer.install_from_path(&src, "dup").await;
        assert!(result.is_err());

        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }
}
