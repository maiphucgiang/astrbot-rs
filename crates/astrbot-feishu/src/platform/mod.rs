//! Feishu platform adapter: message receive/send, event handling

use async_trait::async_trait;
use reqwest::Method;
use serde_json::json;
use tracing::{debug, error, info, warn};

use crate::{
    auth::FeishuAuth, FeishuChat, FeishuError, FeishuUser, IncomingMessage, OutgoingMessage,
    Result, WebhookEvent,
};

/// Configuration for the Feishu adapter
#[derive(Clone, Debug)]
pub struct FeishuAdapterConfig {
    pub bot_name: String,
    pub reply_in_thread: bool,
    pub auto_mark_read: bool,
    pub max_message_length: usize,
}

impl Default for FeishuAdapterConfig {
    fn default() -> Self {
        Self {
            bot_name: "AstrBot".into(),
            reply_in_thread: true,
            auto_mark_read: true,
            max_message_length: 4000,
        }
    }
}

/// Handler trait for incoming messages
#[async_trait]
pub trait MessageHandler: Send + Sync {
    /// Process an incoming message. Return a reply string if applicable.
    async fn handle_message(&self, msg: &IncomingMessage) -> Result<Option<OutgoingMessage>>;
}

/// Feishu platform adapter
#[derive(Clone)]
pub struct FeishuAdapter {
    auth: FeishuAuth,
    config: FeishuAdapterConfig,
}

impl FeishuAdapter {
    pub fn new(auth: FeishuAuth, config: FeishuAdapterConfig) -> Self {
        Self { auth, config }
    }

    /// Send a text message to a chat
    pub async fn send_text(&self, chat_id: &str, text: &str) -> Result<String> {
        let content = json!({ "text": text });
        self.send_message(chat_id, "text", content).await
    }

    /// Send a rich text (post) message
    pub async fn send_post(
        &self,
        chat_id: &str,
        title: &str,
        content: serde_json::Value,
    ) -> Result<String> {
        let body = json!({
            "zh_cn": {
                "title": title,
                "content": content
            }
        });
        self.send_message(chat_id, "post", body).await
    }

    /// Send an interactive card
    pub async fn send_card(&self, chat_id: &str, card: serde_json::Value) -> Result<String> {
        let content = json!({ "card": card });
        self.send_message(chat_id, "interactive", content).await
    }

    /// Core send_message logic
    pub async fn send_message(
        &self,
        chat_id: &str,
        msg_type: &str,
        content: serde_json::Value,
    ) -> Result<String> {
        let body = json!({
            "receive_id_type": "chat_id",
            "receive_id": chat_id,
            "msg_type": msg_type,
            "content": content.to_string(),
        });

        let req = self
            .auth
            .auth_request(Method::POST, "/im/v1/messages")
            .await?;

        let resp = req.json(&body).send().await.map_err(FeishuError::Http)?;

        let api_resp: crate::ApiResponse<serde_json::Value> =
            resp.json().await.map_err(FeishuError::Http)?;

        if api_resp.code != 0 {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        let message_id = api_resp
            .data
            .and_then(|d| {
                d.get("message_id")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
            })
            .unwrap_or_default();

        info!("Message sent to {}, id={}", chat_id, message_id);
        Ok(message_id)
    }

    /// Reply to a specific message (creates thread if configured)
    pub async fn reply_message(
        &self,
        parent_message_id: &str,
        msg_type: &str,
        content: serde_json::Value,
    ) -> Result<String> {
        let body = json!({
            "msg_type": msg_type,
            "content": content.to_string(),
            "reply_in_thread": self.config.reply_in_thread,
        });

        let path = format!("/im/v1/messages/{}/reply", parent_message_id);
        let req = self.auth.auth_request(Method::POST, &path).await?;

        let resp = req.json(&body).send().await.map_err(FeishuError::Http)?;
        let api_resp: crate::ApiResponse<serde_json::Value> =
            resp.json().await.map_err(FeishuError::Http)?;

        if api_resp.code != 0 {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        let message_id = api_resp
            .data
            .and_then(|d| {
                d.get("message_id")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
            })
            .unwrap_or_default();

        Ok(message_id)
    }

    /// Get chat info
    pub async fn get_chat(&self, chat_id: &str) -> Result<FeishuChat> {
        let path = format!("/im/v1/chats/{}", chat_id);
        let req = self.auth.auth_request(Method::GET, &path).await?;
        let resp = req.send().await.map_err(FeishuError::Http)?;

        let api_resp: crate::ApiResponse<FeishuChat> =
            resp.json().await.map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        Ok(api_resp.data.unwrap())
    }

    /// Get user info by open_id
    pub async fn get_user(&self, open_id: &str) -> Result<FeishuUser> {
        let path = format!("/contact/v3/users/{}", open_id);
        let req = self.auth.auth_request(Method::GET, &path).await?;
        let resp = req.send().await.map_err(FeishuError::Http)?;

        let api_resp: crate::ApiResponse<FeishuUser> =
            resp.json().await.map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        Ok(api_resp.data.unwrap())
    }

    /// Parse and verify a webhook event
    pub async fn parse_webhook(
        &self,
        timestamp: &str,
        nonce: &str,
        signature: &str,
        body: &str,
    ) -> Result<Option<WebhookEvent>> {
        let valid = self
            .auth
            .verify_webhook(timestamp, nonce, body, signature)?;
        if !valid {
            warn!("Webhook signature verification failed");
            return Err(FeishuError::WebhookVerify);
        }

        let event: WebhookEvent = serde_json::from_str(body).map_err(FeishuError::Serde)?;
        debug!("Webhook event parsed: {}", event.header.event_type);
        Ok(Some(event))
    }

    /// Process an event with a handler
    pub async fn process_event(
        &self,
        event: &WebhookEvent,
        handler: &dyn MessageHandler,
    ) -> Result<()> {
        match event.header.event_type.as_str() {
            "im.message.receive_v1" => {
                let msg: IncomingMessage =
                    serde_json::from_value(event.event.clone()).map_err(FeishuError::Serde)?;
                info!(
                    "Received message {} from {} in {}",
                    msg.message_id, msg.sender.open_id, msg.chat_id
                );

                if let Some(reply) = handler.handle_message(&msg).await? {
                    self.send_message(&msg.chat_id, &reply.msg_type, reply.content)
                        .await?;
                }
            }
            other => {
                debug!("Unhandled event type: {}", other);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = FeishuAdapterConfig::default();
        assert_eq!(cfg.bot_name, "AstrBot");
        assert!(cfg.reply_in_thread);
    }
}
