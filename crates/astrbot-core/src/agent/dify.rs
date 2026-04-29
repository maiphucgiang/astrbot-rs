//! Dify Agent Runner — Chat / Agent / Chatflow / Workflow 四种模式
//!
//! Dify API docs: https://docs.dify.ai/guides/application-orchestrate
//!
//! 支持模式：
//! - `chat`     — 对话型应用
//! - `agent`    — Agent 型应用
//! - `chatflow` — Chatflow 工作流
//! - `workflow` — 普通工作流
//!
//! 会话 KV：conversation_id 缓存在内存 SessionStore 中。

use async_trait::async_trait;
use crate::errors::{AstrBotError, Result};
use crate::message::{MessageChain, MessageComponent};
use super::{AgentContext, AgentResult, AgentExecutor, AgentConfig};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

// Re-use the session store from coze module.
use super::SessionStore;

// ---------------------------------------------------------------------------
// Dify API data models
// ---------------------------------------------------------------------------

/// Dify chat-messages request payload.
#[derive(Debug, Clone, Serialize)]
struct DifyChatRequest {
    inputs: HashMap<String, Value>,
    query: String,
    user: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    files: Vec<DifyFile>,
    response_mode: String, // "streaming" | "blocking"
}

/// Dify workflows/run request payload.
#[derive(Debug, Clone, Serialize)]
struct DifyWorkflowRequest {
    inputs: HashMap<String, Value>,
    user: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    files: Vec<DifyFile>,
    response_mode: String,
}

/// File reference for Dify chat/workflow requests.
#[derive(Debug, Clone, Serialize)]
struct DifyFile {
    #[serde(rename = "type")]
    file_type: String, // "image" | "document" | "audio" | "video" | "custom"
    #[serde(rename = "transfer_method")]
    transfer_method: String, // "local_file" | "remote_url"
    #[serde(skip_serializing_if = "Option::is_none")]
    upload_file_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

/// SSE event from Dify stream.
#[derive(Debug, Clone, Deserialize)]
struct DifySseEvent {
    #[serde(default)]
    event: String,
    #[serde(default)]
    #[serde(rename = "message_id", skip_serializing_if = "Option::is_none")]
    message_id: Option<String>,
    #[serde(default)]
    #[serde(rename = "conversation_id", skip_serializing_if = "Option::is_none")]
    conversation_id: Option<String>,
    #[serde(default)]
    answer: String,
    #[serde(default)]
    #[serde(rename = "status", skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(default)]
    #[serde(rename = "task_id", skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    #[serde(default)]
    #[serde(rename = "id", skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(default)]
    #[serde(rename = "workflow_run_id", skip_serializing_if = "Option::is_none")]
    workflow_run_id: Option<String>,
    #[serde(default)]
    #[serde(rename = "data", skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(default)]
    #[serde(rename = "outputs", skip_serializing_if = "Option::is_none")]
    outputs: Option<Value>,
    #[serde(default)]
    #[serde(rename = "error", skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default)]
    #[serde(rename = "files", skip_serializing_if = "Option::is_none")]
    files: Option<Vec<Value>>,
}

// ---------------------------------------------------------------------------
// Dify API Client
// ---------------------------------------------------------------------------

/// Low-level HTTP client for Dify Open API.
#[derive(Clone)]
pub struct DifyApiClient {
    api_key: String,
    api_base: String,
    http_client: reqwest::Client,
}

impl DifyApiClient {
    pub fn new(api_key: String, api_base: String) -> Self {
        Self {
            api_key,
            api_base,
            http_client: reqwest::Client::new(),
        }
    }

    fn auth_headers(&self) -> reqwest::header::HeaderMap {
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
        headers
    }

