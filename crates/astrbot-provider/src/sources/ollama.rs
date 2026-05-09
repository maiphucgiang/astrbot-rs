use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde_json::{json, Value};

use crate::{ChatMessage, ChatOptions, ChatProvider, ProviderConfig, ProviderError};

/// Ollama local LLM provider.
///
/// Expects `base_url` to point at the Ollama server (default `http://localhost:11434`).
/// Uses the `/api/generate` endpoint (non-streaming) and `/api/generate` with `stream: true`
/// for streaming.
pub struct OllamaProvider {
    client: reqwest::Client,
    config: ProviderConfig,
}

impl OllamaProvider {
    pub fn new(config: ProviderConfig) -> Self {
        let mut config = config;
        if config.base_url.is_empty() {
            config.base_url = "http://localhost:11434".to_string();
        }
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }

    fn build_prompt(&self, messages: &[ChatMessage]) -> String {
        let mut parts = Vec::new();
        for msg in messages {
            let label = match msg.role.as_str() {
                "system" => "System",
                "user" => "User",
                "assistant" => "Assistant",
                _ => &msg.role,
            };
            parts.push(format!("{}: {}", label, msg.content));
        }
        parts.join("\n\n")
    }

    fn build_request_body(&self, prompt: &str, options: &ChatOptions, stream: bool) -> Value {
        let mut body = json!({
            "model": options.model.as_ref().unwrap_or(&self.config.model),
            "prompt": prompt,
            "stream": stream,
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

    async fn generate(&self, prompt: &str, options: &ChatOptions) -> Result<String, ProviderError> {
        let url = format!(
            "{}/api/generate",
            self.config.base_url.trim_end_matches('/')
        );
        let body = self.build_request_body(prompt, options, false);
        let resp = self.client.post(&url).json(&body).send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(ProviderError::Api {
                status: status.as_u16(),
                message: text,
            });
        }
        let parsed: Value = serde_json::from_str(&text)?;
        let content = parsed["response"].as_str().unwrap_or("").to_string();
        Ok(content)
    }
}

#[async_trait]
impl ChatProvider for OllamaProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        options: ChatOptions,
    ) -> Result<String, ProviderError> {
        let prompt = self.build_prompt(&messages);
        self.generate(&prompt, &options).await
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    async fn stream_chat(
        &self,
        messages: Vec<ChatMessage>,
        options: ChatOptions,
    ) -> Result<Box<dyn Stream<Item = Result<String, ProviderError>> + Send>, ProviderError> {
        let prompt = self.build_prompt(&messages);
        let url = format!(
            "{}/api/generate",
            self.config.base_url.trim_end_matches('/')
        );
        let body = self.build_request_body(&prompt, &options, true);
        let resp = self.client.post(&url).json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Api {
                status: status.as_u16(),
                message: text,
            });
        }

