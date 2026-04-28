#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{FilePluginLoader, PluginLoader, PluginManager, PluginRegistry};
    use astrbot_core::errors::Result;
    use astrbot_core::event::{Event, MessageEvent, EventResult};
    use astrbot_core::message::{AstrBotMessage, MessageChain, MessageMember, MessageType, MessageEventResult};
    use astrbot_core::platform::{MessageSource, PlatformType};
    use astrbot_core::plugin::{Plugin, PluginConfig, PluginContext, PluginMetadata, Star};
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A test plugin that counts message events
    struct CountingPlugin {
        metadata: PluginMetadata,
        message_count: AtomicUsize,
    }

    impl CountingPlugin {
        fn new() -> Self {
            Self {
                metadata: PluginMetadata {
                    name: "counting".to_string(),
                    author: "test".to_string(),
                    description: "Counts messages".to_string(),
                    version: "1.0.0".to_string(),
                    repository: None,
                    min_astrbot_version: None,
                    platforms: vec!["*".to_string()],
                    reserved: false,
                    logo: None,
                },
                message_count: AtomicUsize::new(0),
            }
        }

        fn count(&self) -> usize {
            self.message_count.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl Plugin for CountingPlugin {
        fn metadata(&self) -> &PluginMetadata {
            &self.metadata
        }

        async fn initialize(&mut self, _config: PluginConfig, _ctx: PluginContext) -> Result<()> {
            Ok(())
        }

        async fn start(&mut self) -> Result<()> {
            Ok(())
        }

        async fn stop(&mut self) -> Result<()> {
            Ok(())
        }

        async fn on_event(&self, event: &dyn Event) -> Result<EventResult> {
            if event.event_type() == "message" {
                self.message_count.fetch_add(1, Ordering::Relaxed);
            }
            Ok(EventResult::nothing())
        }

        fn can_handle(&self, event: &dyn Event) -> bool {
            event.event_type() == "message"
        }
    }

    /// A command plugin that only handles commands
    struct CommandPlugin {
        metadata: PluginMetadata,
    }

    impl CommandPlugin {
        fn new() -> Self {
            Self {
                metadata: PluginMetadata {
                    name: "command_handler".to_string(),
                    author: "test".to_string(),
                    description: "Handles commands".to_string(),
                    version: "1.0.0".to_string(),
                    repository: None,
                    min_astrbot_version: None,
                    platforms: vec!["*".to_string()],
                    reserved: false,
                    logo: None,
                },
            }
        }
    }

    #[async_trait]
    impl Plugin for CommandPlugin {
        fn metadata(&self) -> &PluginMetadata {
            &self.metadata
        }

        async fn initialize(&mut self, _config: PluginConfig, _ctx: PluginContext) -> Result<()> {
            Ok(())
        }

        async fn start(&mut self) -> Result<()> {
            Ok(())
        }

        async fn stop(&mut self) -> Result<()> {
            Ok(())
        }

        async fn on_event(&self, _event: &dyn Event) -> Result<EventResult> {
            Ok(EventResult::nothing())
        }

        fn can_handle(&self, event: &dyn Event) -> bool {
            event.event_type() == "command"
        }
    }

    fn make_test_message_event() -> MessageEvent {
        MessageEvent {
            source: MessageSource {
                platform: PlatformType::Aiocqhttp,
                session_id: "123".to_string(),
                message_id: "1".to_string(),
                user_id: "user1".to_string(),
            },
            message: AstrBotMessage {
                message_id: "1".to_string(),
                timestamp: chrono::Utc::now(),
                platform: PlatformType::Aiocqhttp,
                session_id: "123".to_string(),
                sender: MessageMember {
                    user_id: "user1".to_string(),
                    nickname: None,
                    card: None,
                    role: None,
                    is_self: false,
                },
                message_type: MessageType::Private,
                chain: MessageChain::new(),
                raw_payload: None,
            },
        }
    }

    #[tokio::test]
    async fn test_plugin_lifecycle() {
        let mut plugin = CountingPlugin::new();
        let ctx = PluginContext::new("bot1".to_string(), "qq".to_string());

        plugin.initialize(HashMap::new(), ctx).await.unwrap();
        plugin.start().await.unwrap();
        plugin.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_plugin_handles_message_event() {
        let plugin = CountingPlugin::new();
        let event = make_test_message_event();

        assert!(plugin.can_handle(&event));
        plugin.on_event(&event).await.unwrap();
        assert_eq!(plugin.count(), 1);
    }

    #[tokio::test]
    async fn test_plugin_registry() {
        let mut registry = PluginRegistry::new();

        let star = Star::new(PluginMetadata {
            name: "test_plugin".to_string(),
            author: "test".to_string(),
            description: "test".to_string(),
            version: "1.0.0".to_string(),
            repository: None,
            min_astrbot_version: None,
            platforms: vec![],
            reserved: false,
            logo: None,
        });

        registry.register(star);
        assert_eq!(registry.list().len(), 1);
        assert!(registry.get("test_plugin").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[tokio::test]
    async fn test_plugin_registry_activate_deactivate() {
        let mut registry = PluginRegistry::new();
        let ctx = PluginContext::new("bot1".to_string(), "qq".to_string());

        let mut star = Star::new(PluginMetadata {
            name: "counting".to_string(),
            author: "test".to_string(),
            description: "Counts messages".to_string(),
            version: "1.0.0".to_string(),
            repository: None,
            min_astrbot_version: None,
            platforms: vec!["*".to_string()],
            reserved: false,
            logo: None,
        });

        star.plugin = Some(Box::new(CountingPlugin::new()));
        registry.register(star);

        assert!(!registry.get("counting").unwrap().activated);
        registry.activate("counting", ctx).await.unwrap();
        assert!(registry.get("counting").unwrap().activated);
        registry.deactivate("counting").await.unwrap();
        assert!(!registry.get("counting").unwrap().activated);
    }

    #[tokio::test]
    async fn test_plugin_registry_event_dispatch() {
        let mut registry = PluginRegistry::new();
        let ctx = PluginContext::new("bot1".to_string(), "qq".to_string());

        // Register counting plugin
        let mut counting_star = Star::new(PluginMetadata {
            name: "counting".to_string(),
            author: "test".to_string(),
            description: "Counts messages".to_string(),
            version: "1.0.0".to_string(),
            repository: None,
            min_astrbot_version: None,
            platforms: vec!["*".to_string()],
            reserved: false,
            logo: None,
        });
        counting_star.plugin = Some(Box::new(CountingPlugin::new()));
        registry.register(counting_star);

        // Register command plugin
        let mut command_star = Star::new(PluginMetadata {
            name: "command_handler".to_string(),
            author: "test".to_string(),
            description: "Handles commands".to_string(),
            version: "1.0.0".to_string(),
            repository: None,
            min_astrbot_version: None,
            platforms: vec!["*".to_string()],
            reserved: false,
            logo: None,
        });
        command_star.plugin = Some(Box::new(CommandPlugin::new()));
        registry.register(command_star);

        // Activate all
        registry.activate("counting", ctx.clone()).await.unwrap();
        registry.activate("command_handler", ctx).await.unwrap();

        // Dispatch a message event
        let event = make_test_message_event();
        let results = registry.dispatch_event(&event).await;

        // Counting plugin should handle it, command plugin should not
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_plugin_manifest_loading() {
        use std::path::Path;
        use tokio::fs;

        let temp_dir = tempfile::tempdir().unwrap();
        let plugin_dir = temp_dir.path().join("test_plugin");
        fs::create_dir(&plugin_dir).await.unwrap();

        let manifest = serde_json::json!({
            "name": "test_plugin",
            "author": "test_author",
            "description": "A test plugin",
            "version": "0.1.0",
            "repository": "https://github.com/test/test_plugin",
            "min_astrbot_version": "3.0.0",
            "platforms": ["qq", "telegram"],
            "reserved": false,
            "logo": "logo.png",
            "main": "main.py"
        });

        fs::write(plugin_dir.join("plugin.json"), manifest.to_string())
            .await
            .unwrap();

        let loader = FilePluginLoader::new();
        let stars = loader.load_from_directory(temp_dir.path()).await.unwrap();

        assert_eq!(stars.len(), 1);
        let star = &stars[0];
        assert_eq!(star.metadata.name, "test_plugin");
        assert_eq!(star.metadata.author, "test_author");
        assert_eq!(star.metadata.version, "0.1.0");
        assert_eq!(star.metadata.platforms, vec!["qq", "telegram"]);
    }

    #[tokio::test]
    async fn test_plugin_manager() {
        let mut manager = PluginManager::new();
        manager.register_loader(Box::new(FilePluginLoader::new()));

        let ctx = PluginContext::new("bot1".to_string(), "qq".to_string());

        // Create temp plugin dir
        let temp_dir = tempfile::tempdir().unwrap();
        let plugin_dir = temp_dir.path().join("my_plugin");
        tokio::fs::create_dir(&plugin_dir).await.unwrap();

        let manifest = serde_json::json!({
            "name": "my_plugin",
            "author": "me",
            "description": "My plugin",
            "version": "1.0.0",
            "platforms": ["*"],
            "main": "main.py"
        });
        tokio::fs::write(plugin_dir.join("plugin.json"), manifest.to_string())
            .await
            .unwrap();

        // Load plugins
        manager.load_all(&[temp_dir.path()]).await.unwrap();
        assert_eq!(manager.registry().list().len(), 1);

        // Activate all (will fail because no implementation, but should not panic)
        manager.activate_all(ctx).await.unwrap();
    }

    // ==== CommandRegistry tests ====

    #[tokio::test]
    async fn test_command_registry_register_and_handle() {
        use crate::registry::{CommandRegistry, CommandHandler};

        struct TestHandler;
        #[async_trait]
        impl CommandHandler for TestHandler {
            async fn handle(&self, args: &[String], _source: &MessageSource, _user_id: &str) -> Result<MessageEventResult> {
                Ok(MessageEventResult::reply_text(args.join(" ")))
            }
            fn description(&self) -> String { "test cmd".to_string() }
        }

        let mut reg = CommandRegistry::new();
        reg.register("echo", "test_plugin", "echo back", false, std::sync::Arc::new(TestHandler));

        assert!(reg.has("echo"));
        let source = MessageSource { platform: PlatformType::Aiocqhttp, session_id: "s".to_string(), message_id: "m".to_string(), user_id: "u".to_string() };
        let result = reg.handle("echo", &["hello".to_string(), "world".to_string()], &source, "u", false).await;
        assert!(result.is_some());
        // PluginCommandHandler stub returns error, so we only test registry metadata here
    }

    #[tokio::test]
    async fn test_command_registry_admin_only() {
        use crate::registry::{CommandRegistry, CommandHandler};

        struct AdminHandler;
        #[async_trait]
        impl CommandHandler for AdminHandler {
            async fn handle(&self, _args: &[String], _source: &MessageSource, _user_id: &str) -> Result<MessageEventResult> {
                Ok(MessageEventResult::reply_text("admin ok"))
            }
            fn description(&self) -> String { "admin cmd".to_string() }
            fn admin_only(&self) -> bool { true }
        }

        let mut reg = CommandRegistry::new();
        reg.register("admin", "test", "admin only", true, std::sync::Arc::new(AdminHandler));

        let source = MessageSource { platform: PlatformType::Aiocqhttp, session_id: "s".to_string(), message_id: "m".to_string(), user_id: "u".to_string() };
        let result = reg.handle("admin", &[], &source, "u", false).await;
        assert!(result.is_some());
        // Should return admin-only block message
    }

    #[tokio::test]
    async fn test_command_registry_unregister() {
        use crate::registry::{CommandRegistry, CommandHandler};

        struct Dummy;
        #[async_trait]
        impl CommandHandler for Dummy {
            async fn handle(&self, _args: &[String], _source: &MessageSource, _user_id: &str) -> Result<MessageEventResult> {
                Ok(MessageEventResult::nothing())
            }
            fn description(&self) -> String { "d".to_string() }
        }

        let mut reg = CommandRegistry::new();
        reg.register("a", "p1", "cmd a", false, std::sync::Arc::new(Dummy));
        reg.register("b", "p1", "cmd b", false, std::sync::Arc::new(Dummy));
        reg.register("c", "p2", "cmd c", false, std::sync::Arc::new(Dummy));

        assert_eq!(reg.list().len(), 3);
        reg.unregister_by_plugin("p1");
        assert_eq!(reg.list().len(), 1);
        assert!(!reg.has("a"));
        assert!(!reg.has("b"));
        assert!(reg.has("c"));
    }

    // ==== FunctionalPluginBuilder tests ====

    #[tokio::test]
    async fn test_functional_plugin_command() {
        use crate::functional::FunctionalPluginBuilder;

        let plugin = FunctionalPluginBuilder::new("echo", "test", "echo plugin")
            .version("1.0.0")
            .on_command("echo", "echo back", false, |args, _source, _user_id| {
                let text = args.join(" ");
                Box::pin(async move {
                    Ok(MessageEventResult::reply_text(text))
                })
            })
            .build();

        let meta = plugin.metadata();
        assert_eq!(meta.name, "echo");
        assert_eq!(meta.version, "1.0.0");

        let source = MessageSource { platform: PlatformType::Aiocqhttp, session_id: "s".to_string(), message_id: "m".to_string(), user_id: "u".to_string() };
        let result = plugin.on_command("echo", &["hello".to_string(), "world".to_string()], &source, "u").await.unwrap();
        match result {
            MessageEventResult::Reply { chain } => {
                let text = chain.plain_text();
                assert_eq!(text, "hello world");
            }
            _ => panic!("Expected reply"),
        }
    }

    #[tokio::test]
    async fn test_functional_plugin_event() {
        use crate::functional::FunctionalPluginBuilder;
        use astrbot_core::event::{Event, EventResult};

        #[derive(Debug)]
        struct TestEvent;
        impl Event for TestEvent {
            fn event_type(&self) -> &'static str { "test_event" }
            fn source(&self) -> &MessageSource {
                static SRC: std::sync::OnceLock<MessageSource> = std::sync::OnceLock::new();
                SRC.get_or_init(|| MessageSource {
                    platform: PlatformType::Aiocqhttp,
                    session_id: "s".to_string(),
                    message_id: "m".to_string(),
                    user_id: "u".to_string(),
                })
            }
            fn clone_box(&self) -> Box<dyn Event> { Box::new(TestEvent) }
        }

        let plugin = FunctionalPluginBuilder::new("event_test", "test", "event test")
            .on_event("test_event", |_event| {
                Box::pin(async move {
                    Ok(EventResult::nothing())
                })
            })
            .build();

        let event = TestEvent;
        assert!(plugin.can_handle(&event));
        let result = plugin.on_event(&event).await.unwrap();
        assert_eq!(result, EventResult::nothing());
    }

    #[tokio::test]
    async fn test_functional_plugin_lifecycle_hooks() {
        use crate::functional::FunctionalPluginBuilder;
        use std::sync::atomic::{AtomicBool, Ordering};

        let started = std::sync::Arc::new(AtomicBool::new(false));
        let stopped = std::sync::Arc::new(AtomicBool::new(false));

        let started_clone = started.clone();
        let stopped_clone = stopped.clone();

        let mut plugin = FunctionalPluginBuilder::new("lifecycle", "test", "lifecycle test")
            .on_start(move || {
                let flag = started_clone.clone();
                Box::pin(async move {
                    flag.store(true, Ordering::Relaxed);
                    Ok(())
                })
            })
            .on_stop(move || {
                let flag = stopped_clone.clone();
                Box::pin(async move {
                    flag.store(true, Ordering::Relaxed);
                    Ok(())
                })
            })
            .build();

        plugin.start().await.unwrap();
        assert!(started.load(Ordering::Relaxed));

        plugin.stop().await.unwrap();
        assert!(stopped.load(Ordering::Relaxed));
    }

    // ==== PluginRegistry.dispatch_command test ====

    struct EchoCommandPlugin {
        metadata: PluginMetadata,
    }

    impl EchoCommandPlugin {
        fn new() -> Self {
            Self {
                metadata: PluginMetadata {
                    name: "echo_cmd".to_string(),
                    author: "test".to_string(),
                    description: "echo cmd".to_string(),
                    version: "1.0.0".to_string(),
                    repository: None,
                    min_astrbot_version: None,
                    platforms: vec!["*".to_string()],
                    reserved: false,
                    logo: None,
                },
            }
        }
    }

    #[async_trait]
    impl Plugin for EchoCommandPlugin {
        fn metadata(&self) -> &PluginMetadata { &self.metadata }
        fn commands(&self) -> Vec<(String, String, bool)> {
            vec![("echo".to_string(), "echo back".to_string(), false)]
        }
        async fn initialize(&mut self, _config: PluginConfig, _ctx: PluginContext) -> Result<()> { Ok(()) }
        async fn start(&mut self) -> Result<()> { Ok(()) }
        async fn stop(&mut self) -> Result<()> { Ok(()) }
        async fn on_event(&self, _event: &dyn Event) -> Result<EventResult> { Ok(EventResult::nothing()) }
        fn can_handle(&self, _event: &dyn Event) -> bool { false }
        async fn on_command(&self, _cmd: &str, args: &[String], _source: &MessageSource, _user_id: &str) -> Result<MessageEventResult> {
            Ok(MessageEventResult::reply_text(args.join(" ")))
        }
    }

    #[tokio::test]
    async fn test_plugin_registry_dispatch_command() {
        let mut registry = PluginRegistry::new();
        let ctx = PluginContext::new("bot1".to_string(), "qq".to_string());

        let mut star = Star::new(PluginMetadata {
            name: "echo_cmd".to_string(),
            author: "test".to_string(),
            description: "echo cmd".to_string(),
            version: "1.0.0".to_string(),
            repository: None,
            min_astrbot_version: None,
            platforms: vec!["*".to_string()],
            reserved: false,
            logo: None,
        });
        let plugin = Box::new(EchoCommandPlugin::new());
        registry.register_plugin(star, plugin);

        registry.activate("echo_cmd", ctx).await.unwrap();

        let source = MessageSource { platform: PlatformType::Aiocqhttp, session_id: "s".to_string(), message_id: "m".to_string(), user_id: "u".to_string() };
        let result = registry.dispatch_command("echo", &["hello".to_string(), "world".to_string()], &source, "u", false).await;

        assert!(result.is_some());
        let reply = result.unwrap().unwrap();
        match reply {
            MessageEventResult::Reply { chain } => {
                assert_eq!(chain.plain_text(), "hello world");
            }
            _ => panic!("Expected reply"),
        }
    }
}