    /// SSE stream parser: splits response body into `data: {...}` events.
    fn parse_sse_stream(
        &self,
        resp: reqwest::Response,
    ) -> impl futures_util::Stream<Item = Result<DifySseEvent>> {
        let bytes_stream = Box::pin(resp.bytes_stream());
        let mut buffer = String::new();

        futures_util::stream::unfold((bytes_stream, buffer), |(mut stream, mut buf)| async move {
            loop {
                // If we already have a complete SSE block, yield it.
                if let Some(pos) = buf.find("\n\n") {
                    let block = buf.split_off(pos);
                    let event_text = buf.trim_start().to_string();
                    buf = block;
                    buf.replace_range(..2, ""); // strip "\n\n"

                    if event_text.starts_with("data:") {
                        let json_str = &event_text[5..];
                        let event: Result<DifySseEvent> =
                            serde_json::from_str(json_str.trim())
                                .map_err(|e| AstrBotError::Serialization(format!("Dify SSE JSON parse error: {}", e)));
                        return Some((event, (stream, buf)));
                    }
                    // Ignore "event: ..." or empty lines, keep looping.
                    continue;
                }

                // Need more data from the stream.
                match stream.next().await {
                    Some(Ok(chunk)) => {
                        buf.push_str(&String::from_utf8_lossy(&chunk));
                    }
                    Some(Err(e)) => {
                        let err = Err(AstrBotError::Network(format!("Dify SSE stream error: {}", e)));
                        return Some((err, (stream, buf)));
                    }
                    None => {
                        // Stream ended. Try to flush any remaining `data:...` block.
                        let trimmed = buf.trim_start();
                        if trimmed.starts_with("data:") && !trimmed.trim().is_empty() {
                            let json_str = &trimmed[5..];
                            let event: Result<DifySseEvent> =
                                serde_json::from_str(json_str.trim())
                                    .map_err(|e| AstrBotError::Serialization(format!("Dify SSE JSON parse error: {}", e)));
                            return Some((event, (stream, String::new())));
                        }
                        return None;
                    }
                }
            }
        })
    }

    /// Chat / Agent / Chatflow — streaming POST /chat-messages
    pub async fn chat_messages_stream(
        &self,
        req: &DifyChatRequest,
        timeout_secs: u64,
    ) -> Result<impl futures_util::Stream<Item = Result<DifySseEvent>> + '_> {
        let url = format!("{}/chat-messages", self.api_base);
        let resp = self
            .http_client
            .post(&url)
            .headers(self.auth_headers())
            .json(req)
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Dify chat-messages request failed: {}", e)))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Provider {
                provider: "dify".to_string(),
                message: format!("Dify chat-messages error: {} - {}", status, body),
            });
        }

        Ok(self.parse_sse_stream(resp))
    }

    /// Workflow — streaming POST /workflows/run
    pub async fn workflow_run_stream(
        &self,
        req: &DifyWorkflowRequest,
        timeout_secs: u64,
    ) -> Result<impl futures_util::Stream<Item = Result<DifySseEvent>> + '_> {
        let url = format!("{}/workflows/run", self.api_base);
        let resp = self
            .http_client
            .post(&url)
            .headers(self.auth_headers())
            .json(req)
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Dify workflow-run request failed: {}", e)))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Provider {
                provider: "dify".to_string(),
                message: format!("Dify workflow-run error: {} - {}", status, body),
            });
        }

        Ok(self.parse_sse_stream(resp))
    }

    /// Upload file — POST /files/upload
    /// Returns `{ "id": "...", "name": "...", "size": 123, ... }`
    pub async fn upload_file(
        &self,
        file_data: Vec<u8>,
        file_name: &str,
        mime_type: &str,
        user: &str,
    ) -> Result<Value> {
        let url = format!("{}/files/upload", self.api_base);

        let part = reqwest::multipart::Part::bytes(file_data)
            .file_name(file_name.to_string())
            .mime_str(mime_type)
            .map_err(|e| AstrBotError::Serialization(format!("MIME type error: {}", e)))?;

        let form = reqwest::multipart::Form::new()
            .text("user", user.to_string())
            .part("file", part);

        let mut headers = reqwest::header::HeaderMap::new();
        let auth = format!("Bearer {}", self.api_key);
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&auth).unwrap(),
        );
        // Content-Type is set automatically by multipart.

        let resp = self
            .http_client
            .post(&url)
            .headers(headers)
            .multipart(form)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Dify file upload request failed: {}", e)))?;

        let status = resp.status();
        let body = resp.json::<Value>().await.map_err(|e| {
            AstrBotError::Serialization(format!("Failed to parse Dify upload response: {}", e))
        })?;

        if !status.is_success() {
            return Err(AstrBotError::Provider {
                provider: "dify".to_string(),
                message: format!("Dify file upload error: {} - {:?}", status, body),
            });
        }

        Ok(body)
    }
}

