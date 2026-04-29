//! Coze (扣子) Agent Runner — API 封装 + SSE 流式 + 会话 KV 存储
//!
//! Coze API docs: https://www.coze.cn/docs/developer_guides/chat_v3
//!
//! 支持两种模式：
//! 1. 非流式：POST /v3/chat → 轮询对话详情
//! 2. SSE 流式：POST /v3/chat/stream → 逐字返回
//!
//! 会话 KV：Coze 的 conversation_id 自动维护，本地缓存 user_id → conversation_id 映射。

use async_trait::async_trait;
use crate::errors::{AstrBotError, Result};
use crate::message::MessageChain;
use crate::platform::MessageSource;
use crate::provider::{ChatMessage, ChatConfig};
use super::{AgentContext, AgentResult, AgentExecutor, AgentConfig, ToolResult};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// Coze API data models
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
struct CozeChatRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    conversation_id: Option<String>,
    bot_id: String,
    user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    additional_messages: Option<Vec<CozeMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "custom_variables")]
    custom_variables: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CozeMessage {
    #[serde(rename = "role")]
    role: String,
    #[serde(rename = "content")]
    content: String,
    #[serde(rename = "content_type", skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CozeChatResponse {
    code: i32,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    data: Option<CozeChatData>,
}

#[derive(Debug, Clone, Deserialize)]
struct CozeChatData {
    #[serde(default)]
    conversation_id: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    status: String, // "completed", "failed", "in_progress"
    #[serde(default)]
    answer: String,
    #[serde(default)]
    messages: Vec<CozeMessage>,
}

// SSE 事件
#[derive(Debug, Clone, Deserialize)]
struct CozeSseEvent {
    #[serde(default)]
    event: String,
    #[serde(default)]
    data: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CozeSseData {
    #[serde(default)]
    id: String,
    #[serde(default)]
    conversation_id: String,
    #[serde(default)]
    bot_id: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    #[serde(rename = "type")]
    msg_type: String,
}

// ---------------------------------------------------------------------------
// Session KV store
// ---------------------------------------------------------------------------

/// Thread-safe in-memory conversation_id cache.
/// Production: replace with Redis / SQLite / etc.
#[derive(Clone, Default)]
pub struct SessionStore {
    inner: Arc<RwLock<HashMap<String, String>>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get conversation_id for user_id. Returns None if not cached.
    pub async fn get(&self, user_id: &str) -> Option<String> {
        let guard = self.inner.read().await;
        guard.get(user_id).cloned()
    }

    /// Set conversation_id for user_id.
    pub async fn set(&self, user_id: &str, conversation_id: &str) {
        let mut guard = self.inner.write().await;
        guard.insert(user_id.to_string(), conversation_id.to_string());
    }

    /// Remove entry.
    pub async fn remove(&self, user_id: &str) {
        let mut guard = self.inner.write().await;
        guard.remove(user_id);
    }
}

// ---------------------------------------------------------------------------
// Coze client
// ---------------------------------------------------------------------------

pub struct CozeClient {
    base_url: String,
    api_token: String,
    bot_id: String,
    http_client: reqwest::Client,
    session_store: SessionStore,
}

impl CozeClient {
    pub fn new(api_token: String, bot_id: String) -> Self {
        Self {
            base_url: "https://api.coze.cn".to_string(),
            api_token,
            bot_id,
            http_client: reqwest::Client::new(),
            session_store: SessionStore::new(),
        }
    }

