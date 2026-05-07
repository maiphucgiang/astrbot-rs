use astrbot_core::pipeline::{
    PipelineContext, PipelineEvent, PipelineScheduler, Stage, StageFlow, StageRegistry,
    WakingCheckStage,
};
use astrbot_core::message::{AstrBotMessage, MessageChain, MessageMember, MessageType};
use astrbot_core::platform::PlatformType;
use astrbot_core::provider::{ChatConfig, ChatMessage, Provider};
use astrbot_core::testing::MockProvider;
use async_trait::async_trait;
use std::sync::Arc;
use std::sync::Mutex;

// 自定义 ProcessStage：使用 MockProvider 生成 LLM 回复
struct E2eProcessStage {
    provider: Arc<dyn Provider>,
}

#[async_trait]
impl Stage for E2eProcessStage {
    async fn initialize(&mut self, _ctx: &PipelineContext) -> astrbot_core::errors::Result<()> {
        Ok(())
    }

    async fn process(
        &self,
        event: &mut PipelineEvent,
    ) -> astrbot_core::errors::Result<StageFlow> {
        let user_text = event.message.chain.plain_text();
        let messages = vec![ChatMessage::user(user_text)];
        let config = ChatConfig {
            model: None,
            temperature: None,
            max_tokens: None,
            top_p: None,
            stream: false,
            extra: std::collections::HashMap::new(),
        };
        let response = self.provider.as_ref().chat(messages, config).await?;
        event.result_chain = Some(MessageChain::new().text(response.content));
        Ok(StageFlow::Done)
    }
}

// 自定义 RespondStage：捕获输出到共享状态
struct E2eRespondStage {
    outputs: Arc<Mutex<Vec<MessageChain>>>,
}

#[async_trait]
impl Stage for E2eRespondStage {
    async fn initialize(&mut self, _ctx: &PipelineContext) -> astrbot_core::errors::Result<()> {
        Ok(())
    }

    async fn process(
        &self,
        event: &mut PipelineEvent,
    ) -> astrbot_core::errors::Result<StageFlow> {
        if let Some(ref chain) = event.result_chain {
            self.outputs.lock().unwrap().push(chain.clone());
        }
        Ok(StageFlow::Done)
    }
}

#[tokio::test]
async fn test_e2e_basic_chat() {
    // 1. MockProvider 配置为回复 "hi"
    let provider = Arc::new(MockProvider::new("mock", "Mock").with_chat_response("hi"));

    // 2. 共享输出捕获
    let outputs = Arc::new(Mutex::new(Vec::new()));

    // 3. 组装 9-stage pipeline（核心链路：WakingCheck → Process → Respond）
    let ctx = Arc::new(PipelineContext::new());
    let mut registry = StageRegistry::new();
    registry.register("WakingCheckStage", Box::new(WakingCheckStage::default()));
    registry.register("ProcessStage", Box::new(E2eProcessStage { provider }));
    registry.register("RespondStage", Box::new(E2eRespondStage {
        outputs: outputs.clone(),
    }));

    registry.initialize_all(&ctx).await.unwrap();
    let scheduler = PipelineScheduler::new(ctx, registry);

    // 4. 构造 "hello" 消息（私聊自动唤醒）
    let message = AstrBotMessage {
        message_id: "msg-1".to_string(),
        timestamp: chrono::Utc::now(),
        platform: PlatformType::Custom,
        session_id: "session-1".to_string(),
        sender: MessageMember {
            user_id: "user-1".to_string(),
            nickname: None,
            card: None,
            role: None,
            is_self: false,
        },
        message_type: MessageType::Private,
        chain: MessageChain::new().text("hello"),
        raw_payload: None,
    };

    let mut event = PipelineEvent::new(message);
    scheduler.execute(&mut event).await.unwrap();

    // 5. 断言：最终响应是 "hi"
    let captured = outputs.lock().unwrap();
    assert_eq!(captured.len(), 1, "应该有一条输出消息");
    assert_eq!(captured[0].plain_text(), "hi", "响应内容应该是 'hi'");
}

