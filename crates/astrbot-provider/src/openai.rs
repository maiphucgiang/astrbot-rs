use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::provider::{
    ChatConfig, ChatMessage, ChatResponse, ChatStreamChunk, ModelInfo, Provider, TokenUsage,
};
use async_trait::async_trait;
use futures_util::Stream;
use serde::{Deserialize, Serialize};

/// OpenAI-compatible provider
pub struct OpenAiProvider {
    id: String,
    name: String,
    api_key: String,
    base_url: String,
    default_model: String,
    client: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new(id: String, api_key: String, base_url: String, default_model: String) -> Self {
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
struct OpenAIRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct OpenAIMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
    model: String,
    usage: OpenAIUsage,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    message: OpenAIChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoiceMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamResponse {
    choices: Vec<OpenAIStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamChoice {
    delta: OpenAIStreamDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamDelta {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIEmbeddingResponse {
    data: Vec<OpenAIEmbedding>,
    #[allow(dead_code)]
    model: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIEmbedding {
    embedding: Vec<f32>,
}

#[async_trait]
impl Provider for OpenAiProvider {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }

    async fn models(&self) -> Result<Vec<String>> {
        Ok(vec![self.default_model.clone()])
    }

    async fn chat(&self, messages: Vec<ChatMessage>, config: ChatConfig) -> Result<ChatResponse> {
        let model = config.model.unwrap_or_else(|| self.default_model.clone());
        let req_messages: Vec<OpenAIMessage> = messages
            .into_iter()
            .map(|m| OpenAIMessage {
                role: m.role,
                content: m.content,
            })
            .collect();

        let request = OpenAIRequest {
            model,
            messages: req_messages,
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            top_p: config.top_p,
            stream: false,
        };

        let url = format!("{}/v1/chat/completions", self.base_url);
        let response = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(&request)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("OpenAI request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Provider {
                provider: self.name.clone(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        let resp: OpenAIResponse = response.json().await.map_err(|e| {
            AstrBotError::Serialization(format!("Failed to parse OpenAI response: {}", e))
        })?;

        let content = resp
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default();

        Ok(ChatResponse {
            content,
            model: resp.model,
            usage: Some(TokenUsage {
                prompt_tokens: resp.usage.prompt_tokens,
                completion_tokens: resp.usage.completion_tokens,
                total_tokens: resp.usage.total_tokens,
            }),
            reasoning: None,
            tool_calls: None,
        })
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        config: ChatConfig,
    ) -> Result<Box<dyn Stream<Item = Result<ChatStreamChunk>> + Send>> {
        let model = config.model.unwrap_or_else(|| self.default_model.clone());
        let req_messages: Vec<OpenAIMessage> = messages
            .into_iter()
            .map(|m| OpenAIMessage {
                role: m.role,
                content: m.content,
            })
            .collect();

        let request = OpenAIRequest {
            model,
            messages: req_messages,
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            top_p: config.top_p,
            stream: true,
        };

        let url = format!("{}/v1/chat/completions", self.base_url);
        let response = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(&request)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("OpenAI stream request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Provider {
                provider: self.name.clone(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        let model_name = self.default_model.clone();

        // Use a custom stream that processes SSE events
        let stream = futures_util::stream::unfold(
            (response, model_name, String::new()),
            |(mut response, model_name, mut buffer)| async move {
                match response.chunk().await {
                    Ok(Some(chunk)) => {
                        let text = String::from_utf8_lossy(&chunk);
                        buffer.push_str(&text);

                        let mut delta = String::new();
                        let mut finish_reason = None;
                        let mut remaining = String::new();

                        for line in buffer.lines() {
                            if line.is_empty() {
                                continue;
                            }
                            if line.starts_with("data: ") {
                                let data = &line[6..];
                                if data == "[DONE]" {
                                    finish_reason = Some("stop".to_string());
                                    continue;
                                }
                                if let Ok(parsed) =
                                    serde_json::from_str::<OpenAIStreamResponse>(data)
                                {
                                    if let Some(choice) = parsed.choices.into_iter().next() {
                                        if let Some(content) = choice.delta.content {
                                            delta.push_str(&content);
                                        }
                                        if choice.finish_reason.is_some() {
                                            finish_reason = choice.finish_reason;
                                        }
                                    }
                                }
                            } else {
                                remaining.push_str(line);
                                remaining.push('\n');
                            }
                        }

                        buffer = remaining;

                        if !delta.is_empty() || finish_reason.is_some() {
                            let chunk = ChatStreamChunk {
                                delta,
                                finish_reason,
                                model: model_name.clone(),
                            };
                            Some((Ok(chunk), (response, model_name, buffer)))
                        } else {
                            // Continue reading
                            Some((
                                Ok(ChatStreamChunk {
                                    delta: String::new(),
                                    finish_reason: None,
                                    model: model_name.clone(),
                                }),
                                (response, model_name, buffer),
                            ))
                        }
                    }
                    Ok(None) => None, // Stream ended
                    Err(e) => Some((
                        Err(AstrBotError::Network(format!("Stream error: {}", e))),
                        (response, model_name, buffer),
                    )),
                }
            },
        );

        Ok(Box::new(stream))
    }

    async fn embedding(&self, texts: Vec<String>, _model: Option<String>) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/v1/embeddings", self.base_url);
        let request_body = serde_json::json!({
            "model": self.default_model,
            "input": texts,
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(&request_body)
            .send()
            .await
            .map_err(|e| {
                AstrBotError::Network(format!("OpenAI embedding request failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Provider {
                provider: self.name.clone(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        let resp: OpenAIEmbeddingResponse = response.json().await.map_err(|e| {
            AstrBotError::Serialization(format!("Failed to parse embedding response: {}", e))
        })?;

        Ok(resp.data.into_iter().map(|d| d.embedding).collect())
    }

    async fn model_info(&self, model: &str) -> Result<ModelInfo> {
        // Default model info for OpenAI models
        let context_length = if model.starts_with("gpt-4") {
            128000
        } else if model.starts_with("gpt-3.5") {
            16385
        } else {
            4096
        };

        Ok(ModelInfo {
            name: model.to_string(),
            context_length,
            supports_streaming: true,
            supports_vision: model.contains("vision") || model.contains("4o"),
            supports_function_calling: model.starts_with("gpt-4")
                || model.starts_with("gpt-3.5-turbo"),
        })
    }

    async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/v1/models", self.base_url);
        match self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
        {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }
}