    pub fn set_base_url(&mut self, url: String) {
        self.base_url = url;
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    fn auth_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", self.api_token)
            ).unwrap_or_else(|_| reqwest::header::HeaderValue::from_static("")),
        );
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        headers
    }

    /// Non-streaming chat: create task → poll until done.
    pub async fn chat(
        &self,
        user_id: &str,
        messages: Vec<CozeMessage>,
        custom_vars: Option<HashMap<String, String>>,
    ) -> Result<CozeChatData> {
        let conversation_id = self.session_store.get(user_id).await;

        let body = CozeChatRequest {
            conversation_id: conversation_id.clone(),
            bot_id: self.bot_id.clone(),
            user_id: user_id.to_string(),
            additional_messages: Some(messages),
            custom_variables: custom_vars,
        };

        let url = format!("{}/v3/chat", self.base_url);
        let resp = self.http_client
            .post(&url)
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Coze chat request failed: {}", e)))?;

        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(AstrBotError::Platform {
                adapter: "Coze".to_string(),
                message: format!("Coze API error {}: {}", status, body_text),
            });
        }

        let parsed: CozeChatResponse = serde_json::from_str(&body_text)
            .map_err(|e| AstrBotError::Serialization(format!("Coze parse failed: {} — body: {}", e, body_text)))?;

        if parsed.code != 0 {
            return Err(AstrBotError::Platform {
                adapter: "Coze".to_string(),
                message: format!("Coze API code={} msg={}", parsed.code, parsed.msg),
            });
        }

        let chat_data = parsed.data.ok_or_else(|| {
            AstrBotError::Platform {
                adapter: "Coze".to_string(),
                message: "Coze response missing data".to_string(),
            }
        })?;

        // Cache conversation_id
        if !chat_data.conversation_id.is_empty() {
            self.session_store.set(user_id, &chat_data.conversation_id).await;
        }

        // Poll if still in progress
        if chat_data.status == "in_progress" {
            let final_data = self.poll_chat(&chat_data.conversation_id, &chat_data.id, user_id).await?;
            return Ok(final_data);
        }

        Ok(chat_data)
    }

    /// Poll chat detail until completed or failed.
    async fn poll_chat(
        &self,
        conversation_id: &str,
        chat_id: &str,
        user_id: &str,
    ) -> Result<CozeChatData> {
        let url = format!("{}/v3/chat/retrieve", self.base_url);
        let body = json!({
            "conversation_id": conversation_id,
            "chat_id": chat_id,
        });

        for attempt in 0..30 {
            sleep(Duration::from_secs(1)).await;

            let resp = self.http_client
                .post(&url)
                .headers(self.auth_headers())
                .json(&body)
                .send()
                .await
                .map_err(|e| AstrBotError::Network(format!("Coze poll failed: {}", e)))?;

            let body_text = resp.text().await.unwrap_or_default();
            let parsed: CozeChatResponse = serde_json::from_str(&body_text)
                .map_err(|e| AstrBotError::Serialization(format!("Coze poll parse failed: {}", e)))?;

            if parsed.code != 0 {
                return Err(AstrBotError::Platform {
                    adapter: "Coze".to_string(),
                    message: format!("Coze poll error code={} msg={}", parsed.code, parsed.msg),
                });
            }

            if let Some(data) = parsed.data {
                if data.status == "completed" || data.status == "failed" {
                    if !data.conversation_id.is_empty() {
                        self.session_store.set(user_id, &data.conversation_id).await;
                    }
                    return Ok(data);
                }
            }

            if attempt % 5 == 0 {
                info!("[Coze] Polling chat {} (attempt {})", chat_id, attempt);
            }
        }

        Err(AstrBotError::Platform {
            adapter: "Coze".to_string(),
            message: "Coze chat polling timeout".to_string(),
        })
    }

    /// SSE streaming chat.
    pub async fn chat_stream(
        &self,
        user_id: &str,
        messages: Vec<CozeMessage>,
        custom_vars: Option<HashMap<String, String>>,
    ) -> Result<impl futures_util::Stream<Item = Result<String>> + '_> {
        let conversation_id = self.session_store.get(user_id).await;

        let body = CozeChatRequest {
            conversation_id,
            bot_id: self.bot_id.clone(),
            user_id: user_id.to_string(),
            additional_messages: Some(messages),
            custom_variables: custom_vars,
        };

        let url = format!("{}/v3/chat/stream", self.base_url);
        let resp = self.http_client
            .post(&url)
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Coze stream request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Platform {
                adapter: "Coze".to_string(),
                message: format!("Coze stream error {}: {}", status, body),
            });
        }

        let byte_stream = resp.bytes_stream();
        let line_stream = byte_stream
            .map(|chunk| chunk.map_err(|e| AstrBotError::Network(e.to_string())))
            .map(|res| res.map(|bytes| String::from_utf8_lossy(&bytes).to_string()));

        let event_stream = futures_util::stream::try_unfold(
            (line_stream, String::new()),
            |(mut stream, mut buf)| async move {
                loop {
                    match stream.next().await {
                        Some(Ok(line)) => {
                            buf.push_str(&line);
                            if line.ends_with('\n') {
                                // Try to parse SSE event from buffer
                                let content = parse_sse_buffer(&buf)?;
                                buf.clear();
                                if let Some(c) = content {
                                    return Ok(Some((c, (stream, buf))));
                                }
                            }
                        }
                        Some(Err(e)) => return Err(e),
                        None => {
                            if buf.is_empty() {
                                return Ok(None);
                            }
                            let content = parse_sse_buffer(&buf)?;
                            return Ok(content.map(|c| (c, (stream, String::new()))));
                        }
                    }
                }
            },
        );

        Ok(event_stream)
    }
}

