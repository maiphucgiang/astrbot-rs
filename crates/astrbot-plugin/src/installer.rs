use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::plugin::PluginMetadata;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub struct PluginInstaller {
    plugin_dir: PathBuf,
}

impl PluginInstaller {
    pub fn new(plugin_dir: PathBuf) -> Self {
        Self { plugin_dir }
    }

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

    pub async fn install_from_git(&self, _url: &str, _target_name: &str) -> Result<PluginMetadata> {
        warn!("[PluginInstaller] install_from_git is a skeleton");
        Err(AstrBotError::NotImplemented(
            "Git-based plugin installation is not yet implemented".to_string(),
        ))
    }

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

    pub async fn is_installed(&self, name: &str) -> bool {
        self.plugin_dir.join(name).exists()
    }

    pub async fn install_pip_package(
        &self,
        package: &str,
        version: Option<&str>,
    ) -> Result<PipInstallRecord> {
        let spec = match version {
            Some(v) => format!("{}=={}", package, v),
            None => package.to_string(),
        };
        info!("[PluginInstaller] pip install {}", spec);
        let output = tokio::process::Command::new("pip3")
            .args(["install", "--quiet", &spec])
            .output()
            .await
            .map_err(|e| AstrBotError::Plugin {
                plugin: "installer".to_string(),
                message: format!("pip install failed to spawn: {}", e),
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AstrBotError::Plugin {
                plugin: "installer".to_string(),
                message: format!("pip install failed: {}", stderr),
            });
        }
        let ver = self.query_pip_version(package).await?;
        let record = PipInstallRecord {
            package: package.to_string(),
            version: ver,
            installed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };
        self.save_pip_registry(&record).await?;
        info!(
            "[PluginInstaller] pip package '{}' v{} installed",
            record.package, record.version
        );
        Ok(record)
    }

    pub async fn uninstall_pip_package(&self, package: &str) -> Result<()> {
        info!("[PluginInstaller] pip uninstall {}", package);
        let output = tokio::process::Command::new("pip3")
            .args(["uninstall", "-y", "--quiet", package])
            .output()
            .await
            .map_err(|e| AstrBotError::Plugin {
                plugin: "installer".to_string(),
                message: format!("pip uninstall failed to spawn: {}", e),
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AstrBotError::Plugin {
                plugin: "installer".to_string(),
                message: format!("pip uninstall failed: {}", stderr),
            });
        }
        self.remove_from_pip_registry(package).await?;
        info!("[PluginInstaller] pip package '{}' uninstalled", package);
        Ok(())
    }

    pub async fn list_installed_pip_packages(&self) -> Result<Vec<PipInstallRecord>> {
        let reg = self.pip_registry_path();
        if !reg.exists() {
            return Ok(vec![]);
        }
        let content = tokio::fs::read_to_string(&reg)
            .await
            .map_err(|e| AstrBotError::Plugin {
                plugin: "installer".to_string(),
                message: format!("Failed to read pip registry: {}", e),
            })?;
        let list: Vec<PipInstallRecord> = serde_json::from_str(&content)
            .map_err(|e| AstrBotError::Serialization(format!("Invalid pip registry: {}", e)))?;
        Ok(list)
    }

    pub async fn is_pip_installed(&self, package: &str) -> bool {
        match self.list_installed_pip_packages().await {
            Ok(list) => list.iter().any(|r| r.package == package),
            Err(_) => false,
        }
    }

    async fn query_pip_version(&self, package: &str) -> Result<String> {
        let output = tokio::process::Command::new("pip3")
            .args(["show", package])
            .output()
            .await
            .map_err(|e| AstrBotError::Plugin {
                plugin: "installer".to_string(),
                message: format!("pip show failed: {}", e),
            })?;
        if !output.status.success() {
            return Err(AstrBotError::NotFound(format!(
                "Package {} not found in pip",
                package
            )));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Some(v) = line.strip_prefix("Version: ") {
                return Ok(v.trim().to_string());
            }
        }
        Err(AstrBotError::Internal(format!(
            "Could not parse version for {}",
            package
        )))
    }

    fn pip_registry_path(&self) -> PathBuf {
        self.plugin_dir.join(".pip_registry.json")
    }

    pub async fn save_pip_registry(&self, record: &PipInstallRecord) -> Result<()> {
        let mut list = self.list_installed_pip_packages().await?;
        list.retain(|r| r.package != record.package);
        list.push(record.clone());
        self.write_pip_registry(&list).await
    }

