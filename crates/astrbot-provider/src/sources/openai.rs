use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde_json::{json, Value};

use crate::{ChatMessage, ChatOptions, ChatProvider, ProviderConfig, ProviderError};

/// OpenAI native provider — full model list + function-calling ready base
pub struct OpenAiChatProvider {
    client: reqwest::Client,
    config: ProviderConfig,
}

impl OpenAiChatProvider {
    pub fn new(config: ProviderConfig) -> Self {
        let mut config = config;
        if config.base_url.is_empty() {
            config.base_url = "https://api.openai.com/v1".to_string();
        }
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }

    pub fn create(api_key: String, model: String) -> Self {
        Self::new(ProviderConfig {
            name: "OpenAI".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_key,
            model,
            extra_headers: None,
        })
    }

    fn build_request(&self, body: Value) -> reqwest::RequestBuilder {
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
    }

    fn build_request_body(&self, messages: Vec<ChatMessage>, options: &ChatOptions) -> Value {
        let openai_messages: Vec<Value> = messages
            .into_iter()
            .map(|msg| {
                json!({
                    "role": msg.role,
                    "content": msg.content,
                })
            })
            .collect();

        let mut body = json!({
            "model": options.model.as_ref().unwrap_or(&self.config.model),
            "messages": openai_messages,
        });

        if let Some(t) = options.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(p) = options.top_p {
            body["top_p"] = json!(p);
        }
        if let Some(m) = options.max_tokens {
            body["max_tokens"] = json!(m);
        }

        body
    }
}

#[async_trait]
impl ChatProvider for OpenAiChatProvider {
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
        let content = json["choices"][0]["message"]["content"]
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
                        if let Some(content) = json["choices"][0]["delta"]["content"].as_str() {
                            delta.push_str(content);
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
            "gpt-4o".to_string(),
            "gpt-4o-mini".to_string(),
            "gpt-4-turbo".to_string(),
            "gpt-4".to_string(),
            "gpt-3.5-turbo".to_string(),
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
            name: "OpenAI".to_string(),
            base_url: "".to_string(),
            api_key: "sk-test".to_string(),
            model: "gpt-4o".to_string(),
            extra_headers: None,
        }
    }

    #[test]
    fn test_openai_provider_new() {
        let provider = OpenAiChatProvider::new(test_config());
        assert_eq!(provider.name(), "OpenAI");
        assert!(provider.is_available());
    }

    #[test]
    fn test_openai_default_base_url() {
        let provider = OpenAiChatProvider::new(test_config());
        assert_eq!(provider.config.base_url, "https://api.openai.com/v1");
    }

    #[test]
    fn test_openai_list_models() {
        let provider = OpenAiChatProvider::new(test_config());
        let models = provider.list_models();
        assert!(models.contains(&"gpt-4o".to_string()));
        assert!(models.contains(&"gpt-4o-mini".to_string()));
        assert!(models.contains(&"gpt-3.5-turbo".to_string()));
    }

    #[test]
    fn test_openai_chat_options_applied() {
        let provider = OpenAiChatProvider::new(test_config());
        let messages = vec![ChatMessage::user("Hello")];
        let options = ChatOptions {
            temperature: Some(0.5),
            top_p: Some(0.9),
            max_tokens: Some(2048),
            model: Some("gpt-4o-mini".to_string()),
        };
        let body = provider.build_request_body(messages, &options);

        assert_eq!(body["model"].as_str(), Some("gpt-4o-mini"));
        assert_eq!(body["max_tokens"].as_u64(), Some(2048));
        let temp = body["temperature"].as_f64().unwrap();
        assert!(
            (temp - 0.5).abs() < 0.01,
            "temperature={}, expected 0.5",
            temp
        );
    }

    #[test]
    fn test_openai_create() {
        let provider = OpenAiChatProvider::create("sk-test".to_string(), "gpt-4o".to_string());
        assert_eq!(provider.name(), "OpenAI");
        assert_eq!(provider.config.model, "gpt-4o");
    }
}
