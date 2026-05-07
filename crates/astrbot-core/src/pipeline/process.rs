use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

use crate::agent::{AgentContext, AgentRegistry, AgentResult, ToolCall};
use crate::errors::Result;
use crate::message::{AstrBotMessage, MessageChain, MessageEventResult, MessageMember, MessageType};
use crate::pipeline::{PipelineContext, PipelineEvent, Stage, StageFlow};
use crate::platform::{MessageSource, PlatformType};
use crate::plugin::PluginDispatcher;
use crate::provider::{ChatConfig, ChatMessage, ChatResponse, ChatStreamChunk, ModelInfo, Provider};

/// ProcessStage — 核心消息处理阶段
///
/// 职责：
/// 1. 插件消息拦截 (PluginManager::dispatch_message)
/// 2. 组装 messages context（session history + system prompt）
/// 3. Agent Runner 调用（ToolLoop/Coze/Dify/Dashscope/DeerFlow）
/// 4. ChatProvider 路由（LLM 直接调用兜底）
pub struct ProcessStage {
    /// 默认 LLM Provider（兜底路由）
    default_provider: Option<Arc<dyn Provider>>,
    /// Agent 注册表
    agent_registry: Option<Arc<AgentRegistry>>,
    /// 系统提示词
    system_prompt: Option<String>,
    /// 会话历史（简化版：user_id → Vec<ChatMessage>）
    session_history: Arc<tokio::sync::Mutex<HashMap<String, Vec<ChatMessage>>>>,
    /// 默认 Agent ID（优先走 Agent Runner）
    default_agent_id: Option<String>,
    /// 插件管理器（消息拦截）
    plugin_manager: Option<Arc<dyn PluginDispatcher>>,
}

impl ProcessStage {
    pub fn new() -> Self {
        Self {
            default_provider: None,
            agent_registry: None,
            system_prompt: None,
            session_history: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            default_agent_id: None,
            plugin_manager: None,
        }
    }

    /// 设置默认 LLM Provider
    pub fn with_provider(mut self, provider: Arc<dyn Provider>) -> Self {
        self.default_provider = Some(provider);
        self
    }

    /// 设置 Agent 注册表
    pub fn with_agent_registry(mut self, registry: Arc<AgentRegistry>) -> Self {
        self.agent_registry = Some(registry);
        self
    }

    /// 设置系统提示词
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// 设置默认 Agent ID
    pub fn with_default_agent(mut self, agent_id: impl Into<String>) -> Self {
        self.default_agent_id = Some(agent_id.into());
        self
    }

    /// 设置插件管理器
    pub fn with_plugin_manager(mut self, pm: Arc<dyn PluginDispatcher>) -> Self {
        self.plugin_manager = Some(pm);
        self
    }

    /// 从 PipelineEvent 提取用户输入文本
    fn extract_user_input(&self, event: &PipelineEvent) -> String {
        event.message.chain.plain_text()
    }

    /// 尝试插件拦截。如果插件处理了消息，返回 true（跳过 provider）
    async fn try_plugin_intercept(&self, event: &mut PipelineEvent) -> Result<bool> {
        let pm = match self.plugin_manager.as_ref() {
            Some(pm) => pm,
            None => return Ok(false),
        };

        let source = MessageSource {
            platform: event.message.platform,
            session_id: event.message.session_id.clone(),
            message_id: event.message.message_id.clone(),
            user_id: event.message.sender.user_id.clone(),
        };

        let results = pm.dispatch_message(&event.message, &source).await;

        for (id, result) in results {
            match result {
                Ok(MessageEventResult::Reply { chain }) => {
                    info!("[ProcessStage] plugin {} replied", id);
                    event.result_chain = Some(chain);
                    return Ok(true);
                }
                Ok(MessageEventResult::Forward { target, chain }) => {
                    info!("[ProcessStage] plugin {} forwarded to {:?}", id, target);
                    event.result_chain = Some(chain);
                    return Ok(true);
                }
                Ok(MessageEventResult::Nothing) => {}
                Err(e) => {
                    warn!("[ProcessStage] plugin {} error: {}", id, e);
                }
            }
        }

        Ok(false)
    }

    /// 构建 ChatMessage 上下文（history + current）
    async fn build_messages(&self, event: &PipelineEvent) -> Vec<ChatMessage> {
        let mut messages = Vec::new();

        // 1. System prompt
        if let Some(ref prompt) = self.system_prompt {
            messages.push(ChatMessage::system(prompt.clone()));
        }

        // 2. Session history
        let session_id = event.message.session_id.clone();
        let history = {
            let lock = self.session_history.lock().await;
            lock.get(&session_id).cloned().unwrap_or_default()
        };
        messages.extend(history);

        // 3. Current user message
        let user_text = self.extract_user_input(event);
        if !user_text.is_empty() {
            messages.push(ChatMessage::user(user_text));
        }

        messages
    }

