//! Pipeline 9-Stage architecture — inspired by AstrBot Python
//!
//! Stage execution order:
//! 1. WakingCheckStage      — wake prefix / @ mention / handler filter
//! 2. WhitelistCheckStage   — id whitelist check
//! 3. SessionStatusCheckStage — session enabled/disabled
//! 4. RateLimitStage        — fixed-window rate limiting
//! 5. ContentSafetyCheckStage — content safety (placeholder)
//! 6. PreProcessStage       — STT / path mapping (placeholder)
//! 7. ProcessStage           — core processing (Star handlers / LLM Agent)
//! 8. ResultDecorateStage    — T2I / TTS / segmented reply (placeholder)
//! 9. RespondStage           — send message

use async_trait::async_trait;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{info, warn, debug};

use crate::errors::{AstrBotError, Result};
use crate::message::{AstrBotMessage, MessageChain, MessageComponent, MessageType};

pub mod process;
pub mod respond;

pub use process::ProcessStage;
pub use respond::{RespondStage, SendFn};

// ---------------------------------------------------------------------------
// PipelineContext
// ---------------------------------------------------------------------------

pub struct PipelineContext {
    // TODO: astrbot_config, plugin_manager, conversation_manager
}

impl PipelineContext {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for PipelineContext {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// StageFlow — onion model support
// ---------------------------------------------------------------------------

pub enum StageFlow {
    /// Normal coroutine — proceed to next stage after completion
    Done,
    /// Onion model — yield before/after recursive stage execution
    Wrap,
}

// ---------------------------------------------------------------------------
// Stage trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait Stage: Send + Sync {
    async fn initialize(&mut self, ctx: &PipelineContext) -> Result<()>;

    /// Process the event.
    ///
    /// Returns `Ok(StageFlow::Done)` — normal flow, continue to next stage.
    /// Returns `Ok(StageFlow::Wrap)` — onion model, caller will recurse into
    /// subsequent stages between pre/post yield points.
    async fn process(&self, event: &mut PipelineEvent) -> Result<StageFlow>;
}

// ---------------------------------------------------------------------------
// PipelineEvent — mutable event state carried through all stages
// ---------------------------------------------------------------------------

pub struct PipelineEvent {
    pub message: AstrBotMessage,
    pub is_stopped: bool,
    pub is_wake: bool,
    pub is_at_or_wake_command: bool,
    pub role: String, // "user" | "admin"
    pub activated_handlers: Vec<String>, // TODO: StarHandlerMetadata
    pub extras: HashMap<String, String>,
    pub result_chain: Option<MessageChain>,
}

impl PipelineEvent {
    pub fn new(message: AstrBotMessage) -> Self {
        Self {
            message,
            is_stopped: false,
            is_wake: false,
            is_at_or_wake_command: false,
            role: "user".to_string(),
            activated_handlers: vec![],
            extras: HashMap::new(),
            result_chain: None,
        }
    }

    pub fn stop_event(&mut self) {
        self.is_stopped = true;
    }

    pub fn is_stopped(&self) -> bool {
        self.is_stopped
    }

    pub fn set_extra(&mut self, key: &str, value: String) {
        self.extras.insert(key.to_string(), value);
    }

    pub fn get_extra(&self, key: &str) -> Option<&String> {
        self.extras.get(key)
    }

    pub fn is_private_chat(&self) -> bool {
        matches!(self.message.message_type, MessageType::Private)
    }

    pub fn is_group_chat(&self) -> bool {
        matches!(self.message.message_type, MessageType::Group)
    }
}

// ---------------------------------------------------------------------------
// StageRegistry — ordered stage collection
// ---------------------------------------------------------------------------

pub struct StageRegistry {
    stages: Vec<(String, Box<dyn Stage>)>,
    order: Vec<&'static str>,
}

impl StageRegistry {
    pub fn new() -> Self {
        Self {
            stages: vec![],
            order: vec![
                "WakingCheckStage",
                "WhitelistCheckStage",
                "SessionStatusCheckStage",
                "RateLimitStage",
                "ContentSafetyCheckStage",
                "PreProcessStage",
                "ProcessStage",
                "ResultDecorateStage",
                "RespondStage",
            ],
        }
    }

