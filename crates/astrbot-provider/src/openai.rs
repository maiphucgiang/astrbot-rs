use astrbot_core::errors::{AstrBotError, Result};
use astrbot_core::provider::{
    ChatConfig, ChatMessage, ChatResponse, ChatStreamChunk, ModelInfo, Provider, TokenUsage,
};
use astrbot_core::tools::ToolCall;
use async_trait::async_trait;
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    messages: Vec<OpenAIRequestMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Value>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct OpenAIRequestMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
    model: String,
    usage: OpenAIUsage,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    message: OpenAIResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAIToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAIToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OpenAIToolFunction,
}

#[derive(Debug, Deserialize)]
struct OpenAIToolFunction {
    name: String,
    arguments: String,
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
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAIStreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamToolCall {
    index: usize,
    id: Option<String>,
    #[serde(rename = "type")]
    call_type: Option<String>,
    function: Option<OpenAIStreamToolFunction>,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamToolFunction {
    name: Option<String>,
    arguments: Option<String>,
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

        // Convert ChatMessage to OpenAI request messages (preserve tool_calls / tool_call_id)
        let req_messages: Vec<OpenAIRequestMessage> = messages
            .into_iter()
            .map(|m| {
                let tool_calls = m.tool_calls.map(|calls| {
                    calls
                        .into_iter()
                        .map(|tc| {
                            serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments.to_string(),
                                }
                            })
                        })
                        .collect::<Vec<Value>>()
                });
                OpenAIRequestMessage {
                    role: m.role,
                    content: m.content,
                    name: m.name,
                    tool_calls,
                    tool_call_id: m.tool_call_id,
                }
            })
            .collect();

        // Extract tools from ChatConfig.extra if present
        let tools = config.extra.get("tools").cloned();

        let request = OpenAIRequest {
            model,
            messages: req_messages,
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            top_p: config.top_p,
            tools,
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

        let choice = resp.choices.into_iter().next();
        let (content, tool_calls) = match choice {
            Some(c) => {
                let msg = c.message;
                let content = msg.content.unwrap_or_default();
                let tool_calls = msg.tool_calls.map(|tcs| {
                    tcs.into_iter()
                        .map(|tc| {
                            let args =
                                serde_json::from_str(&tc.function.arguments).unwrap_or(Value::Null);
                            ToolCall {
                                id: tc.id,
                                name: tc.function.name,
                                arguments: args,
                            }
                        })
                        .collect::<Vec<ToolCall>>()
                });
                (content, tool_calls)
            }
            None => (String::new(), None),
        };

        Ok(ChatResponse {
            content,
            model: resp.model,
            usage: Some(TokenUsage {
                prompt_tokens: resp.usage.prompt_tokens,
                completion_tokens: resp.usage.completion_tokens,
                total_tokens: resp.usage.total_tokens,
            }),
            reasoning: None,
            tool_calls,
        })
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        config: ChatConfig,
    ) -> Result<Box<dyn Stream<Item = Result<ChatStreamChunk>> + Send>> {
        let model = config.model.unwrap_or_else(|| self.default_model.clone());

        let req_messages: Vec<OpenAIRequestMessage> = messages
            .into_iter()
            .map(|m| {
                let tool_calls = m.tool_calls.map(|calls| {
                    calls
                        .into_iter()
                        .map(|tc| {
                            serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments.to_string(),
                                }
                            })
                        })
                        .collect::<Vec<Value>>()
                });
                OpenAIRequestMessage {
                    role: m.role,
                    content: m.content,
                    name: m.name,
                    tool_calls,
                    tool_call_id: m.tool_call_id,
                }
            })
            .collect();

        let tools = config.extra.get("tools").cloned();

        let request = OpenAIRequest {
            model,
            messages: req_messages,
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            top_p: config.top_p,
            tools,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_provider() -> OpenAiProvider {
        OpenAiProvider::new(
            "test-openai".to_string(),
            "sk-test".to_string(),
            "https://api.openai.com".to_string(),
            "gpt-4o".to_string(),
        )
    }

    #[test]
    fn test_openai_provider_new() {
        let provider = test_provider();
        assert_eq!(provider.id(), "test-openai");
        assert_eq!(provider.name(), "test-openai");
    }

    #[test]
    fn test_openai_auth_header() {
        let provider = test_provider();
        assert_eq!(provider.auth_header(), "Bearer sk-test");
    }

    #[test]
    fn test_openai_models_list() {
        let provider = test_provider();
        let models = tokio_test::block_on(provider.models()).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0], "gpt-4o");
    }

    #[test]
    fn test_openai_model_info_gpt4o() {
        let provider = test_provider();
        let info = tokio_test::block_on(provider.model_info("gpt-4o")).unwrap();
        assert_eq!(info.name, "gpt-4o");
        assert_eq!(info.context_length, 128000);
        assert!(info.supports_streaming);
        assert!(info.supports_vision);
        assert!(info.supports_function_calling);
    }

    #[test]
    fn test_openai_model_info_gpt35() {
        let provider = test_provider();
        let info = tokio_test::block_on(provider.model_info("gpt-3.5-turbo")).unwrap();
        assert_eq!(info.name, "gpt-3.5-turbo");
        assert_eq!(info.context_length, 16385);
        assert!(info.supports_function_calling);
    }

    #[test]
    fn test_openai_model_info_unknown() {
        let provider = test_provider();
        let info = tokio_test::block_on(provider.model_info("unknown-model")).unwrap();
        assert_eq!(info.context_length, 4096);
        assert!(!info.supports_vision);
    }

    #[test]
    fn test_openai_request_message_serialization() {
        let msg = OpenAIRequestMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"content\":\"Hello\""));
        assert!(!json.contains("tool_calls"));
    }

    #[test]
    fn test_openai_request_message_with_tool_calls() {
        let msg = OpenAIRequestMessage {
            role: "assistant".to_string(),
            content: "".to_string(),
            name: None,
            tool_calls: Some(vec![serde_json::json!({
                "id": "call_1",
                "type": "function",
                "function": {
                    "name": "echo",
                    "arguments": "{\"text\":\"hi\"}"
                }
            })]),
            tool_call_id: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("call_1"));
        assert!(json.contains("echo"));
    }

    #[test]
    fn test_openai_response_message_deserialization() {
        let json = r#"{"content":"Hello","tool_calls":null}"#;
        let msg: OpenAIResponseMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.content, Some("Hello".to_string()));
        assert!(msg.tool_calls.is_none());
    }

    #[test]
    fn test_openai_response_message_with_tool_calls() {
        let json = r#"{
            "content":null,
            "tool_calls":[
                {
                    "id":"call_abc123",
                    "type":"function",
                    "function":{"name":"get_weather","arguments":"{\"location\":\"NYC\"}"}
                }
            ]
        }"#;
        let msg: OpenAIResponseMessage = serde_json::from_str(json).unwrap();
        assert!(msg.content.is_none());
        let tcs = msg.tool_calls.unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].id, "call_abc123");
        assert_eq!(tcs[0].function.name, "get_weather");
    }

    #[test]
    fn test_openai_stream_delta_deserialization() {
        let json = r#"{"content":"hi","tool_calls":null}"#;
        let delta: OpenAIStreamDelta = serde_json::from_str(json).unwrap();
        assert_eq!(delta.content, Some("hi".to_string()));
    }

    #[test]
    fn test_openai_request_tools_serialization() {
        let req = OpenAIRequest {
            model: "gpt-4o".to_string(),
            messages: vec![OpenAIRequestMessage {
                role: "user".to_string(),
                content: "What's the weather?".to_string(),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            }],
            temperature: Some(0.7),
            max_tokens: Some(1024),
            top_p: Some(1.0),
            tools: Some(serde_json::json!([
                {
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "description": "Get weather",
                        "parameters": {"type": "object", "properties": {}}
                    }
                }
            ])),
            stream: false,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("get_weather"));
        assert!(json.contains("\"stream\":false"));
    }

    #[test]
    fn test_openai_request_skips_none_fields() {
        let req = OpenAIRequest {
            model: "gpt-4o".to_string(),
            messages: vec![OpenAIRequestMessage {
                role: "user".to_string(),
                content: "Hi".to_string(),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            }],
            temperature: None,
            max_tokens: None,
            top_p: None,
            tools: None,
            stream: false,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("temperature"));
        assert!(!json.contains("max_tokens"));
        assert!(!json.contains("tools"));
    }

    #[test]
    fn test_openai_health_check_unavailable() {
        let provider = OpenAiProvider::new(
            "test".to_string(),
            "sk-invalid".to_string(),
            "https://invalid.openai.example.com".to_string(),
            "gpt-4o".to_string(),
        );
        let result = tokio_test::block_on(provider.health_check());
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }
}
