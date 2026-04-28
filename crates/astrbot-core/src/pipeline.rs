//! Message pipeline - orchestrates incoming messages → SessionManager → Provider → Reply
//!
//! Plugin integration: dispatches commands to PluginRegistry and events to plugins.

use async_trait::async_trait;
use crate::{
    access::{AccessManager, AccessControl, RateLimitConfig},
    db::Database,
    errors::{AstrBotError, Result},
    message::{AstrBotMessage, MessageChain, MessageComponent, MessageEventResult, MessageHandler},
    platform::MessageSource,
    provider::{ChatConfig, Provider},
    session::SessionManager,
};
use std::sync::Arc;
use tracing::{error, info, warn};

/// Trait for sending replies back to platforms
#[async_trait::async_trait]
pub trait ReplySender: Send + Sync {
    async fn send_reply(
        &self, source: &MessageSource, chain: &MessageChain
    ) -> Result<()>;
}

/// Trait for resolving plugin-registered commands
#[async_trait]
pub trait PluginCommandResolver: Send + Sync {
    /// Try to handle a command via plugin registry
    async fn resolve(
        &self,
        cmd: &str,
        args: &[String],
        source: &MessageSource,
        user_id: &str,
        is_admin: bool,
    ) -> Option<Result<MessageEventResult>>;
    /// Get plugin command list for /help
    fn command_list(&self) -> Vec<(String, String, bool)>;
}

/// Context compression strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextCompression {
    /// No compression — send all messages
    None,
    /// Truncate by number of turns (keep most recent N)
    TruncateByTurns { max_turns: usize },
    /// Truncate by total token count (approximate)
    TruncateByTokens { max_tokens: usize },
}

/// Configuration for the message pipeline
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Bot nickname (used in system prompt)
    pub nickname: String,
    /// System prompt template
    pub system_prompt: Option<String>,
    /// Default chat config for LLM calls
    pub chat_config: ChatConfig,
    /// Whether to persist messages
    pub persist_messages: bool,
    /// Max context messages to send to LLM
    pub max_context_messages: usize,
    /// Command prefixes (e.g., ["/", "!"])
    pub command_prefixes: Vec<String>,
    /// Admin user IDs
    pub admins: Vec<String>,
    /// Rate limiting configuration
    pub rate_limit: Option<RateLimitConfig>,
    /// Access control (whitelist / blacklist)
    pub access_control: Option<AccessControl>,
    /// Context compression strategy
    pub context_compression: ContextCompression,
    /// Dequeue context length (reserve space for response)
    pub dequeue_context_length: usize,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            nickname: "AstrBot".to_string(),
            system_prompt: Some("You are a helpful assistant.".to_string()),
            chat_config: ChatConfig::default(),
            persist_messages: true,
            max_context_messages: 20,
            command_prefixes: vec!["/".to_string()],
            admins: Vec::new(),
            rate_limit: Some(RateLimitConfig::default()),
            access_control: None,
            context_compression: ContextCompression::TruncateByTurns { max_turns: 10 },
            dequeue_context_length: 2048,
        }
    }
}

/// The central message pipeline that connects all components
pub struct MessagePipeline {
    config: PipelineConfig,
    session_manager: SessionManager,
    provider: Arc<dyn Provider>,
    /// Map of platform type name → reply sender
    reply_senders: dashmap::DashMap<String, Arc<dyn ReplySender>>,
    /// Access manager (rate limit + whitelist)
    access_manager: Option<AccessManager>,
    /// Plugin command registry (optional — set by runtime)
    command_resolver: Option<Arc<dyn PluginCommandResolver>>,
}