    pub fn register(&mut self, name: &'static str, stage: Box<dyn Stage>) {
        let pos = self.order.iter().position(|&o| o == name).unwrap_or(usize::MAX);
        self.stages.push((name.to_string(), stage));
        // Sort by declared order
        self.stages.sort_by_key(|(name, _)| {
            self.order.iter().position(|&o| o == name.as_str()).unwrap_or(usize::MAX)
        });
    }

    pub async fn initialize_all(&mut self, ctx: &PipelineContext) -> Result<()> {
        for (_name, stage) in &mut self.stages {
            stage.initialize(ctx).await?;
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.stages.len()
    }
}

// ---------------------------------------------------------------------------
// PipelineScheduler — onion-model recursive executor
// ---------------------------------------------------------------------------

pub struct PipelineScheduler {
    ctx: Arc<PipelineContext>,
    registry: StageRegistry,
    metrics_collector: Option<Arc<tokio::sync::Mutex<crate::metrics::MetricsCollector>>>,
}

impl PipelineScheduler {
    pub fn new(ctx: Arc<PipelineContext>, registry: StageRegistry) -> Self {
        Self { ctx, registry, metrics_collector: None }
    }

    pub fn with_metrics_collector(
        mut self,
        mc: Arc<tokio::sync::Mutex<crate::metrics::MetricsCollector>>,
    ) -> Self {
        self.metrics_collector = Some(mc);
        self
    }

    pub async fn execute(&self, event: &mut PipelineEvent) -> Result<()> {
        // Count incoming message by platform
        let platform = format!("{:?}", event.message.platform).to_lowercase();
        if let Some(ref mc) = self.metrics_collector {
            let mut lock = mc.lock().await;
            lock.increment_message_count(&platform);
        }

        for i in 0..self.registry.stages.len() {
            let (name, stage) = &self.registry.stages[i];

            let flow = stage.process(event).await?;

            match flow {
                StageFlow::Wrap => {
                    // Onion model: pre-yield → recurse → post-yield
                    // TODO: implement post-processing after subsequent stages
                    // For now, simplified: continue to next stage
                }
                StageFlow::Done => {
                    // Normal flow, continue to next stage
                }
            }

            if event.is_stopped() {
                tracing::debug!("Stage {} stopped event propagation", name);
                break;
            }
        }
        Ok(())
    }
}

// =============================================================================
// Stage Implementations
// =============================================================================

// ---------------------------------------------------------------------------
// 1. WakingCheckStage
// ---------------------------------------------------------------------------

pub struct WakingCheckStage {
    wake_prefixes: Vec<String>,
    friend_needs_wake_prefix: bool,
    ignore_bot_self_message: bool,
    ignore_at_all: bool,
    admins: Vec<String>,
    unique_session: bool,
    disable_builtin_commands: bool,
}

impl Default for WakingCheckStage {
    fn default() -> Self {
        Self {
            wake_prefixes: vec!["/".to_string()],
            friend_needs_wake_prefix: false,
            ignore_bot_self_message: false,
            ignore_at_all: false,
            admins: vec![],
            unique_session: false,
            disable_builtin_commands: false,
        }
    }
}

#[async_trait]
impl Stage for WakingCheckStage {
    async fn initialize(&mut self, _ctx: &PipelineContext) -> Result<()> {
        // TODO: load from astrbot_config
        Ok(())
    }

