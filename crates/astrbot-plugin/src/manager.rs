use crate::installer::PluginInstaller;
use crate::loader::{PluginLoader, StarDescriptor};
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::event::{Event, EventResult};
use astrbot_core::message::MessageEventResult;
use astrbot_core::plugin::{PluginConfig, PluginContext, PluginLifecycle, PluginMetadata, Star};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info};

/// A loaded plugin with lifecycle tracking.
#[derive(Debug)]
pub struct LoadedStar {
    pub star: Star,
    pub lifecycle: PluginLifecycle,
    pub loaded_at: u64,
    pub last_error: Option<String>,
}

/// Summary info for external queries.
#[derive(Debug, Clone, Serialize)]
pub struct StarSummary {
    pub name: String,
    pub version: String,
    pub lifecycle: PluginLifecycle,
    pub loaded_at: u64,
    pub activated: bool,
}

/// Plugin manager — orchestrates the full lifecycle from install to uninstall.
pub struct PluginManager {
    loader: PluginLoader,
    installer: PluginInstaller,
    /// instance_id (name) → LoadedStar
    stars: DashMap<String, LoadedStar>,
    /// Factory for creating PluginContext (set by framework)
    ctx_factory: Option<Arc<dyn Fn() -> PluginContext + Send + Sync>>,
}

impl PluginManager {
    pub fn new(plugin_dir: std::path::PathBuf) -> Self {
        Self {
            loader: PluginLoader::new(plugin_dir.clone()),
            installer: PluginInstaller::new(plugin_dir),
            stars: DashMap::new(),
            ctx_factory: None,
        }
    }

    pub fn set_ctx_factory(&mut self, factory: Arc<dyn Fn() -> PluginContext + Send + Sync>) {
        self.ctx_factory = Some(factory);
    }

    pub async fn install(&self, path: &Path) -> Result<String> {
        let meta = self.installer.install_from_path(path, "auto").await?;
        let name = meta.name.clone();
        info!("[PluginManager] installed '{}'", name);
        Ok(name)
    }

    pub async fn load(&self, name: &str) -> Result<()> {
        let desc = self.read_descriptor(name).await?;
        let star = self.loader.instantiate(&desc).await?;

        let loaded = LoadedStar {
            star,
            lifecycle: PluginLifecycle::Loaded,
            loaded_at: now_secs(),
            last_error: None,
        };
        self.stars.insert(name.to_string(), loaded);
        info!("[PluginManager] loaded '{}'", name);
        Ok(())
    }

    pub async fn initialize(&self, name: &str) -> Result<()> {
        let mut entry = self.stars
            .get_mut(name)
            .ok_or_else(|| AstrBotError::NotFound(format!("Plugin '{}' not loaded", name)))?;

        if entry.lifecycle != PluginLifecycle::Loaded {
            return Err(AstrBotError::Validation(format!(
                "Plugin '{}' cannot initialize from state {:?}",
                name, entry.lifecycle
            )));
        }

        let config = entry.star.config.clone();
        if let Some(ref mut plugin) = entry.star.plugin {
            let ctx = self.mk_context();
            plugin.initialize(config, ctx).await.map_err(|e| {
                entry.lifecycle = PluginLifecycle::Error;
                entry.last_error = Some(format!("initialize failed: {}", e));
                e
            })?;
        }

        entry.lifecycle = PluginLifecycle::Initialized;
        info!("[PluginManager] initialized '{}'", name);
        Ok(())
    }

    pub async fn start(&self, name: &str) -> Result<()> {
        let mut entry = self.stars
            .get_mut(name)
            .ok_or_else(|| AstrBotError::NotFound(format!("Plugin '{}' not loaded", name)))?;

        if entry.lifecycle != PluginLifecycle::Initialized {
            return Err(AstrBotError::Validation(format!(
                "Plugin '{}' cannot start from state {:?}",
                name, entry.lifecycle
            )));
        }

        if let Some(ref mut plugin) = entry.star.plugin {
            plugin.start().await.map_err(|e| {
                entry.lifecycle = PluginLifecycle::Error;
                entry.last_error = Some(format!("start failed: {}", e));
                e
            })?;
        }

        entry.lifecycle = PluginLifecycle::Running;
        entry.star.activated = true;
        info!("[PluginManager] started '{}'", name);
        Ok(())
    }

    pub async fn enable(&self, name: &str) -> Result<()> {
        if !self.stars.contains_key(name) {
            self.load(name).await?;
        }
        self.initialize(name).await?;
        self.start(name).await?;
        Ok(())
    }

