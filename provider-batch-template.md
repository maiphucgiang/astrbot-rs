# Provider 批量填充模板

## 设计思路

所有 OpenAI-compatible Provider 共享同一套 HTTP 调用逻辑，差异仅在：
1. `base_url`
2. `model_name` 映射
3. 部分 Provider 需要额外的 headers（如 `Authorization: Bearer` 格式一致）

## 通用 Trait 设计

```rust
// crates/astrbot-provider/src/openai_compatible/mod.rs

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// OpenAI API 兼容的 Provider 通用实现
pub struct OpenAiCompatibleProvider {
    client: Client,
    config: ProviderConfig,
}

#[derive(Clone, Debug)]
pub struct ProviderConfig {
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub extra_headers: Option<Vec<(String, String)>>,
}

#[async_trait]
pub trait ChatProvider: Send + Sync {
    async fn chat(&self, messages: Vec<Message>, options: ChatOptions) -> Result<String, ProviderError>;
    async fn stream_chat(&self, messages: Vec<Message>, options: ChatOptions) -> Result<BoxStream<'static, Result<String, ProviderError>>, ProviderError>;
    fn supports_streaming(&self) -> bool;
    fn list_models(&self) -> Vec<String>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Default)]
pub struct ChatOptions {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f32>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error: {status} - {message}")]
    Api { status: u16, message: String },
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
```

## 通用实现

```rust
#[async_trait]
impl ChatProvider for OpenAiCompatibleProvider {
    async fn chat(&self, messages: Vec<Message>, options: ChatOptions) -> Result<String, ProviderError> {
        let body = serde_json::json!({
            "model": self.config.model,
            "messages": messages,
            "temperature": options.temperature.unwrap_or(0.7),
            "max_tokens": options.max_tokens,
            "top_p": options.top_p.unwrap_or(1.0),
        });

        let mut request = self.client
            .post(format!("{}/v1/chat/completions", self.config.base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&body);

        if let Some(headers) = &self.config.extra_headers {
            for (k, v) in headers {
                request = request.header(k, v);
            }
        }

        let response = request.send().await?;
        let status = response.status();

        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(ProviderError::Api { status: status.as_u16(), message: text });
        }

        let json: Value = response.json().await?;
        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        Ok(content)
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    async fn stream_chat(&self, _messages: Vec<Message>, _options: ChatOptions) -> Result<BoxStream<'static, Result<String, ProviderError>>, ProviderError> {
        // TODO: SSE streaming implementation
        todo!("SSE streaming")
    }

    fn list_models(&self) -> Vec<String> {
        vec![self.config.model.clone()]
    }
}
```

## 各 Provider 仅需配置

```rust
// crates/astrbot-provider/src/sources/moonshot.rs
pub fn create_moonshot_provider(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "Moonshot AI".to_string(),
        base_url: "https://api.moonshot.cn/v1".to_string(),
        api_key,
        model,
        extra_headers: None,
    })
}

// crates/astrbot-provider/src/sources/deepseek.rs
pub fn create_deepseek_provider(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "DeepSeek".to_string(),
        base_url: "https://api.deepseek.com/v1".to_string(),
        api_key,
        model,
        extra_headers: None,
    })
}

// crates/astrbot-provider/src/sources/groq.rs
pub fn create_groq_provider(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "Groq".to_string(),
        base_url: "https://api.groq.com/openai/v1".to_string(),
        api_key,
        model,
        extra_headers: None,
    })
}

// crates/astrbot-provider/src/sources/openrouter.rs
pub fn create_openrouter_provider(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "OpenRouter".to_string(),
        base_url: "https://openrouter.ai/api/v1".to_string(),
        api_key,
        model,
        extra_headers: Some(vec![
            ("HTTP-Referer".to_string(), "https://astrbot.com".to_string()),
            ("X-Title".to_string(), "AstrBot".to_string()),
        ]),
    })
}

// crates/astrbot-provider/src/sources/siliconflow.rs
pub fn create_siliconflow_provider(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "SiliconFlow".to_string(),
        base_url: "https://api.siliconflow.cn/v1".to_string(),
        api_key,
        model,
        extra_headers: None,
    })
}

// crates/astrbot-provider/src/sources/oneapi.rs
pub fn create_oneapi_provider(base_url: String, api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "OneAPI".to_string(),
        base_url: format!("{}/v1", base_url.trim_end_matches('/')),
        api_key,
        model,
        extra_headers: None,
    })
}

// crates/astrbot-provider/src/sources/lmstudio.rs
pub fn create_lmstudio_provider(base_url: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "LM Studio".to_string(),
        base_url: format!("{}/v1", base_url.trim_end_matches('/')),
        api_key: "lm-studio".to_string(), // LM Studio usually doesn't need API key
        model,
        extra_headers: None,
    })
}

// crates/astrbot-provider/src/sources/zhipu.rs
pub fn create_zhipu_provider(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "Zhipu AI".to_string(),
        base_url: "https://open.bigmodel.cn/api/paas/v4".to_string(),
        api_key,
        model,
        extra_headers: Some(vec![
            ("Authorization".to_string(), format!("Bearer {}", api_key)),
        ]),
    })
}

// crates/astrbot-provider/src/sources/xai.rs
pub fn create_xai_provider(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "xAI (Grok)".to_string(),
        base_url: "https://api.x.ai/v1".to_string(),
        api_key,
        model,
        extra_headers: None,
    })
}

// crates/astrbot-provider/src/sources/minimax.rs
pub fn create_minimax_provider(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "MiniMax".to_string(),
        base_url: "https://api.minimax.chat/v1".to_string(),
        api_key,
        model,
        extra_headers: None,
    })
}

// crates/astrbot-provider/src/sources/volcengine.rs
pub fn create_volcengine_provider(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "Volcano Engine".to_string(),
        base_url: "https://ark.cn-beijing.volces.com/api/v3".to_string(),
        api_key,
        model,
        extra_headers: None,
    })
}
```

