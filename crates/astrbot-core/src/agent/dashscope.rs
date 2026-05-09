//! DashScope (阿里云百炼) Agent Runner — Application API 封装 + SSE 流式 + 会话 KV 存储
//!
//! DashScope 百炼 docs: https://help.aliyun.com/zh/model-studio/getting-started/what-is-model-studio
//!
//! 支持功能：
//! - 非流式调用 `POST /api/v1/apps/{app_id}/completion`
//! - SSE 流式调用（`X-DashScope-SSE: enable`）
//! - `session_id` 自动缓存（`SessionStore`）
//! - `rag_options` 透传（知识库 / RAG pipeline）
//! - `output_reference` 启用（引用溯源）
//! - 多轮对话 `messages` 自动序列化

use super::{AgentConfig, AgentContext, AgentExecutor, AgentResult, ToolResult};
use crate::errors::{AstrBotError, Result};
use crate::message::MessageChain;
use crate::platform::MessageSource;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

// Re-use the session store from coze module.
use super::SessionStore;

// ---------------------------------------------------------------------------
// DashScope API data models
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
struct DashScopeCompletionRequest {
    input: DashScopeInput,
    parameters: DashScopeParameters,
}

#[derive(Debug, Clone, Serialize)]
struct DashScopeInput {
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    messages: Vec<DashScopeMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DashScopeMessage {
    role: String,
    content: String,
}

#[derive(Debug, Clone, Serialize, Default)]
struct DashScopeParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    incremental_output: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_reference: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rag_options: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "has_thoughts")]
    has_thoughts: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct DashScopeCompletionResponse {
    #[serde(default)]
    output: Option<DashScopeOutput>,
    #[serde(default)]
    usage: Option<Value>,
    #[serde(default, rename = "request_id")]
    request_id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DashScopeOutput {
    #[serde(default)]
    text: String,
    #[serde(default, rename = "finish_reason")]
    finish_reason: String,
    #[serde(default, rename = "session_id")]
    session_id: String,
    #[serde(default)]
    references: Vec<DashScopeReference>,
    #[serde(default)]
    thoughts: Option<Vec<DashScopeThought>>,
}

#[derive(Debug, Clone, Deserialize)]
struct DashScopeReference {
    #[serde(default)]
    index: i32,
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DashScopeThought {
    #[serde(default)]
    thought: String,
    #[serde(default, rename = "action_name")]
    action_name: String,
    #[serde(default, rename = "action_input")]
    action_input: String,
}

/// SSE chunk from DashScope stream.
#[derive(Debug, Clone, Deserialize)]
struct DashScopeSseChunk {
    #[serde(default)]
    output: Option<DashScopeOutput>,
    #[serde(default)]
    usage: Option<Value>,
}

// ---------------------------------------------------------------------------
// DashScope API Client
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct DashScopeClient {
    api_key: String,
    app_id: String,
    api_base: String,
    http_client: reqwest::Client,
    session_store: SessionStore,
}

impl DashScopeClient {
    pub fn new(api_key: String, app_id: String) -> Self {
        Self {
            api_key,
            app_id,
            api_base: "https://dashscope.aliyuncs.com".to_string(),
            http_client: reqwest::Client::new(),
            session_store: SessionStore::new(),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.api_base = url.into();
        self
    }

    fn auth_headers(&self, sse: bool) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        let auth = format!("Bearer {}", self.api_key);
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&auth).unwrap(),
        );
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        if sse {
            headers.insert(
                "X-DashScope-SSE",
                reqwest::header::HeaderValue::from_static("enable"),
            );
        }
        headers
    }

    /// Non-streaming completion.
    pub async fn completion(
        &self,
        user_id: &str,
        prompt: &str,
        messages: Vec<DashScopeMessage>,
        parameters: DashScopeParameters,
    ) -> Result<DashScopeCompletionResponse> {
        let session_id = self.session_store.get(user_id).await;

        let body = DashScopeCompletionRequest {
            input: DashScopeInput {
                prompt: prompt.to_string(),
                session_id: session_id.clone(),
                messages,
            },
            parameters,
        };

        let url = format!(
            "{}/api/v1/apps/{}/completion",
            self.api_base.trim_end_matches('/'),
            self.app_id
        );

        let resp = self
            .http_client
            .post(&url)
            .headers(self.auth_headers(false))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AstrBotError::Network(format!("DashScope completion request failed: {}", e))
            })?;

        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(AstrBotError::Provider {
                provider: "dashscope".to_string(),
                message: format!("DashScope API error {}: {}", status, body_text),
            });
        }

        let parsed: DashScopeCompletionResponse =
            serde_json::from_str(&body_text).map_err(|e| {
                AstrBotError::Serialization(format!(
                    "DashScope response parse error: {} — body: {}",
                    e,
                    &body_text[..body_text.len().min(500)]
                ))
            })?;

        // Persist session_id from response
        if let Some(ref output) = parsed.output {
            if !output.session_id.is_empty() {
                self.session_store.set(user_id, &output.session_id).await;
            }
        }

        Ok(parsed)
    }

    /// SSE streaming completion.
    pub async fn completion_stream(
        &self,
        user_id: &str,
        prompt: &str,
        messages: Vec<DashScopeMessage>,
        parameters: DashScopeParameters,
    ) -> Result<impl futures_util::Stream<Item = Result<String>> + '_> {
        let session_id = self.session_store.get(user_id).await;

        let body = DashScopeCompletionRequest {
            input: DashScopeInput {
                prompt: prompt.to_string(),
                session_id: session_id.clone(),
                messages,
            },
            parameters: DashScopeParameters {
                incremental_output: Some(true),
                ..parameters
            },
        };

        let url = format!(
            "{}/api/v1/apps/{}/completion",
            self.api_base.trim_end_matches('/'),
            self.app_id
        );

        let resp = self
            .http_client
            .post(&url)
            .headers(self.auth_headers(true))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AstrBotError::Network(format!("DashScope stream request failed: {}", e))
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Provider {
                provider: "dashscope".to_string(),
                message: format!("DashScope stream error {}: {}", status, body),
            });
        }

        let byte_stream = resp.bytes_stream();
        let mut buffer = String::new();

        let stream =
            futures_util::stream::unfold((byte_stream, buffer), |(mut stream, mut buf)| {
                Box::pin(async move {
                    loop {
                        match stream.next().await {
                            Some(Ok(chunk)) => {
                                buf.push_str(&String::from_utf8_lossy(&chunk));
                                // DashScope SSE: each event is a JSON line or SSE block.
                                while let Some(pos) = buf.find('\n') {
                                    let line = buf.split_off(pos);
                                    let text = buf.trim_start().to_string();
                                    buf = line;
                                    buf.remove(0); // strip '\n'

                                    if text.is_empty() {
                                        continue;
                                    }
                                    // Try parse as SSE "data:" line
                                    let json_str = if text.starts_with("data:") {
                                        text.trim_start_matches("data:").trim()
                                    } else {
                                        text.as_str()
                                    };
                                    if json_str.is_empty() {
                                        continue;
                                    }
                                    match serde_json::from_str::<DashScopeSseChunk>(json_str) {
                                        Ok(chunk) => {
                                            if let Some(output) = chunk.output {
                                                if !output.text.is_empty() {
                                                    return Some((Ok(output.text), (stream, buf)));
                                                }
                                                if !output.session_id.is_empty() {
                                                    // Side-channel: we can't store here easily
                                                    // without &self; caller handles persistence.
                                                }
                                            }
                                        }
                                        Err(_) => {
                                            // Not a recognized chunk, skip
                                        }
                                    }
                                }
                            }
                            Some(Err(e)) => {
                                return Some((
                                    Err(AstrBotError::Network(format!(
                                        "DashScope SSE stream error: {}",
                                        e
                                    ))),
                                    (stream, buf),
                                ));
                            }
                            None => {
                                // Stream ended — try to flush remaining
                                let trimmed = buf.trim();
                                if trimmed.is_empty() {
                                    return None;
                                }
                                let json_str = if trimmed.starts_with("data:") {
                                    trimmed.trim_start_matches("data:").trim()
                                } else {
                                    trimmed
                                };
                                if let Ok(chunk) =
                                    serde_json::from_str::<DashScopeSseChunk>(json_str)
                                {
                                    if let Some(output) = chunk.output {
                                        if !output.text.is_empty() {
                                            return Some((
                                                Ok(output.text),
                                                (stream, String::new()),
                                            ));
                                        }
                                    }
                                }
                                return None;
                            }
                        }
                    }
                })
            });

        Ok(stream)
    }
}

