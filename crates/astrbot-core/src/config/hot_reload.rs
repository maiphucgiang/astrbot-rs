use crate::config::AstrBotConfig;
use crate::errors::{AstrBotError, Result};
use notify::{Config as NotifyConfig, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};

/// Config change event
#[derive(Debug, Clone)]
pub enum ConfigChange {
    /// Full config reloaded
    Reloaded(AstrBotConfig),
    /// Reload failed with error message
    Error(String),
}

/// Watches config file for changes and reloads automatically
pub struct ConfigWatcher {
    config_path: String,
    current_config: Arc<RwLock<AstrBotConfig>>,
    change_tx: mpsc::Sender<ConfigChange>,
    _watcher: RecommendedWatcher,
}

impl ConfigWatcher {
    /// Start watching a config file
    pub async fn start<P: AsRef<Path>>(
        path: P,
        initial_config: AstrBotConfig,
    ) -> Result<(Self, mpsc::Receiver<ConfigChange>)> {
        let path = path.as_ref();
        let config_path = path.to_string_lossy().to_string();
        let current_config = Arc::new(RwLock::new(initial_config));
        let (change_tx, change_rx) = mpsc::channel(8);

        let tx = change_tx.clone();
        let config_arc = Arc::clone(&current_config);
        let watch_path = config_path.clone();

        let mut watcher = RecommendedWatcher::new(
            move |result: std::result::Result<Event, notify::Error>| {
                let tx = tx.clone();
                let config_arc = Arc::clone(&config_arc);
                let watch_path = watch_path.clone();

                tokio::spawn(async move {
                    match result {
                        Ok(event) => {
                            if event.kind.is_modify() {
                                info!("Config file changed, reloading: {}", watch_path);
                                match AstrBotConfig::from_file(&watch_path).await {
                                    Ok(new_config) => {
                                        let mut guard = config_arc.write().await;
                                        *guard = new_config.clone();
                                        let _ = tx.send(ConfigChange::Reloaded(new_config)).await;
                                    }
                                    Err(e) => {
                                        warn!("Config reload failed: {}", e);
                                        let _ = tx.send(ConfigChange::Error(e.to_string())).await;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!("Config watcher error: {}", e);
                        }
                    }
                });
            },
            NotifyConfig::default(),
        )
        .map_err(|e| AstrBotError::Config(format!("Failed to create file watcher: {}", e)))?;

        watcher
            .watch(path, RecursiveMode::NonRecursive)
            .map_err(|e| AstrBotError::Config(format!("Failed to watch config file: {}", e)))?;

        Ok((
            Self {
                config_path,
                current_config,
                change_tx,
                _watcher: watcher,
            },
            change_rx,
        ))
    }

    /// Get current config (cloned)
    pub async fn current_config(&self) -> AstrBotConfig {
        self.current_config.read().await.clone()
    }

    /// Trigger manual reload
    pub async fn reload(&self) -> Result<AstrBotConfig> {
        let new_config = AstrBotConfig::from_file(&self.config_path).await?;
        let mut guard = self.current_config.write().await;
        *guard = new_config.clone();
        let _ = self
            .change_tx
            .send(ConfigChange::Reloaded(new_config.clone()))
            .await;
        Ok(new_config)
    }
}

/// Config reload callback trait
#[async_trait::async_trait]
pub trait ConfigReloadCallback: Send + Sync {
    async fn on_config_reloaded(&self, config: &AstrBotConfig);
    async fn on_config_error(&self, error: &str);
}

/// Config reloader that dispatches changes to registered callbacks
pub struct ConfigReloader {
    change_rx: mpsc::Receiver<ConfigChange>,
    callbacks: Vec<Box<dyn ConfigReloadCallback>>,
}

impl ConfigReloader {
    pub fn new(change_rx: mpsc::Receiver<ConfigChange>) -> Self {
        Self {
            change_rx,
            callbacks: Vec::new(),
        }
    }

    pub fn add_callback(mut self, callback: Box<dyn ConfigReloadCallback>) -> Self {
        self.callbacks.push(callback);
        self
    }

    /// Run the reloader loop (blocks until channel closed)
    pub async fn run(mut self) {
        while let Some(change) = self.change_rx.recv().await {
            match change {
                ConfigChange::Reloaded(config) => {
                    for cb in &self.callbacks {
                        cb.on_config_reloaded(&config).await;
                    }
                }
                ConfigChange::Error(err) => {
                    for cb in &self.callbacks {
                        cb.on_config_error(&err).await;
                    }
                }
            }
        }
    }
}