## 非 OpenAI-compatible Provider（需要独立实现）

```rust
// crates/astrbot-provider/src/sources/dify.rs
// Dify 需要调用 /chat-messages 接口，非 OpenAI 格式

// crates/astrbot-provider/src/sources/coze.rs
// Coze 有独立 API 格式

// crates/astrbot-provider/src/sources/bailian.rs
// 阿里云百炼需要特殊签名
```

## TTS Provider 通用模板

```rust
// crates/astrbot-provider/src/tts/openai_compatible.rs

#[async_trait]
pub trait TtsProvider: Send + Sync {
    async fn synthesize(&self, text: &str, voice: Option<&str>) -> Result<Vec<u8>, TtsError>;
    fn supported_voices(&self) -> Vec<VoiceInfo>;
}

pub struct OpenAiCompatibleTts {
    client: Client,
    config: TtsConfig,
}

#[derive(Clone)]
pub struct TtsConfig {
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub default_voice: String,
    pub model: String,
}

#[async_trait]
impl TtsProvider for OpenAiCompatibleTts {
    async fn synthesize(&self, text: &str, voice: Option<&str>) -> Result<Vec<u8>, TtsError> {
        let body = serde_json::json!({
            "model": self.config.model,
            "input": text,
            "voice": voice.unwrap_or(&self.config.default_voice),
        });

        let response = self.client
            .post(format!("{}/v1/audio/speech", self.config.base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&body)
            .send()
            .await?;

        let bytes = response.bytes().await?.to_vec();
        Ok(bytes)
    }

    fn supported_voices(&self) -> Vec<VoiceInfo> {
        vec![] // TODO: fetch from API or hardcode
    }
}
```

## 注册表

```rust
// crates/astrbot-provider/src/registry.rs

use std::collections::HashMap;

pub struct ProviderRegistry {
    llm_factories: HashMap<String, Box<dyn Fn(ProviderConfig) -> Box<dyn ChatProvider>>>,
    tts_factories: HashMap<String, Box<dyn Fn(TtsConfig) -> Box<dyn TtsProvider>>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            llm_factories: HashMap::new(),
            tts_factories: HashMap::new(),
        };

        // OpenAI-compatible providers
        registry.register_llm("moonshot", |c| Box::new(create_moonshot_provider(c.api_key, c.model)));
        registry.register_llm("deepseek", |c| Box::new(create_deepseek_provider(c.api_key, c.model)));
        registry.register_llm("groq", |c| Box::new(create_groq_provider(c.api_key, c.model)));
        registry.register_llm("openrouter", |c| Box::new(create_openrouter_provider(c.api_key, c.model)));
        registry.register_llm("siliconflow", |c| Box::new(create_siliconflow_provider(c.api_key, c.model)));
        registry.register_llm("oneapi", |c| Box::new(create_oneapi_provider(c.base_url, c.api_key, c.model)));
        registry.register_llm("lmstudio", |c| Box::new(create_lmstudio_provider(c.base_url, c.model)));
        registry.register_llm("zhipu", |c| Box::new(create_zhipu_provider(c.api_key, c.model)));
        registry.register_llm("xai", |c| Box::new(create_xai_provider(c.api_key, c.model)));
        registry.register_llm("minimax", |c| Box::new(create_minimax_provider(c.api_key, c.model)));
        registry.register_llm("volcengine", |c| Box::new(create_volcengine_provider(c.api_key, c.model)));

        registry
    }

    pub fn register_llm(
        &mut self,
        name: &str,
        factory: impl Fn(ProviderConfig) -> Box<dyn ChatProvider> + 'static,
    ) {
        self.llm_factories.insert(name.to_string(), Box::new(factory));
    }
}
```

## Cargo.toml 依赖

```toml
[dependencies]
async-trait = "0.1"
reqwest = { version = "0.12", features = ["json", "stream"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "1.0"
tokio = { version = "1", features = ["full"] }
futures = "0.3"
```
