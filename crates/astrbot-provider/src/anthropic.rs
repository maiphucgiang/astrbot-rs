use async_trait::async_trait;
use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::provider::{
    ChatMessage, ChatConfig, ChatResponse, ChatStreamChunk, ModelInfo, TokenUsage,
    Provider,
};
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use tracing::{debug, error};

/// Anthropic Claude provider
pub struct AnthropicProvider {
    id: String,
    name: String,
    api_key: String,
    base_url: String,
    default_model: String,
    client: reqwest::Client,
}

impl AnthropicProvider {
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
        self.api_key.clone()
    }

    fn convert_messages(messages: Vec<ChatMessage>) -> (Option<String>, Vec<AnthropicMessage>) {
        let mut system = None;
        let mut anthropic_msgs = Vec::new();

        for msg in messages {
            match msg.role.as_str() {
                "system" => {
                    system = Some(msg.content);
                }
                "user" => {
                    anthropic_msgs.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: vec![ContentBlock::text(msg.content)],
                    });
                }
                "assistant" => {
                    anthropic_msgs.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: vec![ContentBlock::text(msg.content)],
                    });
                }
                _ => {}
            }
        }

        (system, anthropic_msgs)
    }
}

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    stream: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<ContentBlock>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

impl ContentBlock {
    fn text(content: String) -> Self {
        Self {
            block_type: "text".to_string(),
            text: Some(content),
        }
    }
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    id: String,
    #[serde(rename = "type")]
    response_type: String,
    role: String,
    content: Vec<ContentBlock>,
    model: String,
    usage: AnthropicUsage,
    #[serde(rename = "stop_reason")]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    #[serde(rename = "input_tokens")]
    input_tokens: u32,
    #[serde(rename = "output_tokens")]
    output_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct AnthropicStreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(flatten)]
    data: serde_json::Value,
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> &str { &self.id }
    fn name(&self) -> &str { &self.name }

    async fn models(&self) -> Result<Vec<String>> {
        Ok(vec![
            "claude-3-opus-20240229".to_string(),
            "claude-3-sonnet-20240229".to_string(),
            "claude-3-haiku-20240307".to_string(),
            "claude-3-5-sonnet-20240620".to_string(),
            self.default_model.clone(),
        ])
    }

    async fn chat(&self, messages: Vec<ChatMessage>, config: ChatConfig) -> Result<ChatResponse> {
        let model = config.model.unwrap_or_else(|| self.default_model.clone());
        let (system, anthropic_msgs) = Self::convert_messages(messages);

        let request = AnthropicRequest {
            model,
            messages: anthropic_msgs,
            system,
            max_tokens: config.max_tokens.or(Some(4096)),
            temperature: config.temperature,
            top_p: config.top_p,
            stream: false,
        };

        let url = format!("{}/v1/messages", self.base_url);
        let response = self.client
            .post(&url)
            .header("x-api-key", self.auth_header())
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Anthropic request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Provider {
                provider: self.name.clone(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        let resp: AnthropicResponse = response.json().await
            .map_err(|e| AstrBotError::Serialization(format!("Failed to parse Anthropic response: {}", e)))?;

        let content = resp.content.into_iter()
            .filter_map(|block| block.text)
            .collect::<Vec<_>>()
            .join("");

        Ok(ChatResponse {
            content,
            model: resp.model,
            usage: Some(TokenUsage {
                prompt_tokens: resp.usage.input_tokens,
                completion_tokens: resp.usage.output_tokens,
                total_tokens: resp.usage.input_tokens + resp.usage.output_tokens,
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
        let (system, anthropic_msgs) = Self::convert_messages(messages);

        let request = AnthropicRequest {
            model,
            messages: anthropic_msgs,
            system,
            max_tokens: config.max_tokens.or(Some(4096)),
            temperature: config.temperature,
            top_p: config.top_p,
            stream: true,
        };

        let url = format!("{}/v1/messages", self.base_url);
        let response = self.client
            .post(&url)
            .header("x-api-key", self.auth_header())
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Anthropic stream request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Provider {
                provider: self.name.clone(),
                message: format!("HTTP {}: {}", status, text),
            });
        }

        let model_name = self.default_model.clone();
        
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
                                if let Ok(event) = serde_json::from_str::<AnthropicStreamEvent>(data) {
                                    match event.event_type.as_str() {
                                        "content_block_delta" => {
                                            if let Some(delta_obj) = event.data.get("delta") {
                                                if let Some(text) = delta_obj.get("text").and_then(|t| t.as_str()) {
                                                    delta.push_str(text);
                                                }
                                            }
                                        }
                                        "message_stop" => {
                                            finish_reason = Some("stop".to_string());
                                        }
                                        _ => {}
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

    async fn embedding(&self, _texts: Vec<String>, _model: Option<String>) -> Result<Vec<Vec<f32>>> {
        Err(AstrBotError::Provider {
            provider: self.name.clone(),
            message: "Anthropic does not support embeddings".to_string(),
        })
    }

    async fn model_info(&self, model: &str) -> Result<ModelInfo> {
        let context_length = if model.contains("opus") {
            200000
        } else if model.contains("sonnet") {
            200000
        } else if model.contains("haiku") {
            200000
        } else {
            200000
        };

        Ok(ModelInfo {
            name: model.to_string(),
            context_length,
            supports_streaming: true,
            supports_vision: model.contains("vision") || model.starts_with("claude-3"),
            supports_function_calling: model.starts_with("claude-3"),
        })
    }

    async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/v1/models", self.base_url);
        match self.client
            .get(&url)
            .header("x-api-key", self.auth_header())
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
        {
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
        let messages = vec![
            ChatMessage::system("You are helpful".to_string()),
            ChatMessage::user("Hello".to_string()),
            ChatMessage::assistant("Hi there".to_string()),
        ];

        let (system, anthropic_msgs) = AnthropicProvider::convert_messages(messages);
        
        assert_eq!(system, Some("You are helpful".to_string()));
        assert_eq!(anthropic_msgs.len(), 2);
        assert_eq!(anthropic_msgs[0].role, "user");
        assert_eq!(anthropic_msgs[1].role, "assistant");
    }
}