// ---------------------------------------------------------------------------
// Dify Agent Runner
// ---------------------------------------------------------------------------

/// Dify API type — determines which endpoint and parsing strategy to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DifyApiType {
    Chat,
    Agent,
    Chatflow,
    Workflow,
}

impl std::str::FromStr for DifyApiType {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "chat" => Ok(DifyApiType::Chat),
            "agent" => Ok(DifyApiType::Agent),
            "chatflow" => Ok(DifyApiType::Chatflow),
            "workflow" => Ok(DifyApiType::Workflow),
            _ => Err(format!("Unknown Dify api_type: {}", s)),
        }
    }
}

/// Dify Agent Runner — implements [`AgentExecutor`].
pub struct DifyAgentRunner {
    api_client: DifyApiClient,
    api_type: DifyApiType,
    workflow_output_key: String,
    query_input_key: String,
    variables: HashMap<String, Value>,
    timeout_secs: u64,
    session_store: SessionStore,
    streaming: bool,
}

impl DifyAgentRunner {
    pub fn new(config: &AgentConfig) -> Result<Self> {
        let api_key = config
            .config
            .get("dify_api_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AstrBotError::Config("Missing Dify API key (dify_api_key)".to_string()))?
            .to_string();

        let api_base = config
            .config
            .get("dify_api_base")
            .and_then(|v| v.as_str())
            .unwrap_or("https://api.dify.ai/v1")
            .to_string();

        let api_type_str = config
            .config
            .get("dify_api_type")
            .and_then(|v| v.as_str())
            .unwrap_or("chat");
        let api_type = DifyApiType::from_str(api_type_str)
            .map_err(|e| AstrBotError::Config(format!("Invalid dify_api_type: {}", e)))?;

        let workflow_output_key = config
            .config
            .get("dify_workflow_output_key")
            .and_then(|v| v.as_str())
            .unwrap_or("astrbot_wf_output")
            .to_string();

        let query_input_key = config
            .config
            .get("dify_query_input_key")
            .and_then(|v| v.as_str())
            .unwrap_or("astrbot_text_query")
            .to_string();

        let variables: HashMap<String, Value> = config
            .config
            .get("variables")
            .and_then(|v| v.as_object())
            .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();

        let timeout_secs = config
            .config
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(60);

        let streaming = config
            .config
            .get("streaming")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        Ok(Self {
            api_client: DifyApiClient::new(api_key, api_base),
            api_type,
            workflow_output_key,
            query_input_key,
            variables,
            timeout_secs,
            session_store: SessionStore::new(),
            streaming,
        })
    }

    /// Build file list from base64 image URLs (upload them to Dify first).
    async fn build_files(
        &self,
        image_urls: &[String],
        user: &str,
    ) -> Result<Vec<DifyFile>> {
        let mut files = Vec::new();
        for url in image_urls {
            // Assume base64 data URI: data:image/png;base64,xxx
            let (mime, data) = parse_data_uri(url)?;
            let upload_resp = self
                .api_client
                .upload_file(data.clone(), "image.png", &mime, user)
                .await?;
            let file_id = upload_resp
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AstrBotError::Provider {
                    provider: "dify".to_string(),
                    message: "Dify upload response missing file id".to_string(),
                })?
                .to_string();

            files.push(DifyFile {
                file_type: "image".to_string(),
                transfer_method: "local_file".to_string(),
                upload_file_id: Some(file_id),
                url: None,
            });
        }
        Ok(files)
    }

