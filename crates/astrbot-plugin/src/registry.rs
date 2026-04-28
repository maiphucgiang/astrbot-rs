use async_trait::async_trait;
use std::collections::HashMap;

use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::message::MessageEventResult;
use astrbot_core::platform::MessageSource;
use astrbot_core::plugin::{Plugin, PluginContext, Star};
use tracing::{error, info, warn};

/// Command handler trait — plugins implement this to handle custom commands
#[async_trait]
pub trait CommandHandler: Send + Sync {
    /// Handle the command
    async fn handle(
        &self,
        args: &[String],
        source: &MessageSource,
        user_id: &str,
    ) -> Result<MessageEventResult>;
    /// Get command description for /help
    fn description(&self) -> String;
    /// Whether this command is admin-only
    fn admin_only(&self) -> bool {
        false
    }
}

/// Entry for a registered command
pub struct CommandEntry {
    pub plugin_name: String,
    pub description: String,
    pub admin_only: bool,
    pub handler: std::sync::Arc<dyn CommandHandler>,
}

/// Registry for plugin-registered commands
pub struct CommandRegistry {
    commands: HashMap<String, CommandEntry>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
        }
    }

    /// Register a command
    pub fn register(
        &mut self,
        name: impl Into<String>,
        plugin_name: impl Into<String>,
        description: impl Into<String>,
        admin_only: bool,
        handler: std::sync::Arc<dyn CommandHandler>,
    ) {
        let name = name.into();
        if self.commands.contains_key(&name) {
            warn!("Command '/{}' is already registered, overwriting", name);
        }
        self.commands.insert(
            name.clone(),
            CommandEntry {
                plugin_name: plugin_name.into(),
                description: description.into(),
                admin_only,
                handler,
            },
        );
        info!("Registered command '/{}'", name);
    }

    /// Unregister all commands from a plugin
    pub fn unregister_by_plugin(&mut self,
        plugin_name: &str,
    ) {
        let to_remove: Vec<String> = self
            .commands
            .iter()
            .filter(|(_, e)| e.plugin_name == plugin_name)
            .map(|(k, _)| k.clone())
            .collect();
        for name in to_remove {
            self.commands.remove(&name);
            info!("Unregistered command '/{}' (plugin '{}')", name, plugin_name);
        }
    }

    /// Check if a command exists
    pub fn has(&self, name: &str) -> bool {
        self.commands.contains_key(name)
    }

    /// Get command entry
    pub fn get(&self, name: &str) -> Option<&CommandEntry> {
        self.commands.get(name)
    }

    /// Get all registered commands (for /help)
    pub fn list(&self) -> Vec<(&String, &CommandEntry)> {
        self.commands.iter().collect()
    }

    /// Handle a command
    pub async fn handle(
        &self,
        name: &str,
        args: &[String],
        source: &MessageSource,
        user_id: &str,
        is_admin: bool,
    ) -> Option<Result<MessageEventResult>> {
        let entry = self.commands.get(name)?;

        if entry.admin_only && !is_admin {
            return Some(Ok(MessageEventResult::reply_text(
                "⛔ This command is admin only.".to_string()
            )));
        }

        Some(entry.handler.handle(args, source, user_id).await)
    }
}