/// Parse SSE buffer into Coze content text.
fn parse_sse_buffer(buf: &str) -> Result<Option<String>> {
    let mut content_parts = Vec::new();
    let mut conversation_id: Option<String> = None;

    for line in buf.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("event:") {
            // event line, ignore for now
        } else if line.starts_with("data:") {
            let data_str = line.trim_start_matches("data:").trim();
            if data_str.is_empty() {
                continue;
            }
            // Try parse as JSON
            if let Ok(event_data) = serde_json::from_str::<CozeSseData>(data_str) {
                if !event_data.content.is_empty() {
                    content_parts.push(event_data.content.clone());
                }
                if !event_data.conversation_id.is_empty() {
                    conversation_id = Some(event_data.conversation_id.clone());
                }
            } else if let Ok(simple) = serde_json::from_str::<serde_json::Value>(data_str) {
                if let Some(c) = simple.get("content").and_then(|v| v.as_str()) {
                    content_parts.push(c.to_string());
                }
                if let Some(cid) = simple.get("conversation_id").and_then(|v| v.as_str()) {
                    conversation_id = Some(cid.to_string());
                }
            }
        }
    }

    let result = if content_parts.is_empty() {
        None
    } else {
        Some(content_parts.concat())
    };

    // Note: conversation_id would need to be stored somewhere.
    // In a real implementation, pass it out via side-channel.
    // For simplicity, we just return the content text.
    let _ = conversation_id; // suppress unused warning

    Ok(result)
}

// ---------------------------------------------------------------------------
// Coze Agent Executor
// ---------------------------------------------------------------------------

/// Coze-backed AgentExecutor — wraps the Coze Bot API as an AstrBot agent.
pub struct CozeAgentExecutor {
    name: String,
    client: CozeClient,
    system_prompt: Option<String>,
}

impl CozeAgentExecutor {
    pub fn new(api_token: String, bot_id: String, name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            client: CozeClient::new(api_token, bot_id),
            system_prompt: None,
        }
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.client = self.client.with_base_url(url.into());
        self
    }
}

#[async_trait]
impl AgentExecutor for CozeAgentExecutor {
    fn name(&self) -> &str {
        &self.name
    }

    fn executor_type(&self) -> &str {
        "coze"
    }

    async fn initialize(&mut self, config: serde_json::Value) -> Result<()> {
        if let Some(base_url) = config.get("base_url").and_then(|v| v.as_str()) {
            self.client.set_base_url(base_url.to_string());
        }
        if let Some(prompt) = config.get("system_prompt").and_then(|v| v.as_str()) {
            self.system_prompt = Some(prompt.to_string());
        }
        info!("[CozeAgent] Initialized — bot_id={}", self.client.bot_id);
        Ok(())
    }

    async fn execute(&self, ctx: &AgentContext) -> Result<AgentResult> {
        let mut coze_messages = Vec::new();

        // System prompt
        if let Some(ref prompt) = self.system_prompt {
            coze_messages.push(CozeMessage {
                role: "system".to_string(),
                content: prompt.clone(),
                content_type: None,
            });
        }

        // Convert conversation history
        for msg in &ctx.messages {
            let role = match msg.role.as_str() {
                "system" => "system",
                "assistant" => "assistant",
                _ => "user",
            };
            coze_messages.push(CozeMessage {
                role: role.to_string(),
                content: msg.content.clone(),
                content_type: None,
            });
        }

        // Use non-streaming API for agent executor (returns complete response)
        let result = self.client.chat(&ctx.user_id,
            coze_messages,
            None,
        ).await?;

        Ok(AgentResult::Text {
            content: result.answer,
        })
    }