    /// Merge static variables + session variables + system_prompt.
    async fn build_payload_vars(&self, session_id: &str, system_prompt: Option<&str>) -> HashMap<String, Value> {
        let mut vars = self.variables.clone();
        if let Some(sp) = system_prompt {
            vars.insert("system_prompt".to_string(), json!(sp));
        }
        // Session variables (if any KV store integration exists)
        // For now, just static + system_prompt.
        let _ = session_id;
        vars
    }

    /// Run Chat / Agent / Chatflow mode.
    async fn run_chat_mode(
        &self,
        ctx: &AgentContext,
        conversation_id: Option<String>,
        files: Vec<DifyFile>,
    ) -> Result<AgentResult> {
        let query = ctx
            .extras
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if query.is_empty() && files.is_empty() {
            return Ok(AgentResult::Text {
                content: "请描述这张图片。".to_string(),
            });
        }

        let vars = self
            .build_payload_vars(&ctx.session_id, None)
            .await;

        let req = DifyChatRequest {
            inputs: vars,
            query,
            user: ctx.user_id.clone(),
            conversation_id,
            files,
            response_mode: if self.streaming {
                "streaming".to_string()
            } else {
                "blocking".to_string()
            },
        };

        let mut stream = self
            .api_client
            .chat_messages_stream(&req, self.timeout_secs)
            .await?;

        let mut result_text = String::new();
        let mut new_conversation_id: Option<String> = None;

        futures_util::pin_mut!(stream);
        while let Some(event_result) = stream.next().await {
            let event = event_result?;
            match event.event.as_str() {
                "message" | "agent_message" => {
                    result_text.push_str(&event.answer);
                }
                "message_end" => {
                    info!("[Dify] Message end");
                    break;
                }
                "error" => {
                    let msg = event.error.unwrap_or_else(|| "Unknown Dify error".to_string());
                    return Err(AstrBotError::Provider {
                        provider: "dify".to_string(),
                        message: format!("Dify error: {}", msg),
                    });
                }
                _ => {
                    // Capture conversation_id from first event if present.
                    if let Some(cid) = event.conversation_id {
                        if new_conversation_id.is_none() && !cid.is_empty() {
                            new_conversation_id = Some(cid);
                        }
                    }
                }
            }
        }

        // Persist conversation_id
        if let Some(cid) = new_conversation_id {
            self.session_store
                .set(&ctx.user_id, &cid)
                .await;
        }

        if result_text.is_empty() {
            warn!("[Dify] Empty response from chat mode");
        }

        Ok(AgentResult::Text {
            content: result_text,
        })
    }