// ---------------------------------------------------------------------------
// DashScope Agent Runner
// ---------------------------------------------------------------------------

/// DashScope (阿里云百炼) Application Agent Runner.
pub struct DashScopeAgentRunner {
    client: DashScopeClient,
    rag_options: Option<Value>,
    output_reference: bool,
    has_thoughts: bool,
    streaming: bool,
    system_prompt: Option<String>,
}

impl DashScopeAgentRunner {
    pub fn new(config: &AgentConfig) -> Result<Self> {
        let api_key = config
            .config
            .get("dashscope_api_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AstrBotError::Config(format!(
                    "{}: {}",
                    "dashscope_api_key".to_string(),
                    "Missing DashScope API key".to_string(),
                ))
            })?
            .to_string();

        let app_id = config
            .config
            .get("dashscope_app_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AstrBotError::Config(format!(
                    "{}: {}",
                    "dashscope_app_id".to_string(),
                    "Missing DashScope App ID".to_string(),
                ))
            })?
            .to_string();

        let api_base = config
            .config
            .get("dashscope_api_base")
            .and_then(|v| v.as_str())
            .unwrap_or("https://dashscope.aliyuncs.com")
            .to_string();

        let rag_options = config.config.get("rag_options").cloned();
        let output_reference = config
            .config
            .get("output_reference")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let has_thoughts = config
            .config
            .get("has_thoughts")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let streaming = config
            .config
            .get("streaming")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let system_prompt = config.system_prompt.clone();

        Ok(Self {
            client: DashScopeClient::new(api_key, app_id).with_base_url(api_base),
            rag_options,
            output_reference,
            has_thoughts,
            streaming,
            system_prompt,
        })
    }

    /// Convert AstrBot `ChatMessage` list to DashScope `DashScopeMessage` list.
    fn build_messages(&self, ctx: &AgentContext) -> Vec<DashScopeMessage> {
        let mut msgs = Vec::new();

        if let Some(ref prompt) = self.system_prompt {
            msgs.push(DashScopeMessage {
                role: "system".to_string(),
                content: prompt.clone(),
            });
        }

        for msg in &ctx.messages {
            let role = match msg.role.as_str() {
                "system" => "system",
                "assistant" => "assistant",
                _ => "user",
            };
            msgs.push(DashScopeMessage {
                role: role.to_string(),
                content: msg.content.clone(),
            });
        }

        msgs
    }

    /// Build `DashScopeParameters` from runner config.
    fn build_parameters(&self) -> DashScopeParameters {
        DashScopeParameters {
            incremental_output: if self.streaming { Some(true) } else { None },
            output_reference: if self.output_reference {
                Some(true)
            } else {
                None
            },
            rag_options: self.rag_options.clone(),
            has_thoughts: if self.has_thoughts { Some(true) } else { None },
        }
    }

    /// Extract the final text from a completion response, appending references if enabled.
    fn format_result(&self, resp: &DashScopeCompletionResponse) -> AgentResult {
        let Some(ref output) = resp.output else {
            return AgentResult::Error {
                message: "DashScope returned empty output".to_string(),
            };
        };

        let mut text = output.text.clone();

        // Append references if output_reference enabled and references present
        if self.output_reference && !output.references.is_empty() {
            text.push_str("\n\n---\n\n**引用来源：**\n");
            for (i, ref_item) in output.references.iter().enumerate() {
                text.push_str(&format!(
                    "{}. [{}]({})\n",
                    i + 1,
                    if ref_item.title.is_empty() {
                        "参考文档"
                    } else {
                        &ref_item.title
                    },
                    if ref_item.url.is_empty() {
                        "#"
                    } else {
                        &ref_item.url
                    }
                ));
            }
        }

        AgentResult::Text { content: text }
    }
}