impl MessagePipeline {
    /// Create a new message pipeline
    pub fn new(
        config: PipelineConfig,
        database: Arc<Database>,
        provider: Arc<dyn Provider>,
    ) -> Self {
        let session_manager = SessionManager::new(database)
            .with_max_context(config.max_context_messages)
            .with_persistence(config.persist_messages);

        let access_manager = if config.rate_limit.is_some() || config.access_control.is_some() {
            let rate = config.rate_limit.clone().unwrap_or_default();
            let access = config.access_control.clone().unwrap_or_default();
            Some(AccessManager::new(rate, access))
        } else {
            None
        };

        Self {
            config,
            session_manager,
            provider,
            reply_senders: dashmap::DashMap::new(),
            access_manager,
            command_resolver: None,
        }
    }

    /// Set the plugin command resolver
    pub fn set_command_resolver(&mut self, resolver: Arc<dyn PluginCommandResolver>) {
        self.command_resolver = Some(resolver);
    }

    /// Register a reply sender for a platform type
    pub fn register_sender(
        &self, platform: String, sender: Arc<dyn ReplySender>,
    ) {
        self.reply_senders.insert(platform.clone(), sender);
        info!("[Pipeline] Registered reply sender for: {}", platform);
    }

    /// Get the session manager
    pub fn session_manager(&self) -> &SessionManager {
        &self.session_manager
    }

    /// Build a MessageSource from an incoming message
    fn build_source(message: &AstrBotMessage) -> MessageSource {
        MessageSource {
            platform: message.platform,
            session_id: message.session_id.clone(),
            message_id: message.message_id.clone(),
            user_id: message.sender.user_id.clone(),
        }
    }

    /// Apply context compression to messages
    fn compress_context(
        &self, messages: Vec<crate::provider::ChatMessage>,
    ) -> Vec<crate::provider::ChatMessage> {
        match self.config.context_compression {
            ContextCompression::None => messages,
            ContextCompression::TruncateByTurns { max_turns } => {
                // Keep system prompt + last N user/assistant pairs
                let system_msgs: Vec<_> = messages.iter()
                    .filter(|m| m.role == "system")
                    .cloned()
                    .collect();
                let other_msgs: Vec<_> = messages.iter()
                    .filter(|m| m.role != "system")
                    .cloned()
                    .collect();

                let keep_count = max_turns * 2; // Each turn = user + assistant
                let start = other_msgs.len().saturating_sub(keep_count);
                let kept = other_msgs[start..].to_vec();

                let mut result = system_msgs;
                result.extend(kept);
                result
            }
            ContextCompression::TruncateByTokens { max_tokens } => {
                let system_msgs: Vec<_> = messages.iter()
                    .filter(|m| m.role == "system")
                    .cloned()
                    .collect();
                let other_msgs: Vec<_> = messages.iter()
                    .filter(|m| m.role != "system")
                    .cloned()
                    .collect();

                let reserve = self.config.dequeue_context_length;
                let budget = if max_tokens > reserve { max_tokens - reserve } else { max_tokens / 2 };

                let mut estimated = system_msgs.iter()
                    .map(|m| estimate_tokens(&m.content))
                    .sum::<usize>();

                let mut kept = Vec::new();
                for msg in other_msgs.iter().rev() {
                    let cost = estimate_tokens(&msg.content);
                    if estimated + cost > budget && !kept.is_empty() {
                        break;
                    }
                    estimated += cost;
                    kept.push(msg.clone());
                }
                kept.reverse();

                let mut result = system_msgs;
                result.extend(kept);
                result
            }
        }
    }

    /// Extract multimodal content (images, voice) from a message chain
    pub fn extract_multimodal(chain: &MessageChain) -> Vec<String> {
        let mut urls = Vec::new();
        for component in chain.0.iter() {
            match component {
                MessageComponent::Image { url: Some(u), .. } => urls.push(format!("[Image: {}]", u)),
                MessageComponent::Image { base64: Some(b), .. } => urls.push(format!("[Image: base64:{}]", b.chars().take(20).collect::<String>())),
                MessageComponent::Voice { url: Some(u), .. } => urls.push(format!("[Voice: {}]", u)),
                MessageComponent::Voice { base64: Some(b), .. } => urls.push(format!("[Voice: base64:{}]", b.chars().take(20).collect::<String>())),
                _ => {}
            }
        }
        urls
    }