    /// Run Workflow mode.
    async fn run_workflow_mode(
        &self,
        ctx: &AgentContext,
        files: Vec<DifyFile>,
    ) -> Result<AgentResult> {
        let prompt = ctx
            .extras
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut inputs = self
            .build_payload_vars(&ctx.session_id, None)
            .await;
        inputs.insert(self.query_input_key.clone(), json!(prompt));
        inputs.insert("astrbot_session_id".to_string(), json!(ctx.session_id.clone()));

        let req = DifyWorkflowRequest {
            inputs,
            user: ctx.user_id.clone(),
            files,
            response_mode: if self.streaming {
                "streaming".to_string()
            } else {
                "blocking".to_string()
            },
        };

        let mut stream = self
            .api_client
            .workflow_run_stream(&req, self.timeout_secs)
            .await?;

        let mut workflow_result: Option<Value> = None;
        let mut text_chunks = Vec::new();

        futures_util::pin_mut!(stream);
        while let Some(event_result) = stream.next().await {
            let event = event_result?;
            match event.event.as_str() {
                "workflow_started" => {
                    if let Some(ref data) = event.data {
                        if let Some(wid) = data.get("workflow_run_id").and_then(|v| v.as_str()) {
                            info!("[Dify] Workflow started: {}", wid);
                        }
                    }
                }
                "node_finished" => {
                    if let Some(ref data) = event.data {
                        if let Some(title) = data.get("title").and_then(|v| v.as_str()) {
                            info!("[Dify] Node finished: {}", title);
                        }
                    }
                }
                "text_chunk" => {
                    if let Some(ref data) = event.data {
                        if let Some(text) = data.get("text").and_then(|v| v.as_str()) {
                            text_chunks.push(text.to_string());
                        }
                    }
                }
                "workflow_finished" => {
                    if let Some(ref data) = event.data {
                        if let Some(err) = data.get("error").and_then(|v| v.as_str()) {
                            if !err.is_empty() {
                                return Err(AstrBotError::Provider {
                                    provider: "dify".to_string(),
                                    message: format!("Dify workflow error: {}", err),
                                });
                            }
                        }
                        workflow_result = Some(data.clone());
                    }
                    break;
                }
                "error" => {
                    let msg = event.error.unwrap_or_else(|| "Unknown Dify error".to_string());
                    return Err(AstrBotError::Provider {
                        provider: "dify".to_string(),
                        message: format!("Dify workflow error: {}", msg),
                    });
                }
                _ => {}
            }
        }

        // Parse final output
        let chain = if let Some(ref result) = workflow_result {
            self.parse_workflow_output(result).await?
        } else if !text_chunks.is_empty() {
            MessageChain::new().text(text_chunks.join(""))
        } else {
            MessageChain::new().text("Dify workflow returned empty result.")
        };

        // For workflow, we return the chain content as text.
        let content = chain
            .plain_text();

        Ok(AgentResult::Text { content })
    }

    /// Parse workflow output — handle string / list / file attachments.
    async fn parse_workflow_output(&self, data: &Value) -> Result<MessageChain> {
        let mut chain = MessageChain::new();

        // Extract the configured output key
        let outputs = data.get("outputs").cloned().unwrap_or(json!({}));
        let output = outputs
            .get(&self.workflow_output_key)
            .cloned()
            .unwrap_or_else(|| outputs.clone());

        match output {
            Value::String(s) => {
                chain.0.push(MessageComponent::Plain { text: s });
            }
            Value::Array(arr) => {
                // Check if this is an Array[File] from Dify HTTP request node
                for item in arr {
                    if let Some(obj) = item.as_object() {
                        if obj.get("dify_model_identity")
                            .and_then(|v| v.as_str())
                            == Some("__dify__file__")
                        {
                            // File array — parse each file
                            let url = obj.get("url").and_then(|v| v.as_str()).unwrap_or("");
                            if let Some(file_type) = obj.get("type").and_then(|v| v.as_str()) {
                                match file_type {
                                    "image" => {
                                        chain.0.push(MessageComponent::Image {
                                            url: Some(url.to_string()),
                                            file_id: None,
                                            base64: None,
                                        });
                                    }
                                    "audio" => {
                                        chain.0.push(MessageComponent::Voice {
                                            url: Some(url.to_string()),
                                            file_id: None,
                                            base64: None,
                                        });
                                    }
                                    "video" => {
                                        chain.0.push(MessageComponent::Image {
                                            url: Some(url.to_string()),
                                            file_id: None,
                                            base64: None,
                                        });
                                    }
                                    _ => {
                                        let name = obj.get("filename").and_then(|v| v.as_str()).unwrap_or("file");
                                        chain.0.push(MessageComponent::File {
                                            name: name.to_string(),
                                            url: Some(url.to_string()),
                                            file_id: None,
                                        });
                                    }
                                }
                            }
                        } else {
                            // Plain array element — stringify
                            chain.0.push(MessageComponent::Plain {
                                text: item.to_string(),
                            });
                        }
                    }
                }
            }
            other => {
                chain.0.push(MessageComponent::Plain {
                    text: other.to_string(),
                });
            }
        }

        // Scan top-level files array
        if let Some(files) = data.get("files").and_then(|v| v.as_array()) {
            for file in files {
                if let Some(obj) = file.as_object() {
                    let file_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    let url = obj.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    match file_type {
                        "image" => chain.0.push(MessageComponent::Image {
                            url: Some(url.to_string()),
                            file_id: None,
                            base64: None,
                        }),
                        _ => {}
                    }
                }
            }
        }

        Ok(chain)
    }
}

