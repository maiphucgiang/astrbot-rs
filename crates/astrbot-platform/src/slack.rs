use crate::adapter::PlatformAdapter;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::message::{
    AstrBotMessage, HandlerRef, MessageChain, MessageComponent, MessageHandler, MessageMember,
    MessageType,
};
use astrbot_core::platform::{MessageSource, PlatformMetadata, PlatformType};
use async_trait::async_trait;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// Slack API models
// https://api.slack.com
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct SlackSocketResponse {
    ok: bool,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SlackEnvelope {
    #[serde(rename = "envelope_id")]
    envelope_id: String,
    #[serde(default)]
    payload: Option<SlackPayload>,
    #[serde(default)]
    #[serde(rename = "type")]
    envelope_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SlackPayload {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    event: Option<SlackEventBody>,
    #[serde(default)]
    #[serde(rename = "channel_id")]
    channel_id: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    #[serde(rename = "client_msg_id")]
    client_msg_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SlackEventBody {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(rename = "channel")]
    channel: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    #[serde(rename = "client_msg_id")]
    client_msg_id: Option<String>,
    #[serde(default)]
    #[serde(rename = "channel_type")]
    channel_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SlackAck {
    #[serde(rename = "envelope_id")]
    envelope_id: String,
}

// ---------------------------------------------------------------------------
// Slack adapter
// ---------------------------------------------------------------------------

pub struct SlackAdapter {
    metadata: PlatformMetadata,
    bot_token: String,
    app_token: String, // xapp-... token for Socket Mode
    http_client: reqwest::Client,
    running: Arc<AtomicBool>,
    ws_task: Option<JoinHandle<()>>,
    message_handler: HandlerRef,
}

impl SlackAdapter {
    pub fn new(bot_token: String, app_token: String) -> Self {
        let metadata = PlatformMetadata {
            id: "slack".to_string(),
            name: "Slack".to_string(),
            platform_type: PlatformType::Slack,
            enabled: true,
            extra: HashMap::new(),
        };

        Self {
            metadata,
            bot_token,
            app_token,
            http_client: reqwest::Client::new(),
            running: Arc::new(AtomicBool::new(false)),
            ws_task: None,
            message_handler: None,
        }
    }

    async fn get_socket_url(&self) -> Result<String> {
        let resp = self
            .http_client
            .post("https://slack.com/api/apps.connections.open")
            .header("Authorization", format!("Bearer {}", self.app_token))
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Slack socket URL request: {}", e)))?;

        let data: SlackSocketResponse = resp
            .json()
            .await
            .map_err(|e| AstrBotError::Serialization(format!("Slack socket URL parse: {}", e)))?;

        if !data.ok {
            return Err(AstrBotError::Platform {
                adapter: "slack".to_string(),
                message: format!("Slack API error: {}", data.error.unwrap_or_default()),
            });
        }

        data.url.ok_or_else(|| AstrBotError::Platform {
            adapter: "slack".to_string(),
            message: "No socket URL in response".to_string(),
        })
    }

    async fn send_channel_message(&self, channel: &str, text: &str) -> Result<()> {
        let body = serde_json::json!({
            "channel": channel,
            "text": text,
        });

        let resp = self
            .http_client
            .post("https://slack.com/api/chat.postMessage")
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Slack send message: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "slack".to_string(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        Ok(())
    }
}

#[async_trait]
impl PlatformAdapter for SlackAdapter {
    fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    async fn initialize(&mut self) -> Result<()> {
        info!("Slack adapter initialized");
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        self.running.store(true, Ordering::SeqCst);

        let app_token = self.app_token.clone();
        let bot_token = self.bot_token.clone();
        let running = Arc::clone(&self.running);
        let handler = self.message_handler.clone();
        let client = self.http_client.clone();

        let task = tokio::spawn(async move {
            let mut retry_count = 0;
            let max_retries = 5;

            while running.load(Ordering::SeqCst) && retry_count < max_retries {
                let url = match Self::get_socket_url_with_client(&client, &app_token).await {
                    Ok(url) => url,
                    Err(e) => {
                        error!("Slack socket URL failed: {}", e);
                        retry_count += 1;
                        sleep(Duration::from_secs(2u64.pow(retry_count.min(5)))).await;
                        continue;
                    }
                };

                match tokio_tungstenite::connect_async(&url).await {
                    Ok((ws_stream, _)) => {
                        info!("Slack Socket Mode connected");
                        retry_count = 0;

                        let (mut write, mut read) = ws_stream.split();

                        while running.load(Ordering::SeqCst) {
                            match tokio::time::timeout(Duration::from_secs(30), read.next()).await {
                                Ok(Some(Ok(msg))) => {
                                    if let tokio_tungstenite::tungstenite::Message::Text(text) = msg
                                    {
                                        if let Ok(envelope) =
                                            serde_json::from_str::<SlackEnvelope>(&text)
                                        {
                                            // Acknowledge every envelope
                                            let ack = SlackAck {
                                                envelope_id: envelope.envelope_id.clone(),
                                            };
                                            let ack_json =
                                                serde_json::to_string(&ack).unwrap_or_default();
                                            let _ = write
                                                .send(
                                                    tokio_tungstenite::tungstenite::Message::Text(
                                                        ack_json.into(),
                                                    ),
                                                )
                                                .await;

                                            // Process message events
                                            if let Some(payload) = envelope.payload {
                                                if payload.event_type == "event_callback" {
                                                    if let Some(event) = payload.event {
                                                        if event.event_type == "message" {
                                                            if let (
                                                                Some(user),
                                                                Some(text),
                                                                Some(channel),
                                                            ) = (
                                                                event.user.as_ref(),
                                                                event.text.as_ref(),
                                                                Some(event.channel.clone()),
                                                            ) {
                                                                let platform = PlatformType::Slack;
                                                                let source = MessageSource {
                                                                    platform,
                                                                    session_id: channel.clone(),
                                                                    message_id: event
                                                                        .client_msg_id
                                                                        .clone()
                                                                        .unwrap_or_default(),
                                                                    user_id: user.clone(),
                                                                };
                                                                let member = MessageMember {
                                                                    user_id: user.clone(),
                                                                    nickname: None,
                                                                    card: None,
                                                                    role: None,
                                                                    is_self: false,
                                                                };
                                                                let message = AstrBotMessage {
                                                                    message_id: event
                                                                        .client_msg_id
                                                                        .clone()
                                                                        .unwrap_or_default(),
                                                                    timestamp: Utc::now(),
                                                                    platform,
                                                                    session_id: channel,
                                                                    sender: member,
                                                                    message_type: match event
                                                                        .channel_type
                                                                        .as_deref()
                                                                    {
                                                                        Some("im") => {
                                                                            MessageType::Private
                                                                        }
                                                                        _ => MessageType::Group,
                                                                    },
                                                                    chain: MessageChain::new()
                                                                        .text(text),
                                                                    raw_payload: Some(
                                                                        serde_json::to_value(
                                                                            &event,
                                                                        )
                                                                        .unwrap_or_default(),
                                                                    ),
                                                                };
                                                                if let Some(ref h) = handler {
                                                                    h.on_message(message).await;
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Ok(Some(Err(e))) => {
                                    error!("Slack WS error: {}", e);
                                    break;
                                }
                                Ok(None) => {
                                    info!("Slack WS closed");
                                    break;
                                }
                                Err(_) => {
                                    warn!("Slack WS heartbeat timeout");
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Slack WS connect failed: {}", e);
                        retry_count += 1;
                        sleep(Duration::from_secs(2u64.pow(retry_count.min(5)))).await;
                    }
                }
            }
        });

        self.ws_task = Some(task);
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        if let Some(task) = self.ws_task.take() {
            task.abort();
        }
        info!("Slack adapter stopped");
        Ok(())
    }

    async fn send_message(&self, target: &MessageSource, chain: &MessageChain) -> Result<()> {
        let content = chain.plain_text();
        self.send_channel_message(&target.session_id, &content)
            .await
    }

    async fn reply_message(&self, original: &AstrBotMessage, chain: &MessageChain) -> Result<()> {
        let content = chain.plain_text();
        self.send_channel_message(&original.session_id, &content)
            .await
    }

    async fn health_check(&self) -> Result<bool> {
        match self.get_socket_url().await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    fn set_message_handler(&mut self, handler: Arc<dyn MessageHandler>) {
        self.message_handler = Some(handler);
    }
}

impl SlackAdapter {
    // Helper for the async task — static method that doesn't need self
    async fn get_socket_url_with_client(
        client: &reqwest::Client,
        app_token: &str,
    ) -> Result<String> {
        let resp = client
            .post("https://slack.com/api/apps.connections.open")
            .header("Authorization", format!("Bearer {}", app_token))
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Slack socket URL request: {}", e)))?;

        let data: SlackSocketResponse = resp
            .json()
            .await
            .map_err(|e| AstrBotError::Serialization(format!("Slack socket URL parse: {}", e)))?;

        if !data.ok {
            return Err(AstrBotError::Platform {
                adapter: "slack".to_string(),
                message: format!("Slack API error: {}", data.error.unwrap_or_default()),
            });
        }

        data.url.ok_or_else(|| AstrBotError::Platform {
            adapter: "slack".to_string(),
            message: "No socket URL in response".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_slack_adapter_new() {
        let adapter = SlackAdapter::new("xoxb-test".to_string(), "xapp-test".to_string());
        assert_eq!(adapter.metadata.name, "Slack");
        assert!(adapter.metadata.enabled);
    }
}
