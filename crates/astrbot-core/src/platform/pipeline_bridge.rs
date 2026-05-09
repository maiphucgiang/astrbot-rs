use async_trait::async_trait;
use std::sync::Arc;

use crate::errors::Result;
use crate::message::{AstrBotMessage, MessageChain};
use crate::pipeline::{PipelineEvent, PipelineScheduler, StageFlow};
use crate::platform::{MessageSource, PlatformType};

/// PlatformPipelineBridge — 统一平台适配器与 Pipeline 的桥接
///
/// 每个平台适配器收到消息后，通过此 bridge 将消息送入 Pipeline，
/// 处理完成后取 result_chain 发回平台。
pub struct PlatformPipelineBridge {
    pipeline: Arc<PipelineScheduler>,
}

impl PlatformPipelineBridge {
    pub fn new(pipeline: Arc<PipelineScheduler>) -> Self {
        Self { pipeline }
    }

    /// 处理单条消息：Pipeline 执行 → 取 result → 发回平台
    pub async fn handle_message(&self, msg: AstrBotMessage) -> Result<Option<MessageChain>> {
        let mut event = PipelineEvent::new(msg);
        self.pipeline.execute(&mut event).await?;
        Ok(event.result_chain.clone())
    }
}

/// Telegram 适配器集成示例 — 完整消息收发链路
///
/// 使用方式：
/// ```rust
/// let pipeline = runtime.pipeline.as_ref().unwrap().clone();
/// let bridge = PlatformPipelineBridge::new(pipeline);
/// let adapter = TelegramPipelineAdapter::new(token, bridge);
/// adapter.start().await?;
/// ```
#[cfg(feature = "telegram-example")]
pub struct TelegramPipelineAdapter {
    bot_token: String,
    bridge: Arc<PlatformPipelineBridge>,
    http_client: reqwest::Client,
}

#[cfg(feature = "telegram-example")]
impl TelegramPipelineAdapter {
    pub fn new(bot_token: String, bridge: Arc<PlatformPipelineBridge>) -> Self {
        Self {
            bot_token,
            bridge,
            http_client: reqwest::Client::new(),
        }
    }

    pub async fn start(&self) -> Result<()> {
        let mut offset: i64 = 0;
        loop {
            let url = format!(
                "https://api.telegram.org/bot{}/getUpdates?offset={}&limit=10",
                self.bot_token, offset
            );
            let resp = self.http_client.get(&url).send().await?;
            let data: serde_json::Value = resp.json().await?;

            if let Some(results) = data.get("result").and_then(|r| r.as_array()) {
                for update in results {
                    if let Some(msg) = update.get("message") {
                        let astr_msg = self.parse_telegram_message(msg);
                        if let Ok(Some(chain)) = self.bridge.handle_message(astr_msg).await {
                            let chat_id = msg
                                .get("chat")
                                .and_then(|c| c.get("id"))
                                .and_then(|i| i.as_i64())
                                .unwrap_or(0);
                            let reply_text = chain.plain_text();
                            let _ = self.send_telegram_message(chat_id, &reply_text).await;
                        }
                        offset = update
                            .get("update_id")
                            .and_then(|u| u.as_i64())
                            .unwrap_or(offset)
                            + 1;
                    }
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    }

    fn parse_telegram_message(&self, msg: &serde_json::Value) -> AstrBotMessage {
        let text = msg
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        let chat_id = msg
            .get("chat")
            .and_then(|c| c.get("id"))
            .and_then(|i| i.as_i64())
            .unwrap_or(0)
            .to_string();
        let user_id = msg
            .get("from")
            .and_then(|f| f.get("id"))
            .and_then(|i| i.as_i64())
            .unwrap_or(0)
            .to_string();
        let username = msg
            .get("from")
            .and_then(|f| f.get("username"))
            .and_then(|u| u.as_str())
            .map(|s| s.to_string());

        AstrBotMessage {
            message_id: msg
                .get("message_id")
                .and_then(|m| m.as_i64())
                .unwrap_or(0)
                .to_string(),
            timestamp: chrono::Utc::now(),
            platform: PlatformType::Telegram,
            session_id: chat_id.clone(),
            sender: crate::message::MessageMember {
                user_id,
                nickname: username,
                card: None,
                role: None,
                is_self: false,
            },
            message_type: crate::message::MessageType::Private,
            chain: crate::message::MessageChain::new().text(&text),
            raw_payload: Some(msg.clone()),
        }
    }

    async fn send_telegram_message(&self, chat_id: i64, text: &str) -> Result<()> {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.bot_token);
        let _ = self
            .http_client
            .post(&url)
            .json(&serde_json::json!({"chat_id": chat_id, "text": text}))
            .send()
            .await?;
        Ok(())
    }
}