/// Parse a data URI like `data:image/png;base64,xxx` → (mime_type, decoded_bytes).
fn parse_data_uri(uri: &str) -> Result<(String, Vec<u8>)> {
    let prefix = "data:";
    if !uri.starts_with(prefix) {
        return Err(AstrBotError::Serialization(format!(
            "Invalid data URI (missing data: prefix): {}",
            &uri[..uri.len().min(40)]
        )));
    }

    let rest = &uri[prefix.len()..];
    let comma_idx = rest
        .find(',')
        .ok_or_else(|| AstrBotError::Serialization("Invalid data URI (no comma)".to_string()))?;

    let meta = &rest[..comma_idx];
    let data = &rest[comma_idx + 1..];

    let mime = meta.split(';').next().unwrap_or("application/octet-stream");

    if meta.contains("base64") {
        let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data)
            .map_err(|e| AstrBotError::Serialization(format!("Base64 decode error: {}", e)))?;
        Ok((mime.to_string(), decoded))
    } else {
        // Non-base64 data URIs are rare; return error for now.
        Err(AstrBotError::Serialization(
            "Non-base64 data URIs are not supported".to_string(),
        ))
    }
}

#[async_trait]
impl AgentExecutor for DifyAgentRunner {
    fn name(&self) -> &str {
        "dify"
    }

    fn executor_type(&self) -> &str {
        match self.api_type {
            DifyApiType::Chat => "chat",
            DifyApiType::Agent => "agent",
            DifyApiType::Chatflow => "chatflow",
            DifyApiType::Workflow => "workflow",
        }
    }

    async fn initialize(&mut self, _config: serde_json::Value) -> Result<()> {
        // Already initialized in `new()`.
        Ok(())
    }

    async fn execute(&self, ctx: &AgentContext) -> Result<AgentResult> {
        // Extract prompt and image URLs from extras.
        let image_urls: Vec<String> = ctx
            .extras
            .get("image_urls")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let files = if !image_urls.is_empty() {
            self.build_files(&image_urls, &ctx.user_id).await?
        } else {
            Vec::new()
        };

        match self.api_type {
            DifyApiType::Chat | DifyApiType::Agent | DifyApiType::Chatflow => {
                let conversation_id = self.session_store.get(&ctx.user_id).await;
                self.run_chat_mode(ctx, conversation_id, files).await
            }
            DifyApiType::Workflow => {
                self.run_workflow_mode(ctx, files).await
            }
        }
    }

    async fn execute_with_tools(
        &self,
        _ctx: &AgentContext,
        _tool_results: Vec<super::ToolResult>,
    ) -> Result<AgentResult> {
        // Dify handles tools internally (Agent mode); we don't feed back tool results.
        Err(AstrBotError::NotImplemented(
            "Dify runner does not support external tool result injection".to_string(),
        ))
    }

