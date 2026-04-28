use async_trait::async_trait;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::provider::{
    ChatMessage, ChatConfig, ChatResponse, ChatStreamChunk, ModelInfo, TokenUsage,
    Provider,
};
use futures_util::Stream;
use serde::{Deserialize, Serialize};

/// Zhipu AI (ChatGLM) provider
pub struct ZhipuProvider {
    id: String,
    name: String,
    api_key: String,
    base_url: String,
    default_model: String,
    client: reqwest::Client,
}

impl ZhipuProvider {
    pub fn new(id: String, api_key: String, base_url: String, default_model: String) -> Self {
        let base_url = if base_url.is_empty() {
            "https://open.bigmodel.cn/api/paas/v4".to_string()
        } else {
            base_url
        };
        Self {
            name: id.clone(),
            id,
            api_key,
            base_url,
            default_model,
            client: reqwest::Client::new(),
        }
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.api_key)
    }
}

#[derive(Debug, Serialize)]
struct ZpRequest {
    model: String,
    messages: Vec<ZpMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct ZpMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ZpResponse {
    choices: Vec<ZpChoice>,
    model: String,
    usage: ZpUsage,
}

#[derive(Debug, Deserialize)]
struct ZpChoice {
    message: ZpChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ZpChoiceMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct ZpUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct ZpStreamChunk {
    choices: Vec<ZpStreamChoice>,
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ZpStreamChoice {
    delta: ZpDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ZpDelta {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ZpModelsResponse {
    data: Vec<ZpModel>,
}

#[derive(Debug, Deserialize)]
struct ZpModel {
    id: String,
}

#[async_trait]
impl Provider for ZhipuProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    async fn models(&self) -> Result<Vec<String>> {
        let resp = self.client
            .get(format!("{}/models", self.base_url))
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| AstrBotError::Provider { provider: "Zhipu".to_string(), message: format!("models request failed: {}", e) })?;

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let data: ZpModelsResponse = resp.json().await
            .map_err(|e| AstrBotError::Provider { provider: "Zhipu".to_string(), message: format!("models JSON parse failed: {}", e) })?;

        Ok(data.data.into_iter().map(|m| m.id).collect())
    }

    async fn chat(&self, messages: Vec<ChatMessage>, config: ChatConfig) -> Result<ChatResponse> {
        let req_body = ZpRequest {
            model: config.model.unwrap_or_else(|| self.default_model.clone()),
            messages: messages.into_iter().map(|m| ZpMessage {
                role: m.role,
                content: m.content,
            }).collect(),
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            top_p: config.top_p,
            stream: false,
        };

        let resp = self.client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", self.auth_header())
            .json(&req_body)
            .send()
            .await
            .map_err(|e| AstrBotError::Provider { provider: "Zhipu".to_string(), message: format!("request failed: {}", e) })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Provider { provider: "Zhipu".to_string(), message: format!("HTTP {}: {}", status, text) });
        }

        let data: ZpResponse = resp.json().await
            .map_err(|e| AstrBotError::Provider { provider: "Zhipu".to_string(), message: format!("JSON parse failed: {}", e) })?;

        let choice = data.choices.into_iter().next()
            .ok_or_else(|| AstrBotError::Provider { provider: "Zhipu".to_string(), message: "empty choices".to_string() })?;

        Ok(ChatResponse {
            content: choice.message.content,
            model: data.model,
            usage: Some(TokenUsage {
                prompt_tokens: data.usage.prompt_tokens,
                completion_tokens: data.usage.completion_tokens,
                total_tokens: data.usage.total_tokens,
            }),
            reasoning: None,
        })
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        config: ChatConfig,
    ) -> Result<Box<dyn Stream<Item = Result<ChatStreamChunk>> + Send>> {
        let req_body = ZpRequest {
            model: config.model.unwrap_or_else(|| self.default_model.clone()),
            messages: messages.into_iter().map(|m| ZpMessage {
                role: m.role,
                content: m.content,
            }).collect(),
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            top_p: config.top_p,
            stream: true,
        };

        let resp = self.client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", self.auth_header())
            .json(&req_body)
            .send()
            .await
            .map_err(|e| AstrBotError::Provider { provider: "Zhipu".to_string(), message: format!("stream request failed: {}", e) })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Provider { provider: "Zhipu".to_string(), message: format!("stream HTTP {}: {}", status, text) });
        }

        let default_model = self.default_model.clone();
        let stream = resp.bytes_stream();
        let mapped = futures_util::stream::try_unfold(
            (stream, String::new()),
            move |(mut stream, mut buffer)| {
                let default_model = default_model.clone();
                async move {
                    use futures_util::StreamExt;
                    let chunk = stream.next().await;
                    if chunk.is_none() {
                        return Ok(None);
                    }
                    let bytes = chunk.unwrap().map_err(|e| AstrBotError::Provider { provider: "Zhipu".to_string(), message: format!("SSE stream error: {}", e) })?;
                    buffer.push_str(&String::from_utf8_lossy(&bytes));

                    let mut lines = buffer.split("\n").peekable();
                    let mut consumed = 0usize;
                    let mut delta = String::new();
                    let mut finish_reason = None::<String>;
                    let mut model = default_model.clone();

                    while let Some(line) = lines.next() {
                        consumed += line.len() + 1;
                        let line = line.strip_prefix("data: ").unwrap_or(line);
                        if line == "[DONE]" || line.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<ZpStreamChunk>(line) {
                            Ok(chunk) => {
                                if let Some(m) = chunk.model {
                                    model = m;
                                }
                                if let Some(choice) = chunk.choices.into_iter().next() {
                                    if let Some(text) = choice.delta.content {
                                        delta.push_str(&text);
                                    }
                                    if choice.finish_reason.is_some() {
                                        finish_reason = choice.finish_reason;
                                    }
                                }
                            }
                            Err(_) => {}
                        }
                    }
                    buffer = buffer.split_off(consumed.saturating_sub(1));
                    Ok(Some((ChatStreamChunk {
                        delta,
                        finish_reason,
                        model,
                    }, (stream, buffer))))
                }
            },
        );

        Ok(Box::new(mapped))
    }

    async fn embedding(&self, _texts: Vec<String>, _model: Option<String>) -> Result<Vec<Vec<f32>>> {
        Err(AstrBotError::NotImplemented("Zhipu embedding not supported".to_string()))
    }

    async fn model_info(&self, _model: &str) -> Result<ModelInfo> {
        Ok(ModelInfo {
            name: self.default_model.clone(),
            context_length: 128000,
            supports_streaming: true,
            supports_vision: false,
            supports_function_calling: false,
        })
    }

    async fn health_check(&self) -> Result<bool> {
        match self.models().await {
            Ok(models) => Ok(!models.is_empty()),
            Err(_) => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zhipu_provider_new() {
        let provider = ZhipuProvider::new(
            "zp-test".to_string(),
            "test-key".to_string(),
            "".to_string(),
            "glm-4".to_string(),
        );
        assert_eq!(provider.id(), "zp-test");
        assert_eq!(provider.default_model, "glm-4");
    }

    #[test]
    fn test_zhipu_default_base_url() {
        let provider = ZhipuProvider::new(
            "zp-test".to_string(),
            "test-key".to_string(),
            "".to_string(),
            "model".to_string(),
        );
        assert_eq!(provider.base_url, "https://open.bigmodel.cn/api/paas/v4");
    }
}
