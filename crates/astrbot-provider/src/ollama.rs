use async_trait::async_trait;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::provider::{
    ChatMessage, ChatConfig, ChatResponse, ChatStreamChunk, ModelInfo, TokenUsage,
    Provider,
};
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Ollama local LLM provider
///
/// Ollama API: https://github.com/ollama/ollama/blob/main/docs/api.md
pub struct OllamaProvider {
    id: String,
    name: String,
    base_url: String,
    default_model: String,
    client: reqwest::Client,
}

impl OllamaProvider {
    pub fn new(id: String, base_url: String, default_model: String) -> Self {
        Self {
            name: id.clone(),
            id,
            base_url: base_url.trim_end_matches('/').to_string(),
            default_model,
            client: reqwest::Client::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Request / Response models
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,  // max_tokens equivalent
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    model: String,
    message: OllamaMessage,
    #[serde(default)]
    done: bool,
    #[serde(rename = "prompt_eval_count", default)]
    prompt_eval_count: u32,
    #[serde(rename = "eval_count", default)]
    eval_count: u32,
    #[serde(rename = "total_duration", default)]
    total_duration: u64,
}

// Stream response (NDJSON)
#[derive(Debug, Deserialize)]
struct OllamaStreamResponse {
    model: String,
    message: OllamaStreamMessage,
    #[serde(default)]
    done: bool,
}

#[derive(Debug, Deserialize)]
struct OllamaStreamMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct OllamaModelTag {
    name: String,
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModelTag>,
}

#[derive(Debug, Serialize)]
struct OllamaEmbeddingRequest {
    model: String,
    prompt: String,
}

#[derive(Debug, Deserialize)]
struct OllamaEmbeddingResponse {
    embedding: Vec<f32>,
}

// ---------------------------------------------------------------------------
// Provider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl Provider for OllamaProvider {
    fn id(&self) -> &str { &self.id }
    fn name(&self) -> &str { &self.name }

    async fn models(&self) -> Result<Vec<String>> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = self.client.get(&url).send().await
            .map_err(|e| AstrBotError::Network(format!("Ollama tags request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Provider {
                provider: self.name.clone(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        let data: OllamaTagsResponse = resp.json().await
            .map_err(|e| AstrBotError::Serialization(format!("Ollama tags parse failed: {}", e)))?;

        let models: Vec<String> = data.models.into_iter().map(|m| m.name).collect();
        Ok(models)
    }

    async fn chat(&self, messages: Vec<ChatMessage>, config: ChatConfig) -> Result<ChatResponse> {
        let model = config.model.unwrap_or_else(|| self.default_model.clone());
        let req_messages: Vec<OllamaMessage> = messages.into_iter()
            .map(|m| OllamaMessage { role: m.role, content: m.content })
            .collect();

        let options = if config.temperature.is_some() || config.max_tokens.is_some() || config.top_p.is_some() {
            Some(OllamaOptions {
                temperature: config.temperature,
                num_predict: config.max_tokens,
                top_p: config.top_p,
            })
        } else {
            None
        };

        let request = OllamaChatRequest {
            model,
            messages: req_messages,
            stream: false,
            options,
        };

        let url = format!("{}/api/chat", self.base_url);
        let response = self.client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Ollama chat request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Provider {
                provider: self.name.clone(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        let resp: OllamaChatResponse = response.json().await
            .map_err(|e| AstrBotError::Serialization(format!("Ollama chat parse failed: {}", e)))?;

        let total_tokens = resp.prompt_eval_count + resp.eval_count;

        Ok(ChatResponse {
            content: resp.message.content,
            model: resp.model,
            usage: Some(TokenUsage {
                prompt_tokens: resp.prompt_eval_count,
                completion_tokens: resp.eval_count,
                total_tokens,
            }),
            reasoning: None,
        })
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        config: ChatConfig,
    ) -> Result<Box<dyn Stream<Item = Result<ChatStreamChunk>> + Send>> {
        let model = config.model.unwrap_or_else(|| self.default_model.clone());
        let req_messages: Vec<OllamaMessage> = messages.into_iter()
            .map(|m| OllamaMessage { role: m.role, content: m.content })
            .collect();

        let options = if config.temperature.is_some() || config.max_tokens.is_some() || config.top_p.is_some() {
            Some(OllamaOptions {
                temperature: config.temperature,
                num_predict: config.max_tokens,
                top_p: config.top_p,
            })
        } else {
            None
        };

        let request = OllamaChatRequest {
            model,
            messages: req_messages,
            stream: true,
            options,
        };

        let url = format!("{}/api/chat", self.base_url);
        let response = self.client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Ollama stream request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Provider {
                provider: self.name.clone(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        let model_name = self.default_model.clone();

        // NDJSON stream: each line is a JSON object
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
                            if let Ok(parsed) = serde_json::from_str::<OllamaStreamResponse>(line) {
                                delta.push_str(&parsed.message.content);
                                if parsed.done {
                                    finish_reason = Some("stop".to_string());
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
                            // Yield empty chunk to keep stream alive
                            Some((Ok(ChatStreamChunk {
                                delta: String::new(),
                                finish_reason: None,
                                model: model_name.clone(),
                            }), (response, model_name, buffer)))
                        }
                    }
                    Ok(None) => None,
                    Err(e) => Some((Err(AstrBotError::Network(format!("Ollama stream error: {}", e))), (response, model_name, buffer))),
                }
            },
        );

        Ok(Box::new(stream))
    }

    async fn embedding(&self, texts: Vec<String>, model: Option<String>) -> Result<Vec<Vec<f32>>> {
        let model = model.unwrap_or_else(|| self.default_model.clone());
        let mut results = Vec::with_capacity(texts.len());

        for text in texts {
            let request = OllamaEmbeddingRequest {
                model: model.clone(),
                prompt: text,
            };

            let url = format!("{}/api/embeddings", self.base_url);
            let response = self.client
                .post(&url)
                .json(&request)
                .send()
                .await
                .map_err(|e| AstrBotError::Network(format!("Ollama embedding request failed: {}", e)))?;

            if !response.status().is_success() {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                return Err(AstrBotError::Provider {
                    provider: self.name.clone(),
                    message: format!("HTTP {}: {}", status, text),
                });
            }

            let resp: OllamaEmbeddingResponse = response.json().await
                .map_err(|e| AstrBotError::Serialization(format!("Ollama embedding parse failed: {}", e)))?;

            results.push(resp.embedding);
        }

        Ok(results)
    }

    async fn model_info(&self, model: &str) -> Result<ModelInfo> {
        // Ollama models typically support streaming; vision/function calling varies by model
        let supports_vision = model.contains("llava")
            || model.contains("vision")
            || model.contains("bakllava")
            || model.contains("moondream");

        let supports_function_calling = model.contains("llama3")
            || model.contains("qwen2.5")
            || model.contains("mistral")
            || model.contains("nemotron");

        Ok(ModelInfo {
            name: model.to_string(),
            context_length: 128000,  // Most local models default to 128k context
            supports_streaming: true,
            supports_vision,
            supports_function_calling,
        })
    }

    async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/api/tags", self.base_url);
        match self.client.get(&url).send().await {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }
}
