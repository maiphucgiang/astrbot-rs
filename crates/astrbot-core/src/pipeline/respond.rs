use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::errors::Result;
use crate::message::{MessageChain, MessageComponent};
use crate::pipeline::{PipelineContext, PipelineEvent, Stage, StageFlow};
use crate::platform::PlatformType;

/// 发送函数类型：异步发送消息到指定来源
pub type SendFn = Arc<
    dyn Fn(
            crate::platform::MessageSource,
            MessageChain,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

/// RespondStage — 消息回复阶段
///
/// 职责：
/// 1. 检查 result_chain 是否为空
/// 2. 通过回调函数发送消息
/// 3. 失败重试逻辑（max 3 次，指数退避）
/// 4. 消息格式化（Markdown → 平台特定格式）
pub struct RespondStage {
    /// 发送回调
    sender: Option<SendFn>,
    /// 最大重试次数
    max_retries: u32,
    /// 基础退避时长（ms）
    base_backoff_ms: u64,
    /// 是否启用 Markdown 格式化
    enable_markdown_formatting: bool,
}

impl RespondStage {
    pub fn new() -> Self {
        Self {
            sender: None,
            max_retries: 3,
            base_backoff_ms: 500,
            enable_markdown_formatting: true,
        }
    }

    /// 设置发送回调
    pub fn with_sender(mut self, sender: SendFn) -> Self {
        self.sender = Some(sender);
        self
    }

    /// 设置重试策略
    pub fn with_retry_policy(mut self, max_retries: u32, base_backoff_ms: u64) -> Self {
        self.max_retries = max_retries;
        self.base_backoff_ms = base_backoff_ms;
        self
    }

    /// 消息格式化：根据平台特性做转换
    fn format_for_platform(&self, chain: &MessageChain, platform: PlatformType) -> MessageChain {
        if !self.enable_markdown_formatting {
            return chain.clone();
        }

        let mut formatted = MessageChain::new();
        for comp in chain.components() {
            match comp {
                MessageComponent::Plain { text, .. } => {
                    let converted = match platform {
                        PlatformType::Aiocqhttp => markdown_to_qq(text),
                        PlatformType::Weixin => markdown_to_wechat(text),
                        PlatformType::Discord => text.clone(),
                        _ => text.clone(),
                    };
                    formatted = formatted.text(converted);
                }
                other => {
                    formatted.0.push(other.clone());
                }
            }
        }
        formatted
    }

    /// 指数退避：计算第 n 次重试的等待时间
    fn backoff_duration(&self, attempt: u32) -> Duration {
        let ms = self.base_backoff_ms * 2u64.pow(attempt);
        Duration::from_millis(ms.min(8000)) // 上限 8s
    }

    /// 发送消息（带重试）。sender 未配置时 graceful degradation，返回 Ok。
    async fn send_with_retry(
        &self,
        source: crate::platform::MessageSource,
        chain: MessageChain,
    ) -> Result<()> {
        let sender = match self.sender.as_ref() {
            Some(s) => s,
            None => {
                warn!("[RespondStage] Sender not configured — skipping send (graceful)");
                return Ok(());
            }
        };

        for attempt in 0..=self.max_retries {
            let result = (sender)(source.clone(), chain.clone()).await;

            match result {
                Ok(()) => {
                    if attempt > 0 {
                        info!("Message sent successfully after {} retries", attempt);
                    }
                    return Ok(());
                }
                Err(e) => {
                    if attempt < self.max_retries {
                        let wait = self.backoff_duration(attempt);
                        warn!(
                            "Send failed (attempt {}/{}): {}, retrying in {:?}",
                            attempt + 1,
                            self.max_retries + 1,
                            e,
                            wait
                        );
                        sleep(wait).await;
                    } else {
                        warn!(
                            "Send failed after {} attempts, giving up: {}",
                            self.max_retries + 1,
                            e
                        );
                        return Err(crate::errors::AstrBotError::Internal(format!(
                            "Failed to send message after {} retries: {}",
                            self.max_retries + 1,
                            e
                        )));
                    }
                }
            }
        }

        Ok(())
    }
}

/// 简化 Markdown → QQ 文本转换
fn markdown_to_qq(text: &str) -> String {
    text.replace("**", "")
        .replace("*", "")
        .replace("`", "")
        .replace("```", "")
        .replace("# ", "")
        .replace("## ", "")
        .replace("### ", "")
}

/// 简化 Markdown → WeChat 文本转换
fn markdown_to_wechat(text: &str) -> String {
    text.replace("**", "*").replace("`", "'").replace("```", "")
}

#[async_trait]
impl Stage for RespondStage {
    async fn initialize(&mut self, _ctx: &PipelineContext) -> Result<()> {
        Ok(())
    }

    async fn process(&self, event: &mut PipelineEvent) -> Result<StageFlow> {
        // 1. 检查 result_chain
        let chain = match &event.result_chain {
            Some(chain) if !chain.components().is_empty() => chain.clone(),
            _ => {
                info!("[RespondStage] no result to send");
                return Ok(StageFlow::Done);
            }
        };

        info!(
            "[RespondStage] sending response to {} on {:?}",
            event.message.sender.user_id, event.message.platform
        );

        // 2. 格式化
        let formatted = self.format_for_platform(&chain, event.message.platform);

        // 3. 发送（带重试）。失败不 panic，graceful degradation。
        let source = crate::platform::MessageSource {
            platform: event.message.platform,
            session_id: event.message.session_id.clone(),
            message_id: event.message.message_id.clone(),
            user_id: event.message.sender.user_id.clone(),
        };
        if let Err(e) = self.send_with_retry(source, formatted).await {
            warn!(
                "[RespondStage] Send failed after all retries: {}. Message dropped gracefully.",
                e
            );
        }

        Ok(StageFlow::Done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{AstrBotMessage, MessageMember, MessageType};
    use crate::platform::{MessageSource, PlatformType};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn make_test_event(text: &str) -> PipelineEvent {
        let msg = AstrBotMessage {
            message_id: "m".to_string(),
            timestamp: chrono::Utc::now(),
            platform: PlatformType::Custom,
            session_id: "s".to_string(),
            sender: MessageMember {
                user_id: "u".to_string(),
                nickname: None,
                card: None,
                role: None,
                is_self: false,
            },
            message_type: MessageType::Private,
            chain: MessageChain::new(),
            raw_payload: None,
        };
        PipelineEvent::new(msg)
    }

    #[tokio::test]
    async fn test_respond_stage_no_sender_graceful() {
        // sender 未配置时应该 graceful degradation，不 panic，不 Err
        let stage = RespondStage::new(); // 没有 with_sender
        let mut event = make_test_event("trigger");
        event.result_chain = Some(MessageChain::new().text("Hello!"));

        let flow = stage.process(&mut event).await.unwrap();
        assert!(matches!(flow, StageFlow::Done));
    }

    #[tokio::test]
    async fn test_respond_stage_empty_chain() {
        let stage = RespondStage::new();
        let mut event = make_test_event("");
        let flow = stage.process(&mut event).await.unwrap();
        assert!(matches!(flow, StageFlow::Done));
    }

    #[tokio::test]
    async fn test_respond_stage_sends_message() {
        let sent = Arc::new(AtomicUsize::new(0));
        let sent_clone = sent.clone();

        let sender: SendFn = Arc::new(move |_src, _chain| {
            let sent = sent_clone.clone();
            Box::pin(async move {
                sent.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        });

        let stage = RespondStage::new().with_sender(sender);
        let mut event = make_test_event("trigger");
        event.result_chain = Some(MessageChain::new().text("Hello!"));

        let flow = stage.process(&mut event).await.unwrap();
        assert!(matches!(flow, StageFlow::Done));
        assert_eq!(sent.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_respond_stage_retry_then_success() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_clone = attempts.clone();

        let sender: SendFn = Arc::new(move |_src, _chain| {
            let attempts = attempts_clone.clone();
            Box::pin(async move {
                let n = attempts.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    Err(crate::errors::AstrBotError::Internal(
                        "network error".to_string(),
                    ))
                } else {
                    Ok(())
                }
            })
        });

        let stage = RespondStage::new()
            .with_sender(sender)
            .with_retry_policy(3, 10); // 10ms base for fast test

        let mut event = make_test_event("trigger");
        event.result_chain = Some(MessageChain::new().text("Retry me"));

        let flow = stage.process(&mut event).await.unwrap();
        assert!(matches!(flow, StageFlow::Done));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_backoff_duration() {
        let stage = RespondStage::new().with_retry_policy(3, 500);
        assert_eq!(stage.backoff_duration(0), Duration::from_millis(500));
        assert_eq!(stage.backoff_duration(1), Duration::from_millis(1000));
        assert_eq!(stage.backoff_duration(2), Duration::from_millis(2000));
        assert_eq!(stage.backoff_duration(10), Duration::from_millis(8000)); // capped
    }

    #[test]
    fn test_markdown_to_qq() {
        let input = "**bold** and `code`\n# header";
        assert_eq!(markdown_to_qq(input), "bold and code\nheader");
    }

    #[test]
    fn test_markdown_to_wechat() {
        let input = "**bold** and `code`";
        assert_eq!(markdown_to_wechat(input), "*bold* and 'code'");
    }
}
