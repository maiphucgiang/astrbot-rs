use crate::errors::Result;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::Stream;

// ---------------------------------------------------------------------------
// TTS Provider trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait TtsProvider: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    async fn synthesize(&self, text: &str) -> Result<Bytes>;
    async fn health_check(&self) -> Result<()>;
    async fn voices(&self) -> Result<Vec<String>>;
}

// ---------------------------------------------------------------------------
// STT Provider trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait SttProvider: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    async fn transcribe(&self, audio: Bytes) -> Result<String>;
    async fn health_check(&self) -> Result<()>;
    fn supported_formats(&self) -> Vec<String>;
}

/// Trait for LLM providers
#[async_trait]
pub trait Provider: Send + Sync {
    /// Get provider ID
    fn id(&self) -> &str;
    /// Get provider name
    fn name(&self) -> &str;
    /// Get supported models
    async fn models(&self) -> Result<Vec<String>>;
    /// Send a chat completion request (non-streaming)
    async fn chat(&self, messages: Vec<ChatMessage>, config: ChatConfig) -> Result<ChatResponse>;
    /// Send a chat completion request (streaming)
    /// Returns a stream of partial response chunks
    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        config: ChatConfig,
    ) -> Result<Box<dyn Stream<Item = Result<ChatStreamChunk>> + Send>>;
    /// Generate embeddings for text
    async fn embedding(&self, texts: Vec<String>, model: Option<String>) -> Result<Vec<Vec<f32>>>;
    /// Get model info (capabilities, context length, etc.)
    async fn model_info(&self, model: &str) -> Result<ModelInfo>;
    /// Check if provider is healthy
    async fn health_check(&self) -> Result<bool>;
}

/// A chat message for LLM interaction
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Tool calls made by the assistant (for function calling)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<crate::tools::ToolCall>>,
    /// Tool call ID this message is responding to (role=tool)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn assistant_with_tools(
        content: impl Into<String>,
        tool_calls: Vec<crate::tools::ToolCall>,
    ) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
            name: None,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        }
    }
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: content.into(),
            name: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// Configuration for a chat completion request
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ChatConfig {
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f32>,
    pub stream: bool,
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

/// Response from a chat completion
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub usage: Option<TokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Tool calls requested by the model (for function calling)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<crate::tools::ToolCall>>,
}

/// A chunk from a streaming response
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatStreamChunk {
    /// Delta content (partial text)
    pub delta: String,
    /// Whether this is the final chunk
    pub finish_reason: Option<String>,
    /// Model name
    pub model: String,
}

/// Token usage information
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Model information
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub context_length: u32,
    pub supports_streaming: bool,
    pub supports_vision: bool,
    pub supports_function_calling: bool,
}

/// Metadata about a provider
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderMetaData {
    pub id: String,
    pub name: String,
    pub provider_type: String,
    pub enabled: bool,
    pub models: Vec<String>,
}

/// Provider configuration (loaded from config file)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub provider_type: String,
    pub enabled: bool,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub default_model: String,
    pub models: Vec<String>,
    #[serde(default)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}