/// Plugin registry — manages loaded plugins with command support
pub struct PluginRegistry {
    plugins: HashMap<String, Star>,
    command_registry: CommandRegistry,
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            command_registry: CommandRegistry::new(),
        }
    }

    /// Get the command registry
    pub fn commands(&self) -> &CommandRegistry {
        &self.command_registry
    }

    /// Get mutable command registry
    pub fn commands_mut(&mut self) -> &mut CommandRegistry {
        &mut self.command_registry
    }

    /// Register a plugin star (without implementation)
    pub fn register(&mut self, star: Star) {
        self.plugins.insert(star.metadata.name.clone(), star);
    }

    /// Register a plugin with its implementation
    pub fn register_plugin(&mut self, mut star: Star, plugin: Box<dyn Plugin>) {
        // Register commands from the plugin
        for (cmd, desc, admin_only) in plugin.commands() {
            let plugin_name = star.metadata.name.clone();
            let handler = std::sync::Arc::new(PluginCommandHandler {
                plugin_name: plugin_name.clone(),
            });
            self.command_registry.register(
                cmd,
                plugin_name,
                desc,
                admin_only,
                handler,
            );
        }

        star.plugin = Some(plugin);
        self.plugins.insert(star.metadata.name.clone(), star);
    }

    /// Get a plugin by name
    pub fn get(&self, name: &str) -> Option<&Star> {
        self.plugins.get(name)
    }

    /// Get a mutable plugin by name
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Star> {
        self.plugins.get_mut(name)
    }

    /// List all registered plugins
    pub fn list(&self) -> Vec<&Star> {
        self.plugins.values().collect()
    }

    /// List activated plugins
    pub fn activated_plugins(&self) -> Vec<&Star> {
        self.plugins.values().filter(|p| p.activated).collect()
    }

    /// Activate a plugin (initialize + start + register commands)
    pub async fn activate(
        &mut self,
        name: &str,
        ctx: PluginContext,
    ) -> Result<()> {
        let star = self.plugins.get_mut(name)
            .ok_or_else(|| AstrBotError::NotFound(format!("plugin: {}", name)))?;

        if star.activated {
            warn!("Plugin '{}' is already activated", name);
            return Ok(());
        }

        if let Some(ref mut plugin) = star.plugin {
            let config = star.config.clone();
            plugin.initialize(config, ctx).await?;
            plugin.start().await?;
            star.activated = true;
            info!("Plugin '{}' activated", name);
        } else {
            return Err(AstrBotError::Plugin {
                plugin: name.to_string(),
                message: "plugin implementation not loaded".to_string(),
            });
        }

        Ok(())
    }

    /// Deactivate a plugin (stop + unregister commands)
    pub async fn deactivate(&mut self, name: &str) -> Result<()> {
        let star = self.plugins.get_mut(name)
            .ok_or_else(|| AstrBotError::NotFound(format!("plugin: {}", name)))?;

        if !star.activated {
            return Ok(());
        }

        // Unregister commands
        self.command_registry.unregister_by_plugin(name);

        if let Some(ref mut plugin) = star.plugin {
            plugin.stop().await?;
            star.activated = false;
            info!("Plugin '{}' deactivated", name);
        }

        Ok(())
    }

    /// Dispatch an event to all activated plugins
    pub async fn dispatch_event(
        &self,
        event: &dyn astrbot_core::event::Event,
    ) -> Vec<Result<astrbot_core::event::EventResult>> {
        let mut results = Vec::new();

        for star in self.plugins.values() {
            if !star.activated {
                continue;
            }

            if let Some(ref plugin) = star.plugin {
                if plugin.can_handle(event) {
                    let result = plugin.on_event(event).await;
                    results.push(result);
                }
            }
        }

        results
    }

    /// Dispatch a command to the plugin that owns it
    pub async fn dispatch_command(
        &self,
        cmd: &str,
        args: &[String],
        source: &MessageSource,
        user_id: &str,
        is_admin: bool,
    ) -> Option<Result<MessageEventResult>> {
        for star in self.plugins.values() {
            if !star.activated {
                continue;
            }
            if let Some(ref plugin) = star.plugin {
                let commands = plugin.commands();
                if let Some((_, _, admin_only)) = commands.iter().find(|(name, _, _)| name == cmd) {
                    if *admin_only && !is_admin {
                        return Some(Ok(MessageEventResult::reply_text(
                            "⛔ This command is admin only.".to_string()
                        )));
                    }
                    return Some(plugin.on_command(cmd, args, source, user_id).await);
                }
            }
        }
        None
    }

    /// Remove a plugin
    pub async fn remove(&mut self,
        name: &str,
    ) -> Result<()> {
        if let Some(star) = self.plugins.get(name) {
            if star.activated {
                self.deactivate(name).await?;
            }
        }
        self.plugins.remove(name);
        Ok(())
    }
}

/// Internal command handler that delegates to the plugin
struct PluginCommandHandler {
    plugin_name: String,
}

#[async_trait]
impl CommandHandler for PluginCommandHandler {
    async fn handle(
        &self,
        _args: &[String],
        _source: &MessageSource,
        _user_id: &str,
    ) -> Result<MessageEventResult> {
        Err(AstrBotError::Internal(
            "Plugin command dispatch goes through PluginRegistry.dispatch_command() — CommandRegistry is metadata-only".to_string()
        ))
    }

    fn description(&self) -> String {
        "(plugin command — dispatched via PluginRegistry)".to_string()
    }
}

/// Plugin manifest (loaded from plugin.json)
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub author: String,
    pub description: String,
    pub version: String,
    pub repository: Option<String>,
    pub min_astrbot_version: Option<String>,
    pub platforms: Vec<String>,
    pub reserved: Option<bool>,
    pub logo: Option<String>,
    pub config_schema: Option<Vec<astrbot_core::plugin::PluginConfigSchema>>,
    pub main: String,
}

/// Trait for plugin loaders
#[async_trait]
pub trait PluginLoader: Send + Sync {
    /// Load plugins from a directory
    async fn load_from_directory(&self,
        path: &std::path::Path,
    ) -> Result<Vec<Star>>;
    /// Loader name
    fn name(&self) -> &'static str;
}

/// File-based plugin loader — loads from plugin directories
pub struct FilePluginLoader;

