use async_trait::async_trait;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::provider::{
    ChatMessage, ChatConfig, ChatResponse, ChatStreamChunk, ModelInfo, TokenUsage,
    Provider,
};
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Google Gemini provider
pub struct GeminiProvider {
    id: String,
    name: String,
    api_key: String,
    base_url: String,
    default_model: String,
    client: reqwest::Client,
}

impl GeminiProvider {
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

    fn convert_messages(&self, messages: Vec<ChatMessage>) -> Vec<GeminiContent> {
        messages.into_iter()
            .map(|msg| {
                let role = match msg.role.as_str() {
                    "user" => "user".to_string(),
                    "assistant" | "model" => "model".to_string(),
                    _ => "user".to_string(),
                };
                GeminiContent {
                    role,
                    parts: vec![Part::text(msg.content)],
                }
            })
            .collect()
    }

    fn build_url(&self, model: &str, stream: bool) -> String {
        let endpoint = if stream {
            "streamGenerateContent"
        } else {
            "generateContent"
        };
        format!(
            "{}/v1beta/models/{}:{}?key={}",
            self.base_url, model, endpoint, self.api_key
        )
    }
}

#[derive(Debug, Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiContent {
    role: String,
    parts: Vec<Part>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Part {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

impl Part {
    fn text(content: String) -> Self {
        Self {
            text: Some(content),
        }
    }
}

#[derive(Debug, Serialize)]
struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct GeminiResponse {
    candidates: Vec<Candidate>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<UsageMetadata>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    content: GeminiContent,
    #[serde(rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: Option<u32>,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: Option<u32>,
    #[serde(rename = "totalTokenCount")]
    total_token_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct GeminiStreamChunk {
    candidates: Vec<Candidate>,
}

#[async_trait]
impl Provider for GeminiProvider {
    fn id(&self) -> &str { &self.id }
    fn name(&self) -> &str { &self.name }

    async fn models(&self) -> Result<Vec<String>> {
        Ok(vec![
            "gemini-1.5-pro".to_string(),
            "gemini-1.5-flash".to_string(),
            "gemini-1.0-pro".to_string(),
            "gemini-1.0-pro-vision-latest".to_string(),
            self.default_model.clone(),
        ])
    }

    async fn chat(&self, messages: Vec<ChatMessage>, config: ChatConfig) -> Result<ChatResponse> {
        let model = config.model.unwrap_or_else(|| self.default_model.clone());
        let contents = self.convert_messages(messages);

        let request = GeminiRequest {
            contents,
            generation_config: Some(GenerationConfig {
                temperature: config.temperature,
                top_p: config.top_p,
                max_output_tokens: config.max_tokens,
            }),
        };

        let url = self.build_url(&model, false);
        let response = self.client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Gemini request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Provider {
                provider: self.name.clone(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        let resp: GeminiResponse = response.json().await
            .map_err(|e| AstrBotError::Serialization(format!("Failed to parse Gemini response: {}", e)))?;

        let content = resp.candidates.into_iter()
            .next()
            .and_then(|c| c.content.parts.into_iter().next())
            .and_then(|p| p.text)
            .unwrap_or_default();

        let usage = resp.usage_metadata.map(|u| TokenUsage {
            prompt_tokens: u.prompt_token_count.unwrap_or(0),
            completion_tokens: u.candidates_token_count.unwrap_or(0),
            total_tokens: u.total_token_count.unwrap_or(0),
        });

        Ok(ChatResponse {
            content,
            model: model.clone(),
            usage,
            reasoning: None,
        })
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        config: ChatConfig,
    ) -> Result<Box<dyn Stream<Item = Result<ChatStreamChunk>> + Send>> {
        let model = config.model.unwrap_or_else(|| self.default_model.clone());
        let contents = self.convert_messages(messages);

        let request = GeminiRequest {
            contents,
            generation_config: Some(GenerationConfig {
                temperature: config.temperature,
                top_p: config.top_p,
                max_output_tokens: config.max_tokens,
            }),
        };

        let url = self.build_url(&model, true);
        let response = self.client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Gemini stream request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Provider {
                provider: self.name.clone(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        let model_name = model.clone();
        
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
                        
                        // Gemini streaming returns JSON objects per line
                        for line in buffer.lines() {
                            let line = line.trim();
                            if line.is_empty() || line == "," {
                                continue;
                            }
                            
                            // Remove array brackets if present
                            let line = line.trim_start_matches('[').trim_end_matches(']');
                            let line = line.trim_start_matches(',').trim();
                            
                            if let Ok(chunk_data) = serde_json::from_str::<GeminiStreamChunk>(line) {
                                if let Some(candidate) = chunk_data.candidates.into_iter().next() {
                                    if let Some(part) = candidate.content.parts.into_iter().next() {
                                        if let Some(text) = part.text {
                                            delta.push_str(&text);
                                        }
                                    }
                                    if candidate.finish_reason.is_some() {
                                        finish_reason = Some("stop".to_string());
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
                            Some((Ok(ChatStreamChunk {
                                delta: String::new(),
                                finish_reason: None,
                                model: model_name.clone(),
                            }), (response, model_name, buffer)))
                        }
                    }
                    Ok(None) => None,
                    Err(e) => Some((Err(AstrBotError::Network(format!("Stream error: {}", e))), (response, model_name, buffer))),
                }
            },
        );

        Ok(Box::new(stream))
    }

    async fn embedding(&self, texts: Vec<String>, model: Option<String>) -> Result<Vec<Vec<f32>>> {
        let model = model.unwrap_or_else(|| "text-embedding-004".to_string());
        
        #[derive(Debug, Serialize)]
        struct EmbeddingRequest {
            content: GeminiContent,
        }

        #[derive(Debug, Deserialize)]
        struct EmbeddingResponse {
            embedding: Embedding,
        }

        #[derive(Debug, Deserialize)]
        struct Embedding {
            values: Vec<f32>,
        }

        let mut results = Vec::new();
        
        for text in texts {
            let request = EmbeddingRequest {
                content: GeminiContent {
                    role: "user".to_string(),
                    parts: vec![Part::text(text)],
                },
            };

            let url = format!(
                "{}/v1beta/models/{}:embedContent?key={}",
                self.base_url, model, self.api_key
            );

            let response = self.client
                .post(&url)
                .json(&request)
                .send()
                .await
                .map_err(|e| AstrBotError::Network(format!("Gemini embedding request failed: {}", e)))?;

            if !response.status().is_success() {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                return Err(AstrBotError::Provider {
                    provider: self.name.clone(),
                    message: format!("HTTP {}: {}", status, text),
                });
            }

            let resp: EmbeddingResponse = response.json().await
                .map_err(|e| AstrBotError::Serialization(format!("Failed to parse embedding response: {}", e)))?;

            results.push(resp.embedding.values);
        }

        Ok(results)
    }

    async fn model_info(&self, model: &str) -> Result<ModelInfo> {
        let context_length = if model.contains("1.5") {
            128000
        } else {
            32000
        };

        Ok(ModelInfo {
            name: model.to_string(),
            context_length,
            supports_streaming: true,
            supports_vision: model.contains("vision") || model.contains("1.5"),
            supports_function_calling: model.contains("1.5") || model.starts_with("gemini-1.0-pro"),
        })
    }

    async fn health_check(&self) -> Result<bool> {
        let url = format!(
            "{}/v1beta/models?key={}",
            self.base_url, self.api_key
        );
        match self.client.get(&url).send().await {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_messages() {
        let provider = GeminiProvider::new(
            "test".to_string(),
            "test-key".to_string(),
            "https://generativelanguage.googleapis.com".to_string(),
            "gemini-1.5-pro".to_string(),
        );

        let messages = vec![
            ChatMessage::user("Hello".to_string()),
            ChatMessage::assistant("Hi there".to_string()),
        ];

        let gemini_msgs = provider.convert_messages(messages);
        
        assert_eq!(gemini_msgs.len(), 2);
        assert_eq!(gemini_msgs[0].role, "user");
        assert_eq!(gemini_msgs[1].role, "model");
    }

    #[test]
    fn test_build_url() {
        let provider = GeminiProvider::new(
            "test".to_string(),
            "test-key".to_string(),
            "https://generativelanguage.googleapis.com".to_string(),
            "gemini-1.5-pro".to_string(),
        );

        let url = provider.build_url("gemini-1.5-pro", false);
        assert!(url.contains("generateContent"));
        assert!(url.contains("key=test-key"));

        let stream_url = provider.build_url("gemini-1.5-pro", true);
        assert!(stream_url.contains("streamGenerateContent"));
    }
}