    pub async fn remove_from_pip_registry(&self, package: &str) -> Result<()> {
        let mut list = self.list_installed_pip_packages().await?;
        list.retain(|r| r.package != package);
        self.write_pip_registry(&list).await
    }

    async fn write_pip_registry(&self, list: &[PipInstallRecord]) -> Result<()> {
        tokio::fs::create_dir_all(&self.plugin_dir)
            .await
            .map_err(|e| AstrBotError::Plugin {
                plugin: "installer".to_string(),
                message: format!("Failed to create plugin dir: {}", e),
            })?;
        let json = serde_json::to_string_pretty(list).map_err(|e| {
            AstrBotError::Serialization(format!("Failed to serialize pip registry: {}", e))
        })?;
        tokio::fs::write(self.pip_registry_path(), json)
            .await
            .map_err(|e| AstrBotError::Plugin {
                plugin: "installer".to_string(),
                message: format!("Failed to write pip registry: {}", e),
            })?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipInstallRecord {
    pub package: String,
    pub version: String,
    pub installed_at: u64,
}

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
        let src = tmp.join("source_plugin");
        tokio::fs::create_dir_all(&src).await.unwrap();
        tokio::fs::write(src.join("main.rs"), "// plugin code")
            .await
            .unwrap();
        let meta = installer
            .install_from_path(&src, "test_plugin")
            .await
            .unwrap();
        assert_eq!(meta.name, "test_plugin");
        assert!(installer.is_installed("test_plugin").await);
        let list = installer.list_installed().await.unwrap();
        assert!(list.contains(&"test_plugin".to_string()));
        installer.uninstall("test_plugin").await.unwrap();
        assert!(!installer.is_installed("test_plugin").await);
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

    #[tokio::test]
    async fn test_pip_registry_save_and_list() {
        let tmp = std::env::temp_dir().join(format!("astrbot_pip_reg_{}", std::process::id()));
        let installer = PluginInstaller::new(tmp.clone());
        let r1 = PipInstallRecord {
            package: "requests".into(),
            version: "2.31.0".into(),
            installed_at: 1000,
        };
        let r2 = PipInstallRecord {
            package: "numpy".into(),
            version: "1.24.0".into(),
            installed_at: 2000,
        };
        installer.save_pip_registry(&r1).await.unwrap();
        installer.save_pip_registry(&r2).await.unwrap();
        let list = installer.list_installed_pip_packages().await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list
            .iter()
            .any(|r| r.package == "requests" && r.version == "2.31.0"));
        let r3 = PipInstallRecord {
            package: "requests".into(),
            version: "2.32.0".into(),
            installed_at: 3000,
        };
        installer.save_pip_registry(&r3).await.unwrap();
        let list = installer.list_installed_pip_packages().await.unwrap();
        assert_eq!(list.len(), 2);
        let req = list.iter().find(|r| r.package == "requests").unwrap();
        assert_eq!(req.version, "2.32.0");
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    #[tokio::test]
    async fn test_pip_registry_remove() {
        let tmp = std::env::temp_dir().join(format!("astrbot_pip_rem_{}", std::process::id()));
        let installer = PluginInstaller::new(tmp.clone());
        let r1 = PipInstallRecord {
            package: "a".into(),
            version: "1.0.0".into(),
            installed_at: 1,
        };
        let r2 = PipInstallRecord {
            package: "b".into(),
            version: "2.0.0".into(),
            installed_at: 2,
        };
        installer.save_pip_registry(&r1).await.unwrap();
        installer.save_pip_registry(&r2).await.unwrap();
        installer.remove_from_pip_registry("a").await.unwrap();
        let list = installer.list_installed_pip_packages().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].package, "b");
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    #[tokio::test]
    async fn test_is_pip_installed() {
        let tmp = std::env::temp_dir().join(format!("astrbot_pip_chk_{}", std::process::id()));
        let installer = PluginInstaller::new(tmp.clone());
        assert!(!installer.is_pip_installed("nonexistent").await);
        let r = PipInstallRecord {
            package: "flask".into(),
            version: "3.0.0".into(),
            installed_at: 1,
        };
        installer.save_pip_registry(&r).await.unwrap();
        assert!(installer.is_pip_installed("flask").await);
        assert!(!installer.is_pip_installed("django").await);
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }
}
