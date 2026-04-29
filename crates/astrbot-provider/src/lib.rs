pub mod client;
pub mod openai;
pub mod openai_compatible;
pub mod sources;
pub mod registry;
pub mod tts;

// New providers from Redmao Phase 2
pub mod ai21;
pub mod azure;
pub mod baichuan;
pub mod cohere;
pub mod fireworks;
pub mod groq;
pub mod openrouter;
pub mod perplexity;
pub mod together;
pub mod zerooneai;
pub mod baidu;

pub use openai_compatible::*;
pub use registry::*;
pub use tts::*;

use async_trait::async_trait;
use astrbot_core::{AstrMessage, MessageContent};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// LLM Chat Provider trait
#[async_trait]
pub trait ChatProvider: Send + Sync {
    /// 模型提供商名称
    fn name(&self) -> &str;

    /// 单次对话（非流式）
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        options: ChatOptions,
    ) -> Result<String, ProviderError>;

    /// 是否支持流式输出
    fn supports_streaming(&self) -> bool {
        true
    }

    /// 流式对话
    async fn stream_chat(
        &self,
        _messages: Vec<ChatMessage>,
        _options: ChatOptions,
    ) -> Result<Box<dyn futures::Stream<Item = Result<String, ProviderError>> + Send>, ProviderError> {
        Err(ProviderError::NotImplemented("streaming".to_string()))
    }

    /// 列出可用模型
    fn list_models(&self) -> Vec<String>;

    /// 是否可用
    fn is_available(&self) -> bool {
        true
    }
}

/// 文本嵌入 Provider trait
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// 模型提供商名称
    fn name(&self) -> &str;

    /// 将文本列表转换为 embedding 向量
    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, ProviderError>;

    /// 单个文本嵌入（便利方法）
    async fn embed_one(&self, text: String) -> Result<Vec<f32>, ProviderError> {
        let mut results = self.embed(vec![text]).await?;
        results.pop().ok_or_else(|| ProviderError::Unavailable("empty embedding response".to_string()))
    }
}

/// 聊天消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: &str) -> Self {
        Self {
            role: "user".to_string(),
            content: content.to_string(),
        }
    }

    pub fn assistant(content: &str) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.to_string(),
        }
    }

    pub fn system(content: &str) -> Self {
        Self {
            role: "system".to_string(),
            content: content.to_string(),
        }
    }
}

/// 对话选项
#[derive(Debug, Clone, Default)]
pub struct ChatOptions {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f32>,
    pub model: Option<String>,
}

/// Provider 配置
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub extra_headers: Option<Vec<(String, String)>>,
}

/// Provider 错误
#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error {status}: {message}")]
    Api { status: u16, message: String },
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Not implemented: {0}")]
    NotImplemented(String),
    #[error("Provider unavailable: {0}")]
    Unavailable(String),
}
