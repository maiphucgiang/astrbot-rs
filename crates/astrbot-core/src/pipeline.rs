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

use crate::errors::{AstrBotError, Result};
use crate::message::{AstrBotMessage, MessageChain, MessageComponent, MessageType};

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
}

impl PipelineScheduler {
    pub fn new(ctx: Arc<PipelineContext>, registry: StageRegistry) -> Self {
        Self { ctx, registry }
    }

    pub async fn execute(&self, event: &mut PipelineEvent) -> Result<()> {
        self._process_stages(event, 0).await
    }

    async fn _process_stages(&self, event: &mut PipelineEvent, from_stage: usize) -> Result<()> {
        for i in from_stage..self.registry.stages.len() {
            let (name, stage) = &self.registry.stages[i];

            let flow = stage.process(event).await?;

            match flow {
                StageFlow::Wrap => {
                    // Onion model: pre-yield → recurse → post-yield
                    // For now, simplified: recurse into subsequent stages
                    if !event.is_stopped() {
                        self._process_stages(event, i + 1).await?;
                    }
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
        let msg_str = event.message.chain.to_plain_text();
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
        for comp in &event.message.chain.components {
            if let MessageComponent::At { user_id, .. } = comp {
                // TODO: check if at self
                if !self.ignore_at_all || user_id != "all" {
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

pub struct SessionStatusCheckStage;

#[async_trait]
impl Stage for SessionStatusCheckStage {
    async fn initialize(&mut self, _ctx: &PipelineContext) -> Result<()> {
        Ok(())
    }

    async fn process(&self, event: &mut PipelineEvent) -> Result<StageFlow> {
        // TODO: check SessionServiceManager.is_session_enabled(session_id)
        // For now, always enabled
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
                let next_window = timestamps.front().unwrap() + self.rate_limit_time;
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
// 5. ContentSafetyCheckStage — stub
// ---------------------------------------------------------------------------

pub struct ContentSafetyCheckStage;

#[async_trait]
impl Stage for ContentSafetyCheckStage {
    async fn initialize(&mut self, _ctx: &PipelineContext) -> Result<()> {
        Ok(())
    }

    async fn process(&self, _event: &mut PipelineEvent) -> Result<StageFlow> {
        // TODO: StrategySelector.check(text) — Baidu / local
        // Onion model for pre/post check
        Ok(StageFlow::Wrap)
    }
}

// ---------------------------------------------------------------------------
// 6. PreProcessStage — stub
// ---------------------------------------------------------------------------

pub struct PreProcessStage;

#[async_trait]
impl Stage for PreProcessStage {
    async fn initialize(&mut self, _ctx: &PipelineContext) -> Result<()> {
        Ok(())
    }

    async fn process(&self, _event: &mut PipelineEvent) -> Result<StageFlow> {
        // TODO: STT, path mapping, pre_ack_emoji
        Ok(StageFlow::Done)
    }
}

// ---------------------------------------------------------------------------
// 7. ProcessStage — core processing (Star handlers / LLM)
// ---------------------------------------------------------------------------

pub struct ProcessStage;

#[async_trait]
impl Stage for ProcessStage {
    async fn initialize(&mut self, _ctx: &PipelineContext) -> Result<()> {
        // TODO: init AgentRequestSubStage + StarRequestSubStage
        Ok(())
    }

    async fn process(&self, event: &mut PipelineEvent) -> Result<StageFlow> {
        // TODO:
        // 1. Check activated_handlers → StarRequestSubStage
        // 2. No handlers or LLM fallback → AgentRequestSubStage
        // Onion model for pre/post LLM calls

        if event.is_at_or_wake_command && !event.is_stopped() {
            // Placeholder: simulate LLM response
            // event.result_chain = Some(MessageChain::from_plain("Hello from ProcessStage"));
        }

        Ok(StageFlow::Wrap)
    }
}

// ---------------------------------------------------------------------------
// 8. ResultDecorateStage — stub
// ---------------------------------------------------------------------------

pub struct ResultDecorateStage;

#[async_trait]
impl Stage for ResultDecorateStage {
    async fn initialize(&mut self, _ctx: &PipelineContext) -> Result<()> {
        Ok(())
    }

    async fn process(&self, _event: &mut PipelineEvent) -> Result<StageFlow> {
        // TODO: reply_prefix, T2I, TTS, segmented_reply, mention/quote
        Ok(StageFlow::Done)
    }
}

// ---------------------------------------------------------------------------
// 9. RespondStage
// ---------------------------------------------------------------------------

pub struct RespondStage;

#[async_trait]
impl Stage for RespondStage {
    async fn initialize(&mut self, _ctx: &PipelineContext) -> Result<()> {
        Ok(())
    }

    async fn process(&self, _event: &mut PipelineEvent) -> Result<StageFlow> {
        // TODO:
        // 1. Check empty message chain
        // 2. Streaming: send_streaming()
        // 3. Segmented reply: calculate intervals, send segment by segment
        // 4. Record/Video: send separately
        // 5. OnAfterMessageSentEvent hook
        // 6. clear_result()
        Ok(StageFlow::Done)
    }
}

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
        registry.register("ProcessStage", Box::new(ProcessStage));
        registry.register("RespondStage", Box::new(RespondStage));

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
}