    async fn process(&self, event: &mut PipelineEvent) -> Result<StageFlow> {
        // 1. unique_session builder (platform-specific session id)
        // TODO: platform-specific unique session id

        // 2. ignore bot self message
        // TODO: check if sender is self

        // 3. Set sender role (admin check)
        // TODO: check admins_id list

        // 4. Check wake prefix / @ mention / private chat
        let msg_str = event.message.chain.plain_text();
        let mut is_wake = false;

        for prefix in &self.wake_prefixes {
            if msg_str.starts_with(prefix) {
                is_wake = true;
                event.is_wake = true;
                event.is_at_or_wake_command = true;
                break;
            }
        }

        // Check @ mention (simplified)
        for comp in event.message.chain.components() {
            if let MessageComponent::At { target, .. } = comp {
                // TODO: check if at self
                if !self.ignore_at_all || target != "all" {
                    is_wake = true;
                    event.is_wake = true;
                    event.is_at_or_wake_command = true;
                }
            }
        }

        // Private chat auto-wake
        if event.is_private_chat() && !self.friend_needs_wake_prefix {
            is_wake = true;
            event.is_wake = true;
            event.is_at_or_wake_command = true;
        }

        // 5. Check plugin handler filters (simplified)
        // TODO: iterate star_handlers_registry, filter by event type + permissions
        // TODO: SessionPluginManager.filter_handlers_by_session

        // Set activated handlers
        // event.set_extra("activated_handlers", ...);

        if !is_wake {
            event.stop_event();
        }

        Ok(StageFlow::Done)
    }
}

// ---------------------------------------------------------------------------
// 2. WhitelistCheckStage
// ---------------------------------------------------------------------------

pub struct WhitelistCheckStage {
    enabled: bool,
    whitelist: Vec<String>,
    ignore_admin_on_group: bool,
    ignore_admin_on_friend: bool,
}

impl Default for WhitelistCheckStage {
    fn default() -> Self {
        Self {
            enabled: false,
            whitelist: vec![],
            ignore_admin_on_group: false,
            ignore_admin_on_friend: false,
        }
    }
}

#[async_trait]
impl Stage for WhitelistCheckStage {
    async fn initialize(&mut self, _ctx: &PipelineContext) -> Result<()> {
        // TODO: load from astrbot_config["platform_settings"]["enable_id_white_list"]
        Ok(())
    }

