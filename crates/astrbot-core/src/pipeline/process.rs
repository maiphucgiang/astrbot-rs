use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

use crate::agent::{AgentContext, AgentRegistry, AgentResult, ToolCall};
use crate::errors::Result;
use crate::message::{AstrBotMessage, MessageChain};
use crate::pipeline::{PipelineContext, PipelineEvent, Stage, StageFlow};
use crate::platform::{MessageSource, PlatformType};
use crate::provider::{ChatConfig, ChatMessage, Provider};

/// ProcessStage — 核心消息处理阶段
///
/// 职责：
/// 1. 组装 messages context（session history + system prompt）
/// 2. Agent Runner 调用（ToolLoop/Coze/Dify/Dashscope/DeerFlow）
/// 3. ChatProvider 路由（LLM 直接调用兜底）
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
}

impl ProcessStage {
    pub fn new() -> Self {
        Self {
            default_provider: None,
            agent_registry: None,
            system_prompt: None,
            session_history: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            default_agent_id: None,
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

    /// 从 PipelineEvent 提取用户输入文本
    fn extract_user_input(&self, event: &PipelineEvent) -> String {
        event.message.chain.plain_text()
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

        // 构建消息上下文
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
    use crate::message::{MessageMember, MessageType};
    use crate::platform::{MessageSource, PlatformType};
    use crate::provider::{ChatResponse, ModelInfo, TokenUsage};
    use crate::testing::MockProvider;
    use chrono::Utc;

    fn make_test_event(text: &str) -> PipelineEvent {
        let msg = AstrBotMessage {
            message_id: "msg-1".to_string(),
            timestamp: Utc::now(),
            platform: PlatformType::Custom,
            session_id: "sess-1".to_string(),
            sender: MessageMember {
                user_id: "user-1".to_string(),
                nickname: Some("Test".to_string()),
                card: None,
                role: None,
                is_self: false,
            },
            message_type: MessageType::Text,
            chain: MessageChain::new().text(text),
            raw_payload: None,
        };
        PipelineEvent::new(msg)
    }

    #[tokio::test]
    async fn test_process_stage_provider_fallback() {
        let provider = Arc::new(MockProvider::new(vec!["Hello from mock!".to_string()]));
        let stage = ProcessStage::new().with_provider(provider);

        let mut event = make_test_event("Hi");
        let flow = stage.process(&mut event).await.unwrap();

        assert!(matches!(flow, StageFlow::Done));
        assert!(event.result_chain.is_some());
        assert_eq!(
            event.result_chain.unwrap().plain_text(),
            "Hello from mock!"
        );
    }

    #[tokio::test]
    async fn test_process_stage_empty_message() {
        let stage = ProcessStage::new();
        let mut event = make_test_event("");
        let flow = stage.process(&mut event).await.unwrap();
        assert!(matches!(flow, StageFlow::Done));
        assert!(event.result_chain.is_none());
    }

    #[tokio::test]
    async fn test_process_stage_history_roundtrip() {
        let provider = Arc::new(MockProvider::new(vec![
            "Reply 1".to_string(),
            "Reply 2".to_string(),
        ]));
        let stage = ProcessStage::new().with_provider(provider);

        let mut event1 = make_test_event("Message 1");
        stage.process(&mut event1).await.unwrap();

        let mut event2 = make_test_event("Message 2");
        // 使用相同 session_id，历史应该保留
        stage.process(&mut event2).await.unwrap();

        let history = stage.session_history.lock().await;
        let sess = history.get("sess-1").unwrap();
        assert!(sess.len() >= 2);
    }
}