    async fn health_check(&self) -> Result<bool> {
        // Lightweight: try to hit the API base (no auth required for 401 check).
        let resp = self
            .api_client
            .http_client
            .get(&self.api_client.api_base)
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

    fn test_config(api_type: &str) -> AgentConfig {
        AgentConfig {
            id: "test-dify".to_string(),
            name: "Test Dify".to_string(),
            executor_type: "dify".to_string(),
            enabled: true,
            config: json!({
                "dify_api_key": "test-key",
                "dify_api_base": "https://api.dify.ai/v1",
                "dify_api_type": api_type,
                "timeout": 30,
                "streaming": false,
            }),
            system_prompt: None,
            enable_tools: false,
            max_iterations: 5,
        }
    }

    #[test]
    fn test_dify_api_type_from_str() {
        assert_eq!(DifyApiType::from_str("chat").unwrap(), DifyApiType::Chat);
        assert_eq!(DifyApiType::from_str("agent").unwrap(), DifyApiType::Agent);
        assert_eq!(DifyApiType::from_str("chatflow").unwrap(), DifyApiType::Chatflow);
        assert_eq!(DifyApiType::from_str("workflow").unwrap(), DifyApiType::Workflow);
        assert!(DifyApiType::from_str("unknown").is_err());
    }

    #[test]
    fn test_dify_runner_creation_chat() {
        let config = test_config("chat");
        let runner = DifyAgentRunner::new(&config).unwrap();
        assert_eq!(runner.name(), "dify");
        assert_eq!(runner.executor_type(), "chat");
        assert_eq!(runner.api_type, DifyApiType::Chat);
    }

    #[test]
    fn test_dify_runner_creation_workflow() {
        let config = test_config("workflow");
        let runner = DifyAgentRunner::new(&config).unwrap();
        assert_eq!(runner.executor_type(), "workflow");
        assert_eq!(runner.api_type, DifyApiType::Workflow);
        assert_eq!(runner.workflow_output_key, "astrbot_wf_output");
    }

    #[test]
    fn test_parse_data_uri_base64() {
        let uri = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let (mime, data) = parse_data_uri(uri).unwrap();
        assert_eq!(mime, "image/png");
        assert!(!data.is_empty());
    }

    #[tokio::test]
    async fn test_session_store_persistence() {
        let config = test_config("chat");
        let runner = DifyAgentRunner::new(&config).unwrap();
        runner.session_store.set("user_1", "conv_abc").await;
        assert_eq!(runner.session_store.get("user_1").await, Some("conv_abc".to_string()));
    }

    #[tokio::test]
    async fn test_workflow_output_parsing_string() {
        let config = test_config("workflow");
        let runner = DifyAgentRunner::new(&config).unwrap();
        let data = json!({
            "outputs": {
                "astrbot_wf_output": "Hello from workflow"
            }
        });
        let chain = runner.parse_workflow_output(&data).await.unwrap();
        assert_eq!(chain.0.len(), 1);
        match &chain.0[0] {
            MessageComponent::Plain { text } => assert_eq!(text, "Hello from workflow"),
            _ => panic!("Expected Plain component"),
        }
    }

    #[tokio::test]
    async fn test_workflow_output_parsing_image_array() {
        let config = test_config("workflow");
        let runner = DifyAgentRunner::new(&config).unwrap();
        let data = json!({
            "outputs": {
                "astrbot_wf_output": [
                    {
                        "dify_model_identity": "__dify__file__",
                        "type": "image",
                        "url": "https://example.com/img.png"
                    }
                ]
            }
        });
        let chain = runner.parse_workflow_output(&data).await.unwrap();
        assert_eq!(chain.0.len(), 1);
        match &chain.0[0] {
            MessageComponent::Image { url, .. } => assert_eq!(*url, Some("https://example.com/img.png".to_string())),
            _ => panic!("Expected Image component"),
        }
    }

    #[test]
    fn test_dify_file_serialize() {
        let f = DifyFile {
            file_type: "image".to_string(),
            transfer_method: "local_file".to_string(),
            upload_file_id: Some("file_123".to_string()),
            url: None,
        };
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains("\"type\":\"image\""));
        assert!(json.contains("\"transfer_method\":\"local_file\""));
        assert!(json.contains("\"upload_file_id\":\"file_123\""));
    }

    #[test]
    fn test_dify_chat_request_serialize() {
        let req = DifyChatRequest {
            inputs: HashMap::new(),
            query: "hello".to_string(),
            user: "user_1".to_string(),
            conversation_id: Some("conv_1".to_string()),
            files: vec![],
            response_mode: "streaming".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"query\":\"hello\""));
        assert!(json.contains("\"conversation_id\":\"conv_1\""));
    }
}