impl FilePluginLoader {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl PluginLoader for FilePluginLoader {
    async fn load_from_directory(
        &self,
        path: &std::path::Path,
    ) -> Result<Vec<Star>> {
        let mut stars = Vec::new();

        if !path.exists() {
            warn!("Plugin directory '{}' does not exist", path.display());
            return Ok(stars);
        }

        let mut entries = tokio::fs::read_dir(path).await.map_err(|e| {
            AstrBotError::Internal(format!("failed to read plugin directory: {}", e))
        })?;

        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            AstrBotError::Internal(format!("failed to read directory entry: {}", e))
        })? {
            let plugin_dir = entry.path();
            if !plugin_dir.is_dir() {
                continue;
            }

            let manifest_path = plugin_dir.join("plugin.json");
            if !manifest_path.exists() {
                warn!("Skipping '{}': no plugin.json found", plugin_dir.display());
                continue;
            }

            let manifest_content = tokio::fs::read_to_string(&manifest_path).await.map_err(|e| {
                AstrBotError::Internal(format!("failed to read plugin.json: {}", e))
            })?;

            let manifest: PluginManifest = serde_json::from_str(&manifest_content).map_err(|e| {
                AstrBotError::Plugin {
                    plugin: plugin_dir
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    message: format!("invalid plugin.json: {}", e),
                }
            })?;

            let metadata = astrbot_core::plugin::PluginMetadata {
                name: manifest.name,
                author: manifest.author,
                description: manifest.description,
                version: manifest.version,
                repository: manifest.repository,
                min_astrbot_version: manifest.min_astrbot_version,
                platforms: manifest.platforms,
                reserved: manifest.reserved.unwrap_or(false),
                logo: manifest.logo,
            };

            let star = Star::new(metadata);
            let name = star.metadata.name.clone();
            stars.push(star);
            info!("Loaded plugin manifest: '{}' from {}", name, plugin_dir.display());
        }

        Ok(stars)
    }

    fn name(&self) -> &'static str {
        "file"
    }
}

/// Plugin manager — orchestrates loaders and registry
pub struct PluginManager {
    registry: PluginRegistry,
    loaders: Vec<Box<dyn PluginLoader>>,
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            registry: PluginRegistry::new(),
            loaders: Vec::new(),
        }
    }

    /// Register a loader
    pub fn register_loader(&mut self,
        loader: Box<dyn PluginLoader>,
    ) {
        self.loaders.push(loader);
    }

    /// Load plugins from all loaders
    pub async fn load_all(
        &mut self,
        plugin_dirs: &[&std::path::Path],
    ) -> Result<()> {
        for dir in plugin_dirs {
            for loader in &self.loaders {
                match loader.load_from_directory(dir).await {
                    Ok(stars) => {
                        for star in stars {
                            self.registry.register(star);
                        }
                    }
                    Err(e) => {
                        error!(
                            "Loader '{}' failed to load from {}: {}",
                            loader.name(),
                            dir.display(),
                            e
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Get the registry
    pub fn registry(&self) -> &PluginRegistry {
        &self.registry
    }

    /// Get mutable registry
    pub fn registry_mut(&mut self) -> &mut PluginRegistry {
        &mut self.registry
    }

    /// Activate all plugins
    pub async fn activate_all(
        &mut self,
        ctx: PluginContext,
    ) -> Result<()> {
        let names: Vec<String> = self
            .registry
            .list()
            .iter()
            .map(|s| s.metadata.name.clone())
            .collect();

        for name in names {
            if let Err(e) = self.registry.activate(&name, ctx.clone()).await {
                error!("Failed to activate plugin '{}': {}", name, e);
            }
        }
        Ok(())
    }

    /// Deactivate all plugins
    pub async fn deactivate_all(&mut self,
    ) -> Result<()> {
        let names: Vec<String> = self
            .registry
            .list()
            .iter()
            .map(|s| s.metadata.name.clone())
            .collect();

        for name in names {
            if let Err(e) = self.registry.deactivate(&name).await {
                error!("Failed to deactivate plugin '{}': {}", name, e);
            }
        }
        Ok(())
    }
}

// Implement PluginCommandResolver for PluginRegistry
use astrbot_core::pipeline::PluginCommandResolver;

#[async_trait]
impl PluginCommandResolver for PluginRegistry {
    async fn resolve(
        &self,
        cmd: &str,
        args: &[String],
        source: &MessageSource,
        user_id: &str,
        is_admin: bool,
    ) -> Option<Result<MessageEventResult>> {
        self.dispatch_command(cmd, args, source, user_id, is_admin).await
    }

    fn command_list(&self) -> Vec<(String, String, bool)> {
        self.command_registry
            .list()
            .into_iter()
            .map(|(name, entry)| (name.clone(), entry.description.clone(), entry.admin_only))
            .collect()
    }
}