    /// Process a single incoming message through the pipeline
    async fn process_message(&self, message: &AstrBotMessage) -> Result<MessageEventResult> {
        let source = Self::build_source(message);
        let user_id = &message.sender.user_id;

        // 1. Access control + rate limit check
        if let Some(ref access) = self.access_manager {
            if let Err(reason) = access.check(user_id).await {
                warn!("[Pipeline] Access denied for {}: {}", user_id, reason);
                return Ok(MessageEventResult::reply_text(
                    format!("⚠️ {}", reason)
                ));
            }
        }

        // 2. Ensure session exists
        self.session_manager.ensure_session(&source).await?;

        // 3. Save user message (with multimodal content appended)
        let mut user_content = message.chain.plain_text();
        let multimodal = Self::extract_multimodal(&message.chain);
        if !multimodal.is_empty() {
            user_content.push_str("\n\n[Multimedia content]\n");
            for item in multimodal {
                user_content.push_str(&item);
                user_content.push('\n');
            }
        }
        self.session_manager.save_user_message(&source, &user_content).await?;

        // 4. Check for built-in commands
        let prefixes: Vec<char> = self.config.command_prefixes.iter()
            .filter_map(|s| s.chars().next())
            .collect();

        if message.chain.is_command(&prefixes) {
            if let Some((cmd, args)) = message.chain.parse_command(&prefixes) {
                match self.handle_command(&cmd, &args, &source, user_id).await {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        warn!("[Pipeline] Command error: {}", e);
                        // Fall through to normal LLM processing
                    }
                }
            }
        }

        // 5. Build context for LLM
        let system_prompt = self.config.system_prompt.as_deref();
        let context = self.session_manager.build_context(&source, system_prompt).await?;

        // 6. Apply context compression
        let compressed = self.compress_context(context);

        // 7. Call LLM
        let response = self.provider.chat(compressed, self.config.chat_config.clone()).await?;
        let reply_text = response.content;

        // 8. Save assistant message
        let model = Some(response.model.as_str());
        self.session_manager.save_assistant_message(&source, &reply_text, model).await?;

