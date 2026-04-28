//! Functional plugin API — register handlers without implementing full Plugin trait
//!
//! This is the Rust equivalent of Python's `@register` decorator.
//!
//! Example:
//! ```rust,ignore
//! use astrbot_plugin::FunctionalPluginBuilder;
//! use astrbot_core::message::MessageEventResult;
//!
//! let plugin = FunctionalPluginBuilder::new("echo", "AstrBot Team", "Echo plugin")
//!     .on_command("echo", "Echo back your message", false, |args, _source, _user_id| {
//!         let text = args.join(" ");
//!         Box::pin(async move {
//!             Ok(MessageEventResult::reply_text(text))
//!         })
//!     })
//!     .build();
//! ```

use async_trait::async_trait;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use astrbot_core::errors::Result;
use astrbot_core::event::{Event, EventResult};
use astrbot_core::message::{MessageChain, MessageEventResult};
use astrbot_core::platform::MessageSource;
use astrbot_core::plugin::{Plugin, PluginConfig, PluginContext, PluginMetadata, PluginConfigSchema};

/// Async command handler trait — implement this to handle plugin commands
#[async_trait]
pub trait AsyncCommandHandler: Send + Sync {
    async fn handle(
        &self,
        args: &[String],
        source: &MessageSource,
        user_id: &str,
    ) -> Result<MessageEventResult>;
}

/// Async event handler trait — implement this to handle plugin events
#[async_trait]
pub trait AsyncEventHandler: Send + Sync {
    async fn handle(&self, event: &dyn Event) -> Result<EventResult>;
}

/// Async init hook trait
#[async_trait]
pub trait AsyncInitHook: Send + Sync {
    async fn call(&self, config: PluginConfig, ctx: PluginContext) -> Result<()>;
}

/// Async lifecycle hook trait (start/stop)
#[async_trait]
pub trait AsyncLifecycleHook: Send + Sync {
    async fn call(&self) -> Result<()>;
}

/// Builder for creating functional plugins (no need to implement Plugin trait manually)
pub struct FunctionalPluginBuilder {
    metadata: PluginMetadata,
    config_schema: Vec<PluginConfigSchema>,
    commands: Vec<(String, String, bool, Arc<dyn AsyncCommandHandler>)>,
    event_handlers: Vec<(String, Arc<dyn AsyncEventHandler>)>,
    init_hook: Option<Arc<dyn AsyncInitHook>>,
    start_hook: Option<Arc<dyn AsyncLifecycleHook>>,
    stop_hook: Option<Arc<dyn AsyncLifecycleHook>>,
}

impl FunctionalPluginBuilder {
    /// Create a new builder with required metadata
    pub fn new(
        name: impl Into<String>,
        author: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            metadata: PluginMetadata {
                name: name.into(),
                author: author.into(),
                description: description.into(),
                version: "0.1.0".to_string(),
                repository: None,
                min_astrbot_version: None,
                platforms: vec![],
                reserved: false,
                logo: None,
            },
            config_schema: vec![],
            commands: vec![],
            event_handlers: vec![],
            init_hook: None,
            start_hook: None,
            stop_hook: None,
        }
    }

    /// Set version
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.metadata.version = version.into();
        self
    }

    /// Add a config schema entry
    pub fn config_schema(mut self, key: impl Into<String>, value_type: impl Into<String>, description: impl Into<String>, required: bool) -> Self {
        self.config_schema.push(PluginConfigSchema {
            key: key.into(),
            value_type: value_type.into(),
            default: None,
            description: description.into(),
            required,
        });
        self
    }

    /// Register a command handler using a closure
    pub fn on_command<F, Fut>(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        admin_only: bool,
        handler: F,
    ) -> Self
    where
        F: Fn(&[String], &MessageSource, &str) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<MessageEventResult>> + Send + 'static,
    {
        let name = name.into();
        let description = description.into();
        let boxed = Arc::new(ClosureCommandHandler { f: handler });
        self.commands.push((name, description, admin_only, boxed));
        self
    }

    /// Register an event handler for a specific event type
    pub fn on_event<F, Fut>(
        mut self,
        event_type: impl Into<String>,
        handler: F,
    ) -> Self
    where
        F: Fn(&dyn Event) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<EventResult>> + Send + 'static,
    {
        let boxed = Arc::new(ClosureEventHandler { f: handler });
        self.event_handlers.push((event_type.into(), boxed));
        self
    }

    /// Set initialization hook
    pub fn on_init<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(PluginConfig, PluginContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.init_hook = Some(Arc::new(ClosureInitHook { f: hook }));
        self
    }

    /// Set start hook
    pub fn on_start<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.start_hook = Some(Arc::new(ClosureLifecycleHook { f: hook }));
        self
    }

    /// Set stop hook
    pub fn on_stop<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.stop_hook = Some(Arc::new(ClosureLifecycleHook { f: hook }));
        self
    }

    /// Build the plugin
    pub fn build(self) -> FunctionalPlugin {
        FunctionalPlugin {
            metadata: self.metadata,
            config_schema: self.config_schema,
            commands: self.commands,
            event_handlers: self.event_handlers,
            init_hook: self.init_hook,
            start_hook: self.start_hook,
            stop_hook: self.stop_hook,
            ctx: None,
        }
    }
}