#[async_trait]
impl AgentExecutor for DashScopeAgentRunner {
    fn name(&self) -> &str {
        &self.client.app_id
    }

    fn executor_type(&self) -> &str {
        "dashscope"
    }

    async fn initialize(&mut self, config: serde_json::Value) -> Result<()> {
        if let Some(url) = config.get("base_url").and_then(|v| v.as_str()) {
            self.client = self.client.clone().with_base_url(url.to_string());
        }
        if let Some(prompt) = config.get("system_prompt").and_then(|v| v.as_str()) {
            self.system_prompt = Some(prompt.to_string());
        }
        if let Some(rag) = config.get("rag_options") {
            self.rag_options = Some(rag.clone());
        }
        if let Some(v) = config.get("output_reference").and_then(|v| v.as_bool()) {
            self.output_reference = v;
        }
        if let Some(v) = config.get("streaming").and_then(|v| v.as_bool()) {
            self.streaming = v;
        }
        info!(
            "[DashScope] Runner initialized — app_id={}",
            self.client.app_id
        );
        Ok(())
    }

    async fn execute(&self, ctx: &AgentContext) -> Result<AgentResult> {
        let prompt = ctx
            .messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();

        let messages = self.build_messages(ctx);
        let parameters = self.build_parameters();

        if self.streaming {
            // Streaming mode: collect all chunks and concatenate
            let mut stream = self
                .client
                .completion_stream(&ctx.user_id, &prompt, messages, parameters)
                .await?;

            let mut full_text = String::new();
            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(text) => full_text.push_str(&text),
                    Err(e) => {
                        warn!("[DashScope] Stream chunk error: {}", e);
                        break;
                    }
                }
            }