        Ok(MessageEventResult::reply_text(reply_text))
    }

    /// Handle built-in commands + plugin commands
    async fn handle_command(
        &self,
        cmd: &str,
        args: &[String],
        source: &MessageSource,
        user_id: &str,
    ) -> Result<MessageEventResult> {
        let is_admin = self.config.admins.contains(&user_id.to_string());

        // Try built-in commands first
        let builtin = match cmd {
            "sid" | "session" => {
                Ok(MessageEventResult::reply_text(format!(
                    "Session ID: {:?}_{}_{}\nUser ID: {}",
                    source.platform, source.session_id, source.user_id, source.user_id
                )))
            }
            "help" | "h" => {
                let mut help_text = String::from(
                    "Available commands:\n\
                    /sid — Get session ID\n\
                    /wl — Whitelist management (admin only)\n\
                    /ping — Check if bot is alive\n\
                    /help — Show this help"
                );
                // Append plugin commands
                if let Some(ref resolver) = self.command_resolver {
                    let plugin_cmds = resolver.command_list();
                    if !plugin_cmds.is_empty() {
                        help_text.push_str("\n\nPlugin commands:\n");
                        for (name, desc, admin) in plugin_cmds {
                            let marker = if admin { " [admin]" } else { "" };
                            help_text.push_str(&format!("/{} — {}{}\n", name, desc, marker));
                        }
                    }
                }
                Ok(MessageEventResult::reply_text(help_text))
            }
            "ping" => Ok(MessageEventResult::reply_text("Pong!".to_string())),
            "wl" | "whitelist" => {
                if !is_admin {
                    return Ok(MessageEventResult::reply_text(
                        "⛔ Admin only command.".to_string()
                    ));
                }

                if args.is_empty() {
                    return Ok(MessageEventResult::reply_text(
                        "Usage: /wl add <user_id> | /wl remove <user_id> | /wl list | /wl on | /wl off".to_string()
                    ));
                }

                match args[0].as_str() {
                    "add" => {
                        if args.len() < 2 {
                            return Ok(MessageEventResult::reply_text("Usage: /wl add <user_id>".to_string()));
                        }
                        Ok(MessageEventResult::reply_text(
                            format!("📝 Requested to whitelist: {} (restart bot to apply)", args[1])
                        ))
                    }
                    "remove" => {
                        if args.len() < 2 {
                            return Ok(MessageEventResult::reply_text("Usage: /wl remove <user_id>".to_string()));
                        }
                        Ok(MessageEventResult::reply_text(
                            format!("📝 Requested to remove from whitelist: {} (restart bot to apply)", args[1])
                        ))
                    }
                    "list" => {
                        Ok(MessageEventResult::reply_text(
                            "Whitelist: (view via config file or restart with /wl status)".to_string()
                        ))
                    }
                    "on" => {
                        Ok(MessageEventResult::reply_text("📝 Requested to enable whitelist mode (restart to apply)".to_string()))
                    }
                    "off" => {
                        Ok(MessageEventResult::reply_text("📝 Requested to disable whitelist mode (restart to apply)".to_string()))
                    }
                    _ => {
                        Ok(MessageEventResult::reply_text(
                            "Unknown /wl subcommand. Use: add, remove, list, on, off".to_string()
                        ))
                    }
                }
            }
            _ => Err(AstrBotError::NotFound(format!("Unknown command: {}", cmd))),
        };

        // If built-in handled it, return
        if builtin.is_ok() {
            return builtin;
        }

        // Try plugin commands
        if let Some(ref resolver) = self.command_resolver {
            if let Some(result) = resolver.resolve(cmd, args, source, user_id, is_admin).await {
                return result;
            }
        }

        builtin
    }

    /// Send a reply through the appropriate reply sender
    async fn send_reply(&self, message: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
        let platform = format!("{:?}", message.platform).to_lowercase();

        let sender = self.reply_senders.get(&platform)
            .map(|entry| entry.value().clone());

        if let Some(sender) = sender {
            let source = Self::build_source(message);
            sender.send_reply(&source, chain).await?;
            Ok(())
        } else {
            Err(AstrBotError::Platform {
                adapter: platform,
                message: "No reply sender registered for this platform".to_string(),
            })
        }
    }
}

/// Rough token estimation (CJK ≈ 1 char/token, English ≈ 1 word/1.3 tokens)
fn estimate_tokens(text: &str) -> usize {
    let cjk_count = text.chars().filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c) || ('\u{3400}'..='\u{4dbf}').contains(c)).count();
    let non_cjk = text.chars().filter(|c| c.is_alphabetic()).count();
    let words = non_cjk / 5 + 1;
    cjk_count + words * 2 / 3
}

#[async_trait]
impl MessageHandler for MessagePipeline {
    async fn on_message(&self, message: AstrBotMessage) {
        match self.process_message(&message).await {
            Ok(MessageEventResult::Reply { chain }) => {
                if let Err(e) = self.send_reply(&message, &chain).await {
                    error!("[Pipeline] Failed to send reply: {}", e);
                }
            }
            Ok(MessageEventResult::Nothing) => {
                // No reply needed
            }
            Ok(MessageEventResult::Forward { .. }) => {
                warn!("[Pipeline] Forward not yet implemented");
            }
            Err(e) => {
                error!("[Pipeline] Error processing message: {}", e);
                // Try to send error message back
                let error_chain = MessageChain::new().text(format!("Error: {}", e));
                let _ = self.send_reply(&message, &error_chain).await;
            }
        }
    }
}