    async fn execute_with_tools(
        &self,
        ctx: &AgentContext,
        tool_results: Vec<ToolResult>,
    ) -> Result<AgentResult> {
        // Coze handles tool calling internally — we just pass the conversation
        // with tool results appended as user messages for now.
        let mut coze_messages = Vec::new();

        if let Some(ref prompt) = self.system_prompt {
            coze_messages.push(CozeMessage {
                role: "system".to_string(),
                content: prompt.clone(),
                content_type: None,
            });
        }

        for msg in &ctx.messages {
            let role = match msg.role.as_str() {
                "system" => "system",
                "assistant" => "assistant",
                _ => "user",
            };
            coze_messages.push(CozeMessage {
                role: role.to_string(),
                content: msg.content.clone(),
                content_type: None,
            });
        }

        // Append tool results as assistant notes
        for tr in tool_results {
            let text = format!("Tool {} result: {}", tr.call_id, tr.output);
            coze_messages.push(CozeMessage {
                role: "assistant".to_string(),
                content: text,
                content_type: None,
            });
        }

        let result = self.client.chat(&ctx.user_id,
            coze_messages,
            None,
        ).await?;

        Ok(AgentResult::Text {
            content: result.answer,
        })
    }

    async fn health_check(&self) -> Result<bool> {
        match self.client.session_store.get("health_check").await {
            Some(_) => Ok(true),
            None => {
                // Try a lightweight call
                let url = format!("{}/v3/bot/info", self.client.base_url);
                let resp = self.client.http_client
                    .get(&url)
                    .headers(self.client.auth_headers())
                    .query(&[("bot_id", self.client.bot_id.as_str())])
                    .send()
                    .await;
                match resp {
                    Ok(r) => Ok(r.status().is_success()),
                    Err(_) => Ok(false),
                }
            }
        }
    }
}

use super::ToolResult as AgentToolResult; // alias to avoid name clash

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_store() {
        let store = SessionStore::new();
        store.set("user_1", "conv_abc").await;
        assert_eq!(store.get("user_1").await, Some("conv_abc".to_string()));
        store.remove("user_1").await;
        assert_eq!(store.get("user_1").await, None);
    }

    #[test]
    fn test_coze_message_serialize() {
        let msg = CozeMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
            content_type: Some("text".to_string()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("user"));
        assert!(json.contains("hello"));
    }

    #[test]
    fn test_coze_chat_request_serialize() {
        let mut vars = HashMap::new();
        vars.insert("key".to_string(), "value".to_string());
        let req = CozeChatRequest {
            conversation_id: Some("conv_1".to_string()),
            bot_id: "bot_123".to_string(),
            user_id: "user_456".to_string(),
            additional_messages: Some(vec![CozeMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
                content_type: None,
            }]),
            custom_variables: Some(vars),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("bot_123"));
        assert!(json.contains("conv_1"));
    }

    #[test]
    fn test_parse_sse_buffer() {
        let buf = "data:{\"content\":\"hello\",\"conversation_id\":\"conv_1\"}\n\n";
        let result = parse_sse_buffer(buf).unwrap();
        assert_eq!(result, Some("hello".to_string()));
    }

    #[test]
    fn test_parse_sse_buffer_empty() {
        let buf = "event:message\n\n";
        let result = parse_sse_buffer(buf).unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_coze_agent_executor_creation() {
        let executor = CozeAgentExecutor::new(
            "test_token".to_string(),
            "test_bot".to_string(),
            "coze-test",
        );
        assert_eq!(executor.name(), "coze-test");
        assert_eq!(executor.executor_type(), "coze");
        let health = executor.health_check().await.unwrap();
        // health check with fake token will fail (network 401)
        assert!(!health);
    }
}