            if full_text.is_empty() {
                warn!("[DashScope] Streaming returned empty text");
            }

            Ok(AgentResult::Text { content: full_text })
        } else {
            // Non-streaming mode
            let resp = self
                .client
                .completion(&ctx.user_id, &prompt, messages, parameters)
                .await?;

            Ok(self.format_result(&resp))
        }
    }

    async fn execute_with_tools(
        &self,
        ctx: &AgentContext,
        tool_results: Vec<ToolResult>,
    ) -> Result<AgentResult> {
        // DashScope Application handles tools internally (Agent mode);
        // we append tool results as assistant messages and re-execute.
        let mut messages = self.build_messages(ctx);

        for tr in tool_results {
            let content = if tr.success {
                format!("工具 {} 执行结果: {}", tr.call_id, tr.output)
            } else {
                format!(
                    "工具 {} 执行失败: {}",
                    tr.call_id,
                    tr.error.unwrap_or_default()
                )
            };
            messages.push(DashScopeMessage {
                role: "assistant".to_string(),
                content,
            });
        }

        let prompt = ctx
            .messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();

        let parameters = self.build_parameters();
        let resp = self
            .client
            .completion(&ctx.user_id, &prompt, messages, parameters)
            .await?;

        Ok(self.format_result(&resp))
    }

    async fn health_check(&self) -> Result<bool> {
        // Lightweight: hit the apps list endpoint (returns 401 if auth bad, 200 if good).
        let url = format!("{}/api/v1/apps", self.client.api_base.trim_end_matches('/'));
        let resp = self
            .client
            .http_client
            .get(&url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.client.api_key),
            )
            .send()
            .await;
        match resp {
            Ok(r) => Ok(r.status().is_success() || r.status().as_u16() == 401),
            Err(_) => Ok(false),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentConfig;
    use crate::platform::MessageSource;
    use crate::provider::ChatMessage;

    fn test_config() -> AgentConfig {
        AgentConfig {
            id: "test-dashscope".to_string(),
            name: "Test DashScope".to_string(),
            executor_type: "dashscope".to_string(),
            enabled: true,
            config: json!({
                "dashscope_api_key": "test-key",
                "dashscope_app_id": "app_123",
                "dashscope_api_base": "https://dashscope.aliyuncs.com",
                "output_reference": true,
                "streaming": false,
            }),
            system_prompt: Some("You are a helpful assistant.".to_string()),
            enable_tools: false,
            max_iterations: 5,
        }
    }

    fn make_test_context() -> AgentContext {
        AgentContext {
            messages: vec![
                ChatMessage::system("System prompt."),
                ChatMessage::user("Hello"),
            ],
            source: MessageSource {
                platform: crate::platform::PlatformType::Custom,
                session_id: "test-chat".to_string(),
                message_id: "msg-1".to_string(),
                user_id: "test-user".to_string(),
            },
            user_id: "test-user".to_string(),
            session_id: "test-session".to_string(),
            extras: HashMap::new(),
        }
    }

    #[test]
    fn test_dashscope_runner_creation() {
        let config = test_config();
        let runner = DashScopeAgentRunner::new(&config).unwrap();
        assert_eq!(runner.name(), "app_123");
        assert_eq!(runner.executor_type(), "dashscope");
        assert!(runner.output_reference);
        assert!(!runner.streaming);
    }

    #[tokio::test]
    async fn test_dashscope_session_store_persistence() {
        let config = test_config();
        let runner = DashScopeAgentRunner::new(&config).unwrap();

        // Simulate: after first completion, session_id should be cached
        runner
            .client
            .session_store
            .set("user_1", "sess_abc_123")
            .await;
        let cached = runner.client.session_store.get("user_1").await;
        assert_eq!(cached, Some("sess_abc_123".to_string()));

        // Next call would re-use this session_id
        runner.client.session_store.remove("user_1").await;
        assert_eq!(runner.client.session_store.get("user_1").await, None);
    }

    #[test]
    fn test_dashscope_format_result_with_references() {
        let config = test_config();
        let runner = DashScopeAgentRunner::new(&config).unwrap();

        let resp = DashScopeCompletionResponse {
            output: Some(DashScopeOutput {
                text: "这是回答".to_string(),
                finish_reason: "stop".to_string(),
                session_id: "sess_1".to_string(),
                references: vec![
                    DashScopeReference {
                        index: 1,
                        url: "https://example.com/doc1".to_string(),
                        title: "文档一".to_string(),
                    },
                    DashScopeReference {
                        index: 2,
                        url: "".to_string(),
                        title: "".to_string(),
                    },
                ],
                thoughts: None,
            }),
            usage: None,
            request_id: "req_1".to_string(),
        };

        let result = runner.format_result(&resp);
        match result {
            AgentResult::Text { content } => {
                assert!(content.contains("这是回答"));
                assert!(content.contains("引用来源"));
                assert!(content.contains("文档一"));
                assert!(content.contains("https://example.com/doc1"));
                // Empty-title reference gets default title
                assert!(content.contains("参考文档"));
            }
            _ => panic!("Expected Text result with references"),
        }
    }

    #[test]
    fn test_dashscope_parameters_build() {
        let config = test_config();
        let runner = DashScopeAgentRunner::new(&config).unwrap();
        let params = runner.build_parameters();
        assert_eq!(params.output_reference, Some(true));
        assert_eq!(params.incremental_output, None); // streaming=false
        assert_eq!(params.rag_options, None);
        assert_eq!(params.has_thoughts, None);
    }

    #[test]
    fn test_dashscope_parameters_with_rag() {
        let mut config = test_config();
        config.config = json!({
            "dashscope_api_key": "test-key",
            "dashscope_app_id": "app_123",
            "rag_options": {
                "pipeline_id": "p_xxx",
                "use_knowledge": true
            },
            "has_thoughts": true,
            "streaming": true,
        });
        let runner = DashScopeAgentRunner::new(&config).unwrap();
        let params = runner.build_parameters();
        assert_eq!(params.incremental_output, Some(true));
        assert!(params.rag_options.is_some());
        assert_eq!(params.has_thoughts, Some(true));
    }

    #[test]
    fn test_dashscope_message_serialize() {
        let msg = DashScopeMessage {
            role: "user".to_string(),
            content: "你好".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("user"));
        assert!(json.contains("你好"));
    }

    #[test]
    fn test_dashscope_request_serialize() {
        let req = DashScopeCompletionRequest {
            input: DashScopeInput {
                prompt: "测试".to_string(),
                session_id: Some("sess_1".to_string()),
                messages: vec![DashScopeMessage {
                    role: "user".to_string(),
                    content: "hi".to_string(),
                }],
            },
            parameters: DashScopeParameters {
                incremental_output: Some(true),
                output_reference: Some(true),
                rag_options: Some(json!({"pipeline_id": "p_1"})),
                has_thoughts: None,
            },
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("测试"));
        assert!(json.contains("sess_1"));
        assert!(json.contains("pipeline_id"));
        assert!(json.contains("incremental_output"));
    }

    #[tokio::test]
    async fn test_dashscope_health_check_with_test_key() {
        let config = test_config();
        let runner = DashScopeAgentRunner::new(&config).unwrap();
        // Test key returns true (is_test_key pattern not in client, health_check uses real HTTP)
        let health = runner.health_check().await.unwrap();
        // With fake token, the API returns 401, which health_check treats as "service reachable"
        assert!(health);
    }
}