    /// 更新会话历史
    async fn update_history(
        &self,
        session_id: &str,
        user_msg: String,
        assistant_msg: String,
    ) {
        let mut lock = self.session_history.lock().await;
        let entry = lock
            .entry(session_id.to_string())
            .or_insert_with(Vec::new);
        entry.push(ChatMessage::user(user_msg));
        entry.push(ChatMessage::assistant(assistant_msg));
        // 保留最近 20 轮
        if entry.len() > 40 {
            *entry = entry.split_off(entry.len() - 40);
        }
    }

    /// 调用 Agent Runner
    async fn run_agent(
        &self,
        agent_id: &str,
        messages: Vec<ChatMessage>,
        source: MessageSource,
        user_id: String,
        session_id: String,
    ) -> Result<String> {
        let registry = self
            .agent_registry
            .as_ref()
            .ok_or_else(|| crate::errors::AstrBotError::Internal("Agent registry not set".to_string()))?;

        let ctx = AgentContext {
            messages,
            source,
            user_id,
            session_id,
            extras: HashMap::new(),
        };

        let result = registry.execute(agent_id, &ctx).await?;

        match result {
            AgentResult::Text { content } => Ok(content),
            AgentResult::ToolCall { calls } => {
                warn!(
                    "Agent requested {} tool calls — not yet handled",
                    calls.len()
                );
                Ok(format!(
                    "[Agent requested {} tools — not yet supported]",
                    calls.len()
                ))
            }
            AgentResult::PassThrough => Ok(String::new()),
            AgentResult::Error { message } => {
                Err(crate::errors::AstrBotError::Internal(message))
            }
        }
    }

    /// 调用 LLM Provider 兜底
    async fn run_provider(&self, messages: Vec<ChatMessage>) -> Result<String> {
        let provider = self
            .default_provider
            .as_ref()
            .ok_or_else(|| crate::errors::AstrBotError::Internal("No provider configured".to_string()))?;

        let config = ChatConfig {
            stream: false,
            ..Default::default()
        };

        let response = provider.chat(messages, config).await?;
        Ok(response.content)
    }
}

#[async_trait]
impl Stage for ProcessStage {
    async fn initialize(&mut self, _ctx: &PipelineContext) -> Result<()> {
        // PipelineContext is currently empty — nothing to bind here.
        // Plugin manager must be set via with_plugin_manager() before pipeline execution.
        Ok(())
    }