// --- Closure wrapper structs to implement the async traits ---

struct ClosureCommandHandler<F> {
    f: F,
}

#[async_trait]
impl<F, Fut> AsyncCommandHandler for ClosureCommandHandler<F>
where
    F: Fn(&[String], &MessageSource, &str) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<MessageEventResult>> + Send + 'static,
{
    async fn handle(
        &self,
        args: &[String],
        source: &MessageSource,
        user_id: &str,
    ) -> Result<MessageEventResult> {
        (self.f)(args, source, user_id).await
    }
}

struct ClosureEventHandler<F> {
    f: F,
}

#[async_trait]
impl<F, Fut> AsyncEventHandler for ClosureEventHandler<F>
where
    F: Fn(&dyn Event) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<EventResult>> + Send + 'static,
{
    async fn handle(&self, event: &dyn Event) -> Result<EventResult> {
        (self.f)(event).await
    }
}

struct ClosureInitHook<F> {
    f: F,
}

#[async_trait]
impl<F, Fut> AsyncInitHook for ClosureInitHook<F>
where
    F: Fn(PluginConfig, PluginContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    async fn call(&self, config: PluginConfig, ctx: PluginContext) -> Result<()> {
        (self.f)(config, ctx).await
    }
}

struct ClosureLifecycleHook<F> {
    f: F,
}

#[async_trait]
impl<F, Fut> AsyncLifecycleHook for ClosureLifecycleHook<F>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    async fn call(&self) -> Result<()> {
        (self.f)().await
    }
}

/// A plugin built from closures — no need to implement Plugin trait manually
pub struct FunctionalPlugin {
    metadata: PluginMetadata,
    config_schema: Vec<PluginConfigSchema>,
    commands: Vec<(String, String, bool, Arc<dyn AsyncCommandHandler>)>,
    event_handlers: Vec<(String, Arc<dyn AsyncEventHandler>)>,
    init_hook: Option<Arc<dyn AsyncInitHook>>,
    start_hook: Option<Arc<dyn AsyncLifecycleHook>>,
    stop_hook: Option<Arc<dyn AsyncLifecycleHook>>,
    ctx: Option<PluginContext>,
}

#[async_trait]
impl Plugin for FunctionalPlugin {
    fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    fn config_schema(&self) -> Vec<PluginConfigSchema> {
        self.config_schema.clone()
    }

    async fn initialize(
        &mut self, config: PluginConfig, ctx: PluginContext) -> Result<()> {
        self.ctx = Some(ctx.clone());
        if let Some(ref hook) = self.init_hook {
            hook.call(config, ctx).await?;
        }
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        if let Some(ref hook) = self.start_hook {
            hook.call().await?;
        }
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(ref hook) = self.stop_hook {
            hook.call().await?;
        }
        Ok(())
    }

    async fn on_event(&self, event: &dyn Event) -> Result<EventResult> {
        for (event_type, handler) in &self.event_handlers {
            if event.event_type() == event_type.as_str() {
                return handler.handle(event).await;
            }
        }
        Ok(EventResult::nothing())
    }

    fn can_handle(&self, event: &dyn Event) -> bool {
        self.event_handlers.iter().any(|(et, _)| et == event.event_type())
    }

    fn commands(&self) -> Vec<(String, String, bool)> {
        self.commands
            .iter()
            .map(|(name, desc, admin, _)| (name.clone(), desc.clone(), *admin))
            .collect()
    }

    async fn on_command(
        &self,
        command: &str,
        args: &[String],
        source: &MessageSource,
        user_id: &str,
    ) -> Result<MessageEventResult> {
        for (name, _, _, handler) in &self.commands {
            if name == command {
                return handler.handle(args, source, user_id).await;
            }
        }
        Err(astrbot_core::errors::AstrBotError::NotFound(format!(
            "Command '/{}' not found in plugin '{}'",
            command, self.metadata.name
        )))
    }
}