    async fn process(&self, event: &mut PipelineEvent) -> Result<StageFlow> {
        if !self.enabled || self.whitelist.is_empty() {
            return Ok(StageFlow::Done);
        }

        // TODO: webchat exempt

        // Admin exempt
        if event.role == "admin" {
            if self.ignore_admin_on_group && event.is_group_chat() {
                return Ok(StageFlow::Done);
            }
            if self.ignore_admin_on_friend && event.is_private_chat() {
                return Ok(StageFlow::Done);
            }
        }

        // Check whitelist
        let session_id = &event.message.session_id;
        // TODO: get group_id from message
        let in_whitelist = self.whitelist.iter().any(|id| id == session_id);

        if !in_whitelist {
            event.stop_event();
        }

        Ok(StageFlow::Done)
    }
}

// ---------------------------------------------------------------------------
// 3. SessionStatusCheckStage
// ---------------------------------------------------------------------------

pub struct SessionStatusCheckStage {
    /// Sessions that are explicitly disabled
    disabled_sessions: Arc<Mutex<HashMap<String, ()>>>,
}

impl Default for SessionStatusCheckStage {
    fn default() -> Self {
        Self {
            disabled_sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl Stage for SessionStatusCheckStage {
    async fn initialize(&mut self, _ctx: &PipelineContext) -> Result<()> {
        // TODO: load disabled sessions from config/db
        Ok(())
    }

    async fn process(&self, event: &mut PipelineEvent) -> Result<StageFlow> {
        let session_id = &event.message.session_id;
        let disabled = self.disabled_sessions.lock().await;
        if disabled.contains_key(session_id) {
            event.stop_event();
            return Ok(StageFlow::Done);
        }
        Ok(StageFlow::Done)
    }
}

// ---------------------------------------------------------------------------
// 4. RateLimitStage — Fixed Window
// ---------------------------------------------------------------------------

pub struct RateLimitStage {
    rate_limit_count: usize,
    rate_limit_time: Duration,
    strategy: RateLimitStrategy,
    timestamps: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RateLimitStrategy {
    Stall,
    Discard,
}

impl Default for RateLimitStage {
    fn default() -> Self {
        Self {
            rate_limit_count: 0,
            rate_limit_time: Duration::from_secs(0),
            strategy: RateLimitStrategy::Stall,
            timestamps: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl Stage for RateLimitStage {
    async fn initialize(&mut self, _ctx: &PipelineContext) -> Result<()> {
        // TODO: load from config
        Ok(())
    }

    async fn process(&self, event: &mut PipelineEvent) -> Result<StageFlow> {
        if self.rate_limit_count == 0 {
            return Ok(StageFlow::Done);
        }

        let session_id = event.message.session_id.clone();
        let now = Instant::now();

        let mut map = self.timestamps.lock().await;
        let timestamps = map.entry(session_id.clone()).or_insert_with(VecDeque::new);

        // Remove expired timestamps
        let threshold = now - self.rate_limit_time;
        while let Some(&first) = timestamps.front() {
            if first < threshold {
                timestamps.pop_front();
            } else {
                break;
            }
        }

        if timestamps.len() < self.rate_limit_count {
            timestamps.push_back(now);
            return Ok(StageFlow::Done);
        }

        // Rate limit triggered
        match self.strategy {
            RateLimitStrategy::Stall => {
                let next_window = *timestamps.front().unwrap() + self.rate_limit_time;
                let stall_duration = next_window.saturating_duration_since(now) + Duration::from_millis(300);
                tracing::info!(
                    "Session {} rate limited, stalling for {:.2}s",
                    session_id,
                    stall_duration.as_secs_f64()
                );
                tokio::time::sleep(stall_duration).await;
                timestamps.push_back(Instant::now());
            }
            RateLimitStrategy::Discard => {
                tracing::info!("Session {} rate limited, discarding request", session_id);
                event.stop_event();
            }
        }

        Ok(StageFlow::Done)
    }
}

// ---------------------------------------------------------------------------
// 5. ContentSafetyCheckStage — content safety (Baidu + keyword + regex)
// ---------------------------------------------------------------------------

pub struct ContentSafetyCheckStage {
    engine: crate::safety::SafetyEngine,
    /// If true, stop the event on violation (discard message).
    /// If false, mark as unsafe but allow processing (for logging/auditing).
    stop_on_violation: bool,
}

impl Default for ContentSafetyCheckStage {
    fn default() -> Self {
        Self {
            engine: crate::safety::preset_engine(),
            stop_on_violation: true,
        }
    }
}

impl ContentSafetyCheckStage {
    /// Create with a custom safety engine (e.g. including BaiduContentSafety).
    pub fn with_engine(engine: crate::safety::SafetyEngine) -> Self {
        Self {
            engine,
            stop_on_violation: true,
        }
    }

    /// Configure stop-on-violation behavior.
    pub fn with_stop_on_violation(mut self, stop: bool) -> Self {
        self.stop_on_violation = stop;
        self
    }
}

#[async_trait]
impl Stage for ContentSafetyCheckStage {
    async fn initialize(&mut self, _ctx: &PipelineContext) -> Result<()> {
        // TODO: load from astrbot_config — e.g. baidu_client_id/secret, enable/disable
        Ok(())
    }

    async fn process(&self, event: &mut PipelineEvent) -> Result<StageFlow> {
        let chain = &event.message.chain;

        match self.engine.first_violation(chain).await {
            Some(reason) => {
                warn!(
                    "[ContentSafetyCheckStage] violation detected for session {}: {}",
                    event.message.session_id, reason
                );
                if self.stop_on_violation {
                    event.stop_event();
                }
                // Store violation reason in extras for downstream logging
                event.set_extra("safety_violation", reason);
            }
            None => {
                // Content is safe — continue
            }
        }

        Ok(StageFlow::Wrap)
    }
}

// ---------------------------------------------------------------------------
// 6. PreProcessStage — voice/STT, path mapping, pre-ack
// ---------------------------------------------------------------------------

pub struct PreProcessStage {
    enable_stt: bool,
    enable_pre_ack: bool,
}

impl Default for PreProcessStage {
    fn default() -> Self {
        Self {
            enable_stt: false,
            enable_pre_ack: false,
        }
    }
}

#[async_trait]
impl Stage for PreProcessStage {
    async fn initialize(&mut self, _ctx: &PipelineContext) -> Result<()> {
        Ok(())
    }

    async fn process(&self, event: &mut PipelineEvent) -> Result<StageFlow> {
        if self.enable_stt {
            let has_voice = event.message.chain.components().iter().any(|c| {
                matches!(c, MessageComponent::Voice { .. })
            });
            if has_voice {
                debug!("[PreProcess] Voice message detected, STT not yet implemented");
            }
        }
        Ok(StageFlow::Done)
    }
}

// ---------------------------------------------------------------------------
// 7. ProcessStage — re-export from process.rs
// ---------------------------------------------------------------------------

// Real implementation in pipeline/process.rs

// ---------------------------------------------------------------------------
// 8. ResultDecorateStage — reply prefix, T2I, TTS, segmented reply, mention/quote
// ---------------------------------------------------------------------------

pub struct ResultDecorateStage {
    reply_prefix: String,
    reply_with_mention: bool,
    reply_with_quote: bool,
    enable_segmented_reply: bool,
    words_count_threshold: usize,
    t2i_enabled: bool,
    t2i_word_threshold: usize,
    tts_enabled: bool,
    tts_trigger_probability: f64,
}

impl Default for ResultDecorateStage {
    fn default() -> Self {
        Self {
            reply_prefix: String::new(),
            reply_with_mention: false,
            reply_with_quote: false,
            enable_segmented_reply: false,
            words_count_threshold: 150,
            t2i_enabled: false,
            t2i_word_threshold: 150,
            tts_enabled: false,
            tts_trigger_probability: 1.0,
        }
    }
}

#[async_trait]
impl Stage for ResultDecorateStage {
    async fn initialize(&mut self, _ctx: &PipelineContext) -> Result<()> {
        Ok(())
    }

    async fn process(&self, event: &mut PipelineEvent) -> Result<StageFlow> {
        // Cache values that need immutable borrow before taking mutable borrow
        let is_group = event.is_group_chat();
        let message_id = event.message.message_id.clone();
        let sender_id = event.message.sender.user_id.clone();
        let sender_nickname = event.message.sender.nickname.clone();

        let chain = match event.result_chain.as_mut() {
            Some(c) => c,
            None => return Ok(StageFlow::Done),
        };

        if chain.components().is_empty() {
            return Ok(StageFlow::Done);
        }

        // 1. Reply prefix
        if !self.reply_prefix.is_empty() {
            for comp in chain.components_mut() {
                if let MessageComponent::Plain { ref mut text } = comp {
                    *text = format!("{}{}", self.reply_prefix, text);
                    break;
                }
            }
        }

        // 2. Segmented reply
        if self.enable_segmented_reply {
            let mut new_chain = MessageChain::new();
            for comp in chain.components() {
                match comp {
                    MessageComponent::Plain { text } => {
                        if text.chars().count() > self.words_count_threshold {
                            let split_pattern = ['。', '？', '！', '~', '…', '.', '?', '!'];
                            let mut current = String::new();
                            for ch in text.chars() {
                                current.push(ch);
                                if split_pattern.contains(&ch) && !current.trim().is_empty() {
                                    new_chain = new_chain.text(current.trim());
                                    current = String::new();
                                }
                            }
                            if !current.trim().is_empty() {
                                new_chain = new_chain.text(current.trim());
                            }
                        } else {
                            new_chain = new_chain.text(text);
                        }
                    }
                    other => {
                        new_chain.0.push(other.clone());
                    }
                }
            }
            *chain = new_chain;
        }

        // 3. TTS / T2I — placeholder
        if self.tts_enabled {
            debug!("[ResultDecorate] TTS not yet integrated");
        }
        if self.t2i_enabled {
            debug!("[ResultDecorate] T2I not yet integrated");
        }

        // 4. Mention / Quote
        if is_group && (self.reply_with_mention || self.reply_with_quote) {
            let mut new_chain = MessageChain::new();
            if self.reply_with_quote {
                new_chain = new_chain.reply(&message_id, None);
            }
            if self.reply_with_mention {
                let display = sender_nickname.unwrap_or_default();
                new_chain.0.push(MessageComponent::At {
                    target: sender_id,
                    display: Some(display),
                });
            }
            new_chain.0.extend(chain.0.clone());
            *chain = new_chain;
        }

        Ok(StageFlow::Done)
    }
}

// ---------------------------------------------------------------------------
// 9. RespondStage — re-export from respond.rs
// ---------------------------------------------------------------------------

// Real implementation in pipeline/respond.rs

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pipeline_scheduler_execution() {
        let ctx = Arc::new(PipelineContext::new());
        let mut registry = StageRegistry::new();

        registry.register("WakingCheckStage", Box::new(WakingCheckStage::default()));
        registry.register("WhitelistCheckStage", Box::new(WhitelistCheckStage::default()));
        registry.register("RateLimitStage", Box::new(RateLimitStage::default()));
        registry.register("ProcessStage", Box::new(ProcessStage::new()));
        registry.register("RespondStage", Box::new(RespondStage::new()));

        registry.initialize_all(&ctx).await.unwrap();

        let scheduler = PipelineScheduler::new(ctx, registry);

        let message = AstrBotMessage::default();
        let mut event = PipelineEvent::new(message);

        scheduler.execute(&mut event).await.unwrap();

        // WakingCheckStage 默认 wake_prefix="/"，空消息不会唤醒，会被 stop
        assert!(event.is_stopped());
    }

    #[tokio::test]
    async fn test_rate_limit_stall() {
        let ctx = Arc::new(PipelineContext::new());
        let mut registry = StageRegistry::new();

        let mut rate_limit = RateLimitStage::default();
        rate_limit.rate_limit_count = 1;
        rate_limit.rate_limit_time = Duration::from_secs(1);
        rate_limit.strategy = RateLimitStrategy::Stall;

        registry.register("RateLimitStage", Box::new(rate_limit));

        registry.initialize_all(&ctx).await.unwrap();
        let scheduler = PipelineScheduler::new(ctx, registry);

        let message = AstrBotMessage::default();
        let mut event = PipelineEvent::new(message);

        let start = Instant::now();
        scheduler.execute(&mut event).await.unwrap();
        let elapsed = start.elapsed();

        // First request passes immediately
        assert!(!event.is_stopped());
        assert!(elapsed < Duration::from_millis(100));
    }

    #[tokio::test]
    async fn test_rate_limit_discard() {
        let ctx = Arc::new(PipelineContext::new());
        let mut registry = StageRegistry::new();

        let mut rate_limit = RateLimitStage::default();
        rate_limit.rate_limit_count = 1;
        rate_limit.rate_limit_time = Duration::from_secs(10);
        rate_limit.strategy = RateLimitStrategy::Discard;

        registry.register("RateLimitStage", Box::new(rate_limit));

        registry.initialize_all(&ctx).await.unwrap();
        let scheduler = PipelineScheduler::new(ctx, registry);

        let message = AstrBotMessage::default();
        let mut event = PipelineEvent::new(message);

        // First request passes
        scheduler.execute(&mut event).await.unwrap();
        assert!(!event.is_stopped());

        // Second request discarded
        let message2 = AstrBotMessage::default();
        let mut event2 = PipelineEvent::new(message2);
        scheduler.execute(&mut event2).await.unwrap();
        assert!(event2.is_stopped());
    }

    #[tokio::test]
    async fn test_content_safety_allows_safe_content() {
        let ctx = Arc::new(PipelineContext::new());
        let mut registry = StageRegistry::new();

        let engine = crate::safety::SafetyEngine::new()
            .add_strategy(Box::new(crate::safety::KeywordFilter::new(
                "keyword",
                vec!["badword".to_string()],
                false,
            )));
        let safety = ContentSafetyCheckStage::with_engine(engine);

        registry.register("ContentSafetyCheckStage", Box::new(safety));
        registry.initialize_all(&ctx).await.unwrap();

        let scheduler = PipelineScheduler::new(ctx, registry);
        let mut message = AstrBotMessage::default();
        message.chain = crate::message::MessageChain::new().text("hello world this is safe");
        let mut event = PipelineEvent::new(message);

        scheduler.execute(&mut event).await.unwrap();
        assert!(!event.is_stopped(), "Safe content should not be stopped");
    }

    #[tokio::test]
    async fn test_content_safety_blocks_violation() {
        let ctx = Arc::new(PipelineContext::new());
        let mut registry = StageRegistry::new();

        let engine = crate::safety::SafetyEngine::new()
            .add_strategy(Box::new(crate::safety::KeywordFilter::new(
                "keyword",
                vec!["badword".to_string()],
                false,
            )));
        let safety = ContentSafetyCheckStage::with_engine(engine);

        registry.register("ContentSafetyCheckStage", Box::new(safety));
        registry.initialize_all(&ctx).await.unwrap();

        let scheduler = PipelineScheduler::new(ctx, registry);
        let mut message = AstrBotMessage::default();
        message.chain = crate::message::MessageChain::new().text("this contains badword in it");
        let mut event = PipelineEvent::new(message);

        scheduler.execute(&mut event).await.unwrap();
        assert!(event.is_stopped(), "Content with blocked keyword should be stopped");
        assert!(
            event.get_extra("safety_violation").is_some(),
            "Violation reason should be recorded in extras"
        );
    }

    #[tokio::test]
    async fn test_content_safety_audit_mode_no_stop() {
        let ctx = Arc::new(PipelineContext::new());
        let mut registry = StageRegistry::new();

        let engine = crate::safety::SafetyEngine::new()
            .add_strategy(Box::new(crate::safety::KeywordFilter::new(
                "keyword",
                vec!["badword".to_string()],
                false,
            )));
        let safety = ContentSafetyCheckStage::with_engine(engine)
            .with_stop_on_violation(false);

        registry.register("ContentSafetyCheckStage", Box::new(safety));
        registry.initialize_all(&ctx).await.unwrap();

        let scheduler = PipelineScheduler::new(ctx, registry);
        let mut message = AstrBotMessage::default();
        message.chain = crate::message::MessageChain::new().text("this contains badword in it");
        let mut event = PipelineEvent::new(message);

        scheduler.execute(&mut event).await.unwrap();
        // In audit mode, event is NOT stopped but violation is recorded
        assert!(!event.is_stopped(), "Audit mode should not stop event");
        assert!(
            event.get_extra("safety_violation").is_some(),
            "Violation reason should still be recorded in audit mode"
        );
    }

    #[tokio::test]
    async fn test_session_status_check_allows_by_default() {
        let stage = SessionStatusCheckStage::default();
        let msg = AstrBotMessage::default();
        let mut event = PipelineEvent::new(msg);
        let flow = stage.process(&mut event).await.unwrap();
        assert!(matches!(flow, StageFlow::Done));
        assert!(!event.is_stopped());
    }

    #[tokio::test]
    async fn test_result_decorate_reply_prefix() {
        let stage = ResultDecorateStage {
            reply_prefix: "[Bot] ".to_string(),
            ..Default::default()
        };
        let msg = AstrBotMessage::default();
        let mut event = PipelineEvent::new(msg);
        event.result_chain = Some(MessageChain::new().text("Hello"));
        stage.process(&mut event).await.unwrap();
        let text = event.result_chain.unwrap().plain_text();
        assert_eq!(text, "[Bot] Hello");
    }

    #[tokio::test]
    async fn test_result_decorate_segmented_reply() {
        let stage = ResultDecorateStage {
            enable_segmented_reply: true,
            words_count_threshold: 5,
            ..Default::default()
        };
        let msg = AstrBotMessage::default();
        let mut event = PipelineEvent::new(msg);
        event.result_chain = Some(MessageChain::new().text("First. Second. Third."));
        stage.process(&mut event).await.unwrap();
        let texts: Vec<String> = event
            .result_chain
            .unwrap()
            .components()
            .iter()
            .filter_map(|c| match c {
                MessageComponent::Plain { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["First.", "Second.", "Third."]);
    }

    #[tokio::test]
    async fn test_result_decorate_mention_and_quote() {
        let mut msg = AstrBotMessage::default();
        msg.session_id = "group123".to_string();
        msg.message_type = MessageType::Group;
        msg.message_id = "msg456".to_string();
        msg.sender.user_id = "user789".to_string();
        msg.sender.nickname = Some("Alice".to_string());
        let mut event = PipelineEvent::new(msg);
        event.result_chain = Some(MessageChain::new().text("Reply"));
        let stage = ResultDecorateStage {
            reply_with_mention: true,
            reply_with_quote: true,
            ..Default::default()
        };
        stage.process(&mut event).await.unwrap();
        let comps = event.result_chain.unwrap().components().to_vec();
        assert!(matches!(comps[0], MessageComponent::Reply { .. }));
        assert!(matches!(comps[1], MessageComponent::At { .. }));
    }
}