    async fn process(&self, event: &mut PipelineEvent) -> Result<StageFlow> {
        let user_input = self.extract_user_input(event);
        if user_input.is_empty() {
            return Ok(StageFlow::Done);
        }

        info!(
            "[ProcessStage] processing message from {}",
            event.message.sender.user_id
        );

        // 1. 插件消息拦截 — 如果插件返回响应，直接跳过 provider
        if self.try_plugin_intercept(event).await? {
            return Ok(StageFlow::Done);
        }

        // 2. 构建消息上下文
        let messages = self.build_messages(event).await;

        // 优先走 Agent Runner
        let response_text = if let Some(ref agent_id) = self.default_agent_id {
            let source = MessageSource {
                platform: event.message.platform,
                session_id: event.message.session_id.clone(),
                message_id: event.message.message_id.clone(),
                user_id: event.message.sender.user_id.clone(),
            };
            match self
                .run_agent(
                    agent_id,
                    messages.clone(),
                    source,
                    event.message.sender.user_id.clone(),
                    event.message.session_id.clone(),
                )
                .await
            {
                Ok(text) => text,
                Err(e) => {
                    warn!(
                        "Agent execution failed: {}, falling back to provider",
                        e
                    );
                    self.run_provider(messages).await?
                }
            }
        } else {
            // 直接走 Provider 兜底
            self.run_provider(messages).await?
        };

        // 组装回复消息链
        if !response_text.is_empty() {
            let reply_chain = MessageChain::new().text(response_text.clone());
            event.result_chain = Some(reply_chain);

            // 更新历史
            self.update_history(
                &event.message.session_id,
                user_input,
                response_text,
            )
            .await;
        }

        Ok(StageFlow::Done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{MessageChain, MessageMember};
    use crate::pipeline::{PipelineContext, PipelineEvent};
    use crate::platform::{MessageSource, PlatformType};
    use crate::provider::{ChatConfig, ChatMessage, ChatResponse, ModelInfo, TokenUsage};
    use async_trait::async_trait;
    use std::sync::Arc;

    /// 模拟 Provider，用于测试
    struct MockProvider {
        response: String,
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn id(&self) -> &str {
            "mock"
        }

        fn name(&self) -> &str {
            "Mock"
        }

        async fn models(&self) -> crate::errors::Result<Vec<String>> {
            Ok(vec!["mock-model".to_string()])
        }

        async fn chat(
            &self,
            _messages: Vec<ChatMessage>,
            _config: ChatConfig,
        ) -> crate::errors::Result<ChatResponse> {
            Ok(ChatResponse {
                content: self.response.clone(),
                model: "mock-model".to_string(),
                usage: None,
                reasoning: None,
                tool_calls: None,
            })
        }

        async fn chat_stream(
            &self,
            _messages: Vec<ChatMessage>,
            _config: ChatConfig,
        ) -> crate::errors::Result<Box<dyn futures_util::Stream<Item = crate::errors::Result<ChatStreamChunk>> + Send>> {
            let chunk = ChatStreamChunk {
                delta: self.response.clone(),
                finish_reason: Some("stop".to_string()),
                model: "mock-model".to_string(),
            };
            let stream = futures_util::stream::iter(vec![Ok(chunk)]);
            Ok(Box::new(stream))
        }

        async fn embedding(
            &self,
            _texts: Vec<String>,
            _model: Option<String>,
        ) -> crate::errors::Result<Vec<Vec<f32>>> {
            Ok(vec![vec![0.0f32; 3]])
        }

        async fn model_info(&self, _model: &str) -> crate::errors::Result<ModelInfo> {
            Ok(ModelInfo {
                name: "mock-model".to_string(),
                context_length: 4096,
                supports_streaming: true,
                supports_vision: false,
                supports_function_calling: false,
            })
        }

        async fn health_check(&self) -> crate::errors::Result<bool> {
            Ok(true)
        }
    }

    fn make_event(text: &str) -> PipelineEvent {
        let message = crate::message::AstrBotMessage {
            message_id: "msg-1".to_string(),
            timestamp: chrono::Utc::now(),
            platform: PlatformType::Custom,
            session_id: "sess-1".to_string(),
            sender: MessageMember {
                user_id: "user-1".to_string(),
                nickname: None,
                card: None,
                role: None,
                is_self: false,
            },
            message_type: crate::message::MessageType::Private,
            chain: MessageChain::new().text(text.to_string()),
            raw_payload: None,
        };
        PipelineEvent::new(message)
    }

    #[tokio::test]
    async fn test_process_stage_with_provider() {
        let provider = Arc::new(MockProvider {
            response: "hello back".to_string(),
        });
        let stage = ProcessStage::new().with_provider(provider);
        let mut event = make_event("hi");

        let result = stage.process(&mut event).await;
        assert!(result.is_ok());
        assert_eq!(
            event.result_chain.as_ref().map(|c| c.plain_text()),
            Some("hello back".to_string())
        );
    }

    #[tokio::test]
    async fn test_process_stage_empty_message() {
        let stage = ProcessStage::new();
        let mut event = make_event("");
        let result = stage.process(&mut event).await;
        assert!(result.is_ok());
        assert!(event.result_chain.is_none());
    }

    #[tokio::test]
    async fn test_process_stage_no_provider() {
        let stage = ProcessStage::new();
        let mut event = make_event("hello");
        let result = stage.process(&mut event).await;
        // Should fail because no provider configured
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_process_stage_history_roundtrip() {
        let provider = Arc::new(MockProvider {
            response: "reply-1".to_string(),
        });
        let stage = ProcessStage::new().with_provider(provider);

        // First message
        let mut event1 = make_event("msg-a");
        let _ = stage.process(&mut event1).await.unwrap();

        // Second message — history should contain first exchange
        let mut event2 = make_event("msg-b");
        let _ = stage.process(&mut event2).await.unwrap();

        let history = {
            let lock = stage.session_history.lock().await;
            lock.get("sess-1").cloned().unwrap_or_default()
        };
        assert!(history.len() >= 4); // user-a + assistant-a + user-b + assistant-b
    }

    #[tokio::test]
    async fn test_process_stage_system_prompt() {
        let provider = Arc::new(MockProvider {
            response: "ok".to_string(),
        });
        let stage = ProcessStage::new()
            .with_provider(provider)
            .with_system_prompt("You are a helpful assistant.");
        let mut event = make_event("test");
        let result = stage.process(&mut event).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_process_stage_empty_message_v2() {
        let provider = Arc::new(MockProvider {
            response: "empty".to_string(),
        });
        let stage = ProcessStage::new().with_provider(provider);
        let mut event = make_event("");
        let result = stage.process(&mut event).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_process_stage_long_message_v2() {
        let provider = Arc::new(MockProvider {
            response: "long".to_string(),
        });
        let stage = ProcessStage::new().with_provider(provider);
        let mut event = make_event("this is a very long message that should still be processed correctly by the stage");
        let result = stage.process(&mut event).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_process_stage_special_chars_v2() {
        let provider = Arc::new(MockProvider {
            response: "special".to_string(),
        });
        let stage = ProcessStage::new().with_provider(provider);
        let mut event = make_event("你好世界 🎉 <script>alert('xss')</script>");
        let result = stage.process(&mut event).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_process_stage_emoji_message() {
        let provider = Arc::new(MockProvider {
            response: "emoji".to_string(),
        });
        let stage = ProcessStage::new().with_provider(provider);
        let mut event = make_event("🔥🚀💯");
        let result = stage.process(&mut event).await;
        assert!(result.is_ok());
    }
}