#[tokio::test]
async fn test_e2e_with_plugin_intercept() {
    let provider = Arc::new(MockProvider::new("mock", "Mock").with_chat_response("intercepted"));
    let outputs = Arc::new(Mutex::new(Vec::new()));

    let ctx = Arc::new(PipelineContext::new());
    let mut registry = StageRegistry::new();
    registry.register("WakingCheckStage", Box::new(WakingCheckStage::default()));
    registry.register("ProcessStage", Box::new(E2eProcessStage { provider }));
    registry.register("RespondStage", Box::new(E2eRespondStage {
        outputs: outputs.clone(),
    }));

    registry.initialize_all(&ctx).await.unwrap();
    let scheduler = PipelineScheduler::new(ctx, registry);

    let message = AstrBotMessage {
        message_id: "msg-2".to_string(),
        timestamp: chrono::Utc::now(),
        platform: PlatformType::Custom,
        session_id: "session-2".to_string(),
        sender: MessageMember {
            user_id: "user-2".to_string(),
            nickname: None,
            card: None,
            role: None,
            is_self: false,
        },
        message_type: MessageType::Private,
        chain: MessageChain::new().text("trigger"),
        raw_payload: None,
    };

    let mut event = PipelineEvent::new(message);
    scheduler.execute(&mut event).await.unwrap();

    let captured = outputs.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].plain_text(), "intercepted");
}

#[tokio::test]
async fn test_e2e_empty_message() {
    let provider = Arc::new(MockProvider::new("mock", "Mock").with_chat_response("empty"));
    let outputs = Arc::new(Mutex::new(Vec::new()));

    let ctx = Arc::new(PipelineContext::new());
    let mut registry = StageRegistry::new();
    registry.register("WakingCheckStage", Box::new(WakingCheckStage::default()));
    registry.register("ProcessStage", Box::new(E2eProcessStage { provider }));
    registry.register("RespondStage", Box::new(E2eRespondStage {
        outputs: outputs.clone(),
    }));

    registry.initialize_all(&ctx).await.unwrap();
    let scheduler = PipelineScheduler::new(ctx, registry);

    let message = AstrBotMessage {
        message_id: "msg-3".to_string(),
        timestamp: chrono::Utc::now(),
        platform: PlatformType::Custom,
        session_id: "session-3".to_string(),
        sender: MessageMember {
            user_id: "user-3".to_string(),
            nickname: None,
            card: None,
            role: None,
            is_self: false,
        },
        message_type: MessageType::Private,
        chain: MessageChain::new(),
        raw_payload: None,
    };

    let mut event = PipelineEvent::new(message);
    scheduler.execute(&mut event).await.unwrap();

    let captured = outputs.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].plain_text(), "empty");
}

#[tokio::test]
async fn test_e2e_group_message() {
    let provider = Arc::new(MockProvider::new("mock", "Mock").with_chat_response("group-reply"));
    let outputs = Arc::new(Mutex::new(Vec::new()));

    let ctx = Arc::new(PipelineContext::new());
    let mut registry = StageRegistry::new();
    registry.register("WakingCheckStage", Box::new(WakingCheckStage::default()));
    registry.register("ProcessStage", Box::new(E2eProcessStage { provider }));
    registry.register("RespondStage", Box::new(E2eRespondStage {
        outputs: outputs.clone(),
    }));

    registry.initialize_all(&ctx).await.unwrap();
    let scheduler = PipelineScheduler::new(ctx, registry);

    let message = AstrBotMessage {
        message_id: "msg-4".to_string(),
        timestamp: chrono::Utc::now(),
        platform: PlatformType::Custom,
        session_id: "session-4".to_string(),
        sender: MessageMember {
            user_id: "user-4".to_string(),
            nickname: None,
            card: None,
            role: None,
            is_self: false,
        },
        message_type: MessageType::Private,
        chain: MessageChain::new().text("group-hello"),
        raw_payload: None,
    };

    let mut event = PipelineEvent::new(message);
    scheduler.execute(&mut event).await.unwrap();

    let captured = outputs.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].plain_text(), "group-reply");
}