    pub async fn stop(&self, name: &str) -> Result<()> {
        let mut entry = self.stars
            .get_mut(name)
            .ok_or_else(|| AstrBotError::NotFound(format!("Plugin '{}' not loaded", name)))?;

        if entry.lifecycle != PluginLifecycle::Running {
            return Err(AstrBotError::Validation(format!(
                "Plugin '{}' cannot stop from state {:?}",
                name, entry.lifecycle
            )));
        }

        if let Some(ref mut plugin) = entry.star.plugin {
            plugin.stop().await.map_err(|e| {
                entry.last_error = Some(format!("stop failed: {}", e));
                e
            })?;
        }

        entry.lifecycle = PluginLifecycle::Stopped;
        entry.star.activated = false;
        info!("[PluginManager] stopped '{}'", name);
        Ok(())
    }

    pub async fn unload(&self, name: &str) -> Result<()> {
        let mut entry = self.stars
            .get_mut(name)
            .ok_or_else(|| AstrBotError::NotFound(format!("Plugin '{}' not loaded", name)))?;

        if entry.lifecycle == PluginLifecycle::Running {
            self.stop(name).await?;
        }

        entry.star.plugin = None;
        entry.lifecycle = PluginLifecycle::Unloaded;
        info!("[PluginManager] unloaded '{}'", name);
        Ok(())
    }

    pub async fn uninstall(&self, name: &str) -> Result<()> {
        if let Some(entry) = self.stars.get(name) {
            if entry.lifecycle == PluginLifecycle::Running {
                drop(entry);
                self.stop(name).await?;
            }
        }
        self.stars.remove(name);
        self.installer.uninstall(name).await?;
        info!("[PluginManager] uninstalled '{}'", name);
        Ok(())
    }

    pub async fn disable(&self, name: &str) -> Result<()> {
        if let Some(entry) = self.stars.get(name) {
            if entry.lifecycle == PluginLifecycle::Running {
                drop(entry);
                self.stop(name).await?;
            }
        }
        self.unload(name).await?;
        Ok(())
    }

    pub fn list(&self) -> Vec<StarSummary> {
        self.stars
            .iter()
            .map(|e| {
                let s = &e.value().star;
                StarSummary {
                    name: s.metadata.name.clone(),
                    version: s.metadata.version.clone(),
                    lifecycle: e.value().lifecycle,
                    loaded_at: e.value().loaded_at,
                    activated: s.activated,
                }
            })
            .collect()
    }

    pub fn get_lifecycle(&self, name: &str) -> Option<PluginLifecycle> {
        self.stars.get(name).map(|e| e.lifecycle)
    }

    pub fn is_running(&self, name: &str) -> bool {
        self.get_lifecycle(name) == Some(PluginLifecycle::Running)
    }

    pub async fn dispatch_event(&self, event: &dyn Event) -> Vec<EventResult> {
        let handlers: Vec<_> = self
            .stars
            .iter()
            .filter(|e| {
                e.lifecycle == PluginLifecycle::Running
                    && e.star.plugin.as_ref().map(|p| p.can_handle(event)).unwrap_or(false)
            })
            .map(|e| (e.key().clone(), e.value().star.plugin.is_some()))
            .collect();

        if handlers.is_empty() {
            return vec![];
        }

        let mut results = Vec::new();
        for (name, _) in handlers {
            if let Some(entry) = self.stars.get(&name) {
                if let Some(ref plugin) = entry.star.plugin {
                    match plugin.on_event(event).await {
                        Ok(r) => results.push(r),
                        Err(e) => {
                            error!("[PluginManager] plugin '{}' event handler error: {}", name, e);
                        }
                    }
                }
            }
        }
        results
    }

    async fn read_descriptor(&self, name: &str) -> Result<StarDescriptor> {
        let path = self.loader.plugin_dir().join(name);
        if !path.exists() {
            return Err(AstrBotError::NotFound(format!(
                "Plugin '{}' not found on disk",
                name
            )));
        }
        let meta = self.loader.read_metadata(name).await?;
        Ok(StarDescriptor {
            metadata: meta.clone(),
            path,
            name: meta.name,
        })
    }

