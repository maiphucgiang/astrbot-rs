use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use crate::{ChatMessage, ChatOptions, ChatProvider, ProviderConfig, ProviderError};

/// OpenAI API 兼容的 Provider 通用实现
pub struct OpenAiCompatibleProvider {
    client: Client,
    config: ProviderConfig,
}

impl OpenAiCompatibleProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }

    pub fn config(&self) -> &ProviderConfig {
        &self.config
    }
}

#[async_trait]
impl ChatProvider for OpenAiCompatibleProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        options: ChatOptions,
    ) -> Result<String, ProviderError> {
        let model = options.model.as_ref().unwrap_or(&self.config.model);

        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": options.temperature.unwrap_or(0.7),
            "max_tokens": options.max_tokens,
            "top_p": options.top_p.unwrap_or(1.0),
        });

        let mut request = self
            .client
            .post(format!("{}/v1/chat/completions", self.config.base_url.trim_end_matches('/')))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
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
            return Err(ProviderError::Api {
                status: status.as_u16(),
                message: text,
            });
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

    fn list_models(&self) -> Vec<String> {
        vec![self.config.model.clone()]
    }
}