        let stream = resp.bytes_stream();
        let mapped = stream.map(|result| match result {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                let mut content = String::new();
                for line in text.lines() {
                    if let Some(data) = line.strip_prefix("data: ") {
                        if data == "[DONE]" {
                            continue;
                        }
                        if let Ok(json) = serde_json::from_str::<Value>(data) {
                            if let Some(c) = json["response"].as_str() {
                                content.push_str(c);
                            }
                        }
                    }
                }
                Ok(content)
            }
            Err(e) => Err(ProviderError::Http(e)),
        });
        Ok(Box::new(mapped))
    }

    fn list_models(&self) -> Vec<String> {
        vec![self.config.model.clone()]
    }

    fn is_available(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn run_mock_ollama_server(port: u16, response_body: String) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .unwrap();
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let _req = String::from_utf8_lossy(&buf[..n]);
        let http_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(http_response.as_bytes()).await.unwrap();
    }

    #[tokio::test]
    async fn test_ollama_chat_basic() {
        let port = 29880u16;
        let body = r#"{"response":"Hello from Ollama","done":true}"#;
        let server = tokio::spawn(run_mock_ollama_server(port, body.to_string()));
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let provider = OllamaProvider::new(ProviderConfig {
            name: "ollama".to_string(),
            base_url: format!("http://127.0.0.1:{}", port),
            api_key: "".to_string(),
            model: "llama3.2".to_string(),
            extra_headers: None,
        });
        let messages = vec![ChatMessage::user("Say hello")];
        let result = provider
            .chat(messages, ChatOptions::default())
            .await
            .unwrap();
        assert_eq!(result, "Hello from Ollama");
        let _ = server.await;
    }

    #[tokio::test]
    async fn test_ollama_chat_with_system() {
        let port = 29879u16;
        let body = r#"{"response":"Paris","done":true}"#;
        let server = tokio::spawn(run_mock_ollama_server(port, body.to_string()));
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let provider = OllamaProvider::new(ProviderConfig {
            name: "ollama".to_string(),
            base_url: format!("http://127.0.0.1:{}", port),
            api_key: "".to_string(),
            model: "qwen2.5".to_string(),
            extra_headers: None,
        });
        let messages = vec![
            ChatMessage::system("You are a helpful assistant."),
            ChatMessage::user("Capital of France?"),
        ];
        let result = provider
            .chat(messages, ChatOptions::default())
            .await
            .unwrap();
        assert_eq!(result, "Paris");
        let _ = server.await;
    }

    #[tokio::test]
    async fn test_ollama_http_error() {
        let port = 29878u16;
        let server = tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
                .await
                .unwrap();
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _n = stream.read(&mut buf).await.unwrap_or(0);
            let resp = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 26\r\n\r\n{\"error\":\"model not found\"}";
            stream.write_all(resp.as_bytes()).await.unwrap();
        });
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let provider = OllamaProvider::new(ProviderConfig {
            name: "ollama".to_string(),
            base_url: format!("http://127.0.0.1:{}", port),
            api_key: "".to_string(),
            model: "nonexistent".to_string(),
            extra_headers: None,
        });
        let result = provider
            .chat(vec![ChatMessage::user("test")], ChatOptions::default())
            .await;
        assert!(result.is_err());
        let _ = server.await;
    }

    #[tokio::test]
    async fn test_ollama_list_models() {
        let provider = OllamaProvider::new(ProviderConfig {
            name: "ollama".to_string(),
            base_url: "".to_string(),
            api_key: "".to_string(),
            model: "llama3.2".to_string(),
            extra_headers: None,
        });
        let models = provider.list_models();
        assert_eq!(models, vec!["llama3.2".to_string()]);
    }

    #[tokio::test]
    async fn test_ollama_prompt_building() {
        let provider = OllamaProvider::new(ProviderConfig {
            name: "ollama".to_string(),
            base_url: "".to_string(),
            api_key: "".to_string(),
            model: "test".to_string(),
            extra_headers: None,
        });
        let messages = vec![
            ChatMessage::system("Be concise."),
            ChatMessage::user("2+2=?"),
            ChatMessage::assistant("4"),
            ChatMessage::user("3+3=?"),
        ];
        let prompt = provider.build_prompt(&messages);
        assert!(prompt.contains("System: Be concise."));
        assert!(prompt.contains("User: 2+2=?"));
        assert!(prompt.contains("Assistant: 4"));
        assert!(prompt.contains("User: 3+3=?"));
    }

    #[tokio::test]
    async fn test_ollama_temperature_applied() {
        let port = 29877u16;
        let body = r#"{"response":"ok","done":true}"#;
        let server = tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
                .await
                .unwrap();
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(
                req.contains("\"temperature\":0.3"),
                "request should contain temperature=0.3, got: {}",
                req
            );
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).await.unwrap();
        });
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let provider = OllamaProvider::new(ProviderConfig {
            name: "ollama".to_string(),
            base_url: format!("http://127.0.0.1:{}", port),
            api_key: "".to_string(),
            model: "llama3.2".to_string(),
            extra_headers: None,
        });
        let opts = ChatOptions {
            temperature: Some(0.3),
            top_p: None,
            max_tokens: None,
            model: None,
        };
        let _ = provider
            .chat(vec![ChatMessage::user("hi")], opts)
            .await
            .unwrap();
        let _ = server.await;
    }
}