    fn mk_context(&self) -> PluginContext {
        match &self.ctx_factory {
            Some(f) => f(),
            None => PluginContext::new("unknown".to_string(), "unknown".to_string()),
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use astrbot_core::event::EventResult;
    use astrbot_core::message::{MessageChain, MessageComponent};
    use astrbot_core::platform::MessageSource;
    use astrbot_core::plugin::Plugin;

    struct MockPlugin {
        meta: PluginMetadata,
        initialized: bool,
        started: bool,
        stopped: bool,
    }

    impl MockPlugin {
        fn new(name: &str) -> Self {
            Self {
                meta: PluginMetadata {
                    name: name.to_string(),
                    author: "test".to_string(),
                    description: "".to_string(),
                    version: "1.0.0".to_string(),
                    repository: None,
                    min_astrbot_version: None,
                    platforms: vec![],
                    reserved: false,
                    logo: None,
                },
                initialized: false,
                started: false,
                stopped: false,
            }
        }
    }

    #[async_trait::async_trait]
    impl Plugin for MockPlugin {
        fn metadata(&self) -> &PluginMetadata {
            &self.meta
        }

        async fn initialize(&mut self, _config: PluginConfig, _ctx: PluginContext) -> Result<()> {
            self.initialized = true;
            Ok(())
        }

        async fn start(&mut self) -> Result<()> {
            self.started = true;
            Ok(())
        }

        async fn stop(&mut self) -> Result<()> {
            self.stopped = true;
            Ok(())
        }

        async fn on_event(&self, _event: &dyn Event) -> Result<EventResult> {
            Ok(EventResult::nothing())
        }

        fn can_handle(&self, _event: &dyn Event) -> bool {
            true
        }
    }

    fn make_manager() -> (PluginManager, std::path::PathBuf) {
        let tmp = std::env::temp_dir().join(format!(
            "astrbot_mgr_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        (PluginManager::new(tmp.clone()), tmp)
    }

    fn write_plugin(dir: &Path, name: &str) {
        let p = dir.join(name);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("metadata.json"),
            format!(
                r#"{{"name":"{}","author":"t","description":"","version":"1.0.0","platforms":[],"reserved":false}}"#,
                name
            ),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn test_lifecycle_load_init_start() {
        let (mgr, tmp) = make_manager();
        write_plugin(&tmp, "lifecycle_test");

        mgr.load("lifecycle_test").await.unwrap();
        assert_eq!(mgr.get_lifecycle("lifecycle_test"), Some(PluginLifecycle::Loaded));

        mgr.initialize("lifecycle_test").await.unwrap();
        assert_eq!(mgr.get_lifecycle("lifecycle_test"), Some(PluginLifecycle::Initialized));

        mgr.start("lifecycle_test").await.unwrap();
        assert_eq!(mgr.get_lifecycle("lifecycle_test"), Some(PluginLifecycle::Running));

        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    #[tokio::test]
    async fn test_lifecycle_stop_unload() {
        let (mgr, tmp) = make_manager();
        write_plugin(&tmp, "stop_test");

        mgr.load("stop_test").await.unwrap();
        mgr.initialize("stop_test").await.unwrap();
        mgr.start("stop_test").await.unwrap();

        mgr.stop("stop_test").await.unwrap();
        assert_eq!(mgr.get_lifecycle("stop_test"), Some(PluginLifecycle::Stopped));

        mgr.unload("stop_test").await.unwrap();
        assert_eq!(mgr.get_lifecycle("stop_test"), Some(PluginLifecycle::Unloaded));

        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    #[tokio::test]
    async fn test_enable_disable() {
        let (mgr, tmp) = make_manager();
        write_plugin(&tmp, "toggle");

        mgr.enable("toggle").await.unwrap();
        assert_eq!(mgr.get_lifecycle("toggle"), Some(PluginLifecycle::Running));
        assert!(mgr.is_running("toggle"));

        mgr.disable("toggle").await.unwrap();
        assert_eq!(mgr.get_lifecycle("toggle"), Some(PluginLifecycle::Unloaded));
        assert!(!mgr.is_running("toggle"));

        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    #[tokio::test]
    async fn test_uninstall_removes_everything() {
        let (mgr, tmp) = make_manager();
        write_plugin(&tmp, "gone");

        mgr.enable("gone").await.unwrap();
        assert!(mgr.is_running("gone"));

        mgr.uninstall("gone").await.unwrap();
        assert!(mgr.get_lifecycle("gone").is_none());
        assert!(!tmp.join("gone").exists());

        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    #[tokio::test]
    async fn test_list_returns_snapshot() {
        let (mgr, tmp) = make_manager();
        write_plugin(&tmp, "p1");
        write_plugin(&tmp, "p2");

        mgr.load("p1").await.unwrap();
        mgr.load("p2").await.unwrap();

        let list = mgr.list();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|s| s.name == "p1"));
        assert!(list.iter().any(|s| s.name == "p2"));

        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    #[tokio::test]
    async fn test_dispatch_routes_to_running_only() {
        let (mgr, tmp) = make_manager();
        write_plugin(&tmp, "runner");
        write_plugin(&tmp, "idle");

        mgr.load("runner").await.unwrap();
        mgr.initialize("runner").await.unwrap();
        mgr.start("runner").await.unwrap();

        mgr.load("idle").await.unwrap();

        let list = mgr.list();
        let runner = list.iter().find(|s| s.name == "runner").unwrap();
        let idle = list.iter().find(|s| s.name == "idle").unwrap();
        assert_eq!(runner.lifecycle, PluginLifecycle::Running);
        assert_eq!(idle.lifecycle, PluginLifecycle::Loaded);

        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }
}
