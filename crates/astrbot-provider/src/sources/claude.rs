use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde_json::{json, Value};

use crate::{ChatMessage, ChatOptions, ChatProvider, ProviderConfig, ProviderError};

pub struct ClaudeProvider {
    client: reqwest::Client,
    config: ProviderConfig,
}

impl ClaudeProvider {
    pub fn new(config: ProviderConfig) -> Self {
        let mut config = config;
        if config.base_url.is_empty() {
            config.base_url = "https://api.anthropic.com".to_string();
        }
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }

    fn build_request(&self, body: Value) -> reqwest::RequestBuilder {
        let url = format!("{}/v1/messages", self.config.base_url.trim_end_matches('/'));
        self.client
            .post(&url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
    }

    fn build_request_body(&self, messages: Vec<ChatMessage>, options: &ChatOptions) -> Value {
        let mut system_parts = Vec::new();
        let anthropic_messages: Vec<Value> = messages
            .into_iter()
            .filter_map(|msg| match msg.role.as_str() {
                "system" => {
                    system_parts.push(msg.content);
                    None
                }
                "user" | "assistant" => Some(json!({
                    "role": msg.role,
                    "content": msg.content,
                })),
                _ => Some(json!({
                    "role": "user",
                    "content": msg.content,
                })),
            })
            .collect();

        let mut body = json!({
            "model": options.model.as_ref().unwrap_or(&self.config.model),
            "max_tokens": options.max_tokens.unwrap_or(1024),
            "messages": anthropic_messages,
        });

        if !system_parts.is_empty() {
            body["system"] = json!(system_parts.join("\n\n"));
        }
        if let Some(t) = options.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(p) = options.top_p {
            body["top_p"] = json!(p);
        }

        body
    }
}

#[async_trait]
impl ChatProvider for ClaudeProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        options: ChatOptions,
    ) -> Result<String, ProviderError> {
        let body = self.build_request_body(messages, &options);
        let response = self.build_request(body).send().await?;
        let status = response.status();

        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(ProviderError::Api {
                status: status.as_u16(),
                message: text,
            });
        }

        let json: Value = response.json().await?;
        let content = json["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();

        Ok(content)
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    async fn stream_chat(
        &self,
        messages: Vec<ChatMessage>,
        options: ChatOptions,
    ) -> Result<Box<dyn Stream<Item = Result<String, ProviderError>> + Send>, ProviderError> {
        let mut body = self.build_request_body(messages, &options);
        body["stream"] = json!(true);

        let response = self.build_request(body).send().await?;
        let status = response.status();

        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(ProviderError::Api {
                status: status.as_u16(),
                message: text,
            });
        }

        let stream = response.bytes_stream();
        let mapped = stream.map(|result| match result {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                let mut delta = String::new();
                for line in text.lines() {
                    let line = line.strip_prefix("data: ").unwrap_or(line);
                    if line.is_empty() || line == "[DONE]" {
                        continue;
                    }
                    if let Ok(json) = serde_json::from_str::<Value>(line) {
                        if json.get("type").and_then(|t| t.as_str()) == Some("content_block_delta")
                        {
                            if let Some(text) = json
                                .get("delta")
                                .and_then(|d| d.get("text"))
                                .and_then(|t| t.as_str())
                            {
                                delta.push_str(text);
                            }
                        }
                    }
                }
                Ok(delta)
            }
            Err(e) => Err(ProviderError::Http(e)),
        });

        Ok(Box::new(mapped))
    }

    fn list_models(&self) -> Vec<String> {
        vec![
            "claude-3-5-sonnet-20241022".to_string(),
            "claude-3-opus-20240229".to_string(),
            "claude-3-sonnet-20240229".to_string(),
            "claude-3-haiku-20240307".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
            self.config.model.clone(),
        ]
    }

    fn is_available(&self) -> bool {
        !self.config.api_key.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ProviderConfig {
        ProviderConfig {
            name: "Claude".to_string(),
            base_url: "".to_string(),
            api_key: "test-key".to_string(),
            model: "claude-3-5-sonnet-20241022".to_string(),
            extra_headers: None,
        }
    }

    #[test]
    fn test_claude_provider_new() {
        let provider = ClaudeProvider::new(test_config());
        assert_eq!(provider.name(), "Claude");
        assert!(provider.is_available());
    }

    #[test]
    fn test_claude_default_base_url() {
        let provider = ClaudeProvider::new(test_config());
        assert_eq!(provider.config.base_url, "https://api.anthropic.com");
    }

    #[test]
    fn test_claude_system_message_extraction() {
        let provider = ClaudeProvider::new(test_config());
        let messages = vec![
            ChatMessage::system("You are Claude"),
            ChatMessage::user("Hello"),
        ];
        let body = provider.build_request_body(messages, &ChatOptions::default());

        assert_eq!(body.get("system").unwrap().as_str(), Some("You are Claude"));
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"].as_str(), Some("user"));
    }

    #[test]
    fn test_claude_multiple_system_messages() {
        let provider = ClaudeProvider::new(test_config());
        let messages = vec![
            ChatMessage::system("First system"),
            ChatMessage::user("Hi"),
            ChatMessage::system("Second system"),
            ChatMessage::assistant("Hey"),
        ];
        let body = provider.build_request_body(messages, &ChatOptions::default());

        assert_eq!(
            body.get("system").unwrap().as_str(),
            Some("First system\n\nSecond system")
        );
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn test_claude_unknown_role_fallback() {
        let provider = ClaudeProvider::new(test_config());
        let messages = vec![ChatMessage {
            role: "unknown".to_string(),
            content: "test".to_string(),
        }];
        let body = provider.build_request_body(messages, &ChatOptions::default());
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"].as_str(), Some("user"));
    }

    #[test]
    fn test_claude_chat_options_applied() {
        let provider = ClaudeProvider::new(test_config());
        let messages = vec![ChatMessage::user("Hello")];
        let options = ChatOptions {
            temperature: Some(0.5),
            top_p: Some(0.9),
            max_tokens: Some(2048),
            model: Some("claude-3-opus-20240229".to_string()),
        };
        let body = provider.build_request_body(messages, &options);

        assert_eq!(body["model"].as_str(), Some("claude-3-opus-20240229"));
        assert_eq!(body["max_tokens"].as_u64(), Some(2048));
        let temp = body["temperature"].as_f64().unwrap();
        assert!(
            (temp - 0.5).abs() < 0.01,
            "temperature={}, expected 0.5",
            temp
        );
        let top_p = body["top_p"].as_f64().unwrap();
        assert!((top_p - 0.9).abs() < 0.01, "top_p={}, expected 0.9", top_p);
    }

    #[test]
    fn test_claude_list_models() {
        let provider = ClaudeProvider::new(test_config());
        let models = provider.list_models();
        assert!(models.contains(&"claude-3-5-sonnet-20241022".to_string()));
        assert!(models.contains(&"claude-3-opus-20240229".to_string()));
    }
}
