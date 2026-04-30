use crate::errors::{AstrBotError, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::info;

/// Local LLM client powered by Ollama.
/// Connects to a local (or remote) Ollama instance at `base_url`.
#[derive(Debug, Clone)]
pub struct OllamaClient {
    pub base_url: String,
    pub model: String,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub message: ChatMessage,
    pub done: bool,
}

impl OllamaClient {
    pub fn new(base_url: String, model: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            client: reqwest::Client::new(),
        }
    }

    /// Single-turn chat (non-streaming).
    pub async fn chat(&self,
        messages: Vec<ChatMessage>,
        temperature: Option<f32>,
    ) -> Result<String> {
        info!("[OllamaClient] chat — model={}", self.model);
        let url = format!("{}/api/chat", self.base_url);
        let mut body = json!({
            "model": self.model,
            "messages": serde_json::to_value(&messages)
                .map_err(|e| AstrBotError::Serialization(format!("Ollama body serialize: {}", e)))?,
            "stream": false,
        });
        if let Some(t) = temperature {
            body["options"] = json!({ "temperature": t });
        }
        let resp = self.client.post(&url).json(&body).send().await
            .map_err(|e| AstrBotError::Network(format!("Ollama chat request: {}", e)))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Network(format!(
                "Ollama chat HTTP {}: {}",
                status, text
            )));
        }
        let json: serde_json::Value = resp.json().await
            .map_err(|e| AstrBotError::Serialization(format!("Ollama JSON parse: {}", e)))?;
        let content = json["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        Ok(content)
    }

    /// Pull a model from Ollama registry (skeleton).
    pub async fn pull_model(&self, _model: &str) -> Result<()> {
        info!("[OllamaClient] pull_model — skeleton");
        Ok(())
    }

    /// Health check: list local models.
    pub async fn health_check(&self) -> Result<()> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Ollama health: {}", e)))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(AstrBotError::Network(format!(
                "Ollama health check failed: HTTP {}",
                resp.status()
            )))
        }
    }

    /// List locally available models.
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Ollama list models: {}", e)))?;
        let json: serde_json::Value = resp.json().await
            .map_err(|e| AstrBotError::Serialization(format!("Ollama list models JSON: {}", e)))?;
        let models = json["models"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m["name"].as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(models)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn run_mock_ollama_server(port: u16, response_body: String) {
        let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
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
        let port = 29870u16;
        let body = r#"{"message":{"role":"assistant","content":"Hello!"},"done":true}"#;
        let server = tokio::spawn(run_mock_ollama_server(port, body.to_string()));
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let client = OllamaClient::new(format!("http://127.0.0.1:{}", port), "llama3.2".to_string());
        let messages = vec![ChatMessage { role: "user".into(), content: "Say hi".into() }];
        let result = client.chat(messages, None).await.unwrap();
        assert_eq!(result, "Hello!");
        let _ = server.await;
    }

    #[tokio::test]
    async fn test_ollama_chat_with_temperature() {
        let port = 29869u16;
        let body = r#"{"message":{"role":"assistant","content":"Yo"},"done":true}"#;
        let server = tokio::spawn(async move {
            let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(
                req.contains("\"temperature\":0.5") || req.contains("\"options\""),
                "request should contain temperature options"
            );
            let http_response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(http_response.as_bytes()).await.unwrap();
        });
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let client = OllamaClient::new(format!("http://127.0.0.1:{}", port), "qwen2.5".to_string());
        let messages = vec![ChatMessage { role: "user".into(), content: "hi".into() }];
        let result = client.chat(messages, Some(0.5)).await.unwrap();
        assert_eq!(result, "Yo");
        let _ = server.await;
    }

    #[tokio::test]
    async fn test_ollama_health_check() {
        let port = 29868u16;
        let body = r#"{"models":[{"name":"llama3.2"},{"name":"qwen2.5"}]}"#;
        let server = tokio::spawn(run_mock_ollama_server(port, body.to_string()));
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let client = OllamaClient::new(format!("http://127.0.0.1:{}", port), "llama3.2".to_string());
        assert!(client.health_check().await.is_ok());
        let _ = server.await;
    }

    #[tokio::test]
    async fn test_ollama_list_models() {
        let port = 29867u16;
        let body = r#"{"models":[{"name":"llama3.2"},{"name":"qwen2.5"}]}"#;
        let server = tokio::spawn(run_mock_ollama_server(port, body.to_string()));
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let client = OllamaClient::new(format!("http://127.0.0.1:{}", port), "llama3.2".to_string());
        let models = client.list_models().await.unwrap();
        assert_eq!(models, vec!["llama3.2".to_string(), "qwen2.5".to_string()]);
        let _ = server.await;
    }

    #[tokio::test]
    async fn test_ollama_http_error() {
        let port = 29866u16;
        let server = tokio::spawn(async move {
            let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _n = stream.read(&mut buf).await.unwrap_or(0);
            let resp = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 20\r\n\r\n{\"error\":\"load failed\"}";
            stream.write_all(resp.as_bytes()).await.unwrap();
        });
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let client = OllamaClient::new(format!("http://127.0.0.1:{}", port), "bad-model".to_string());
        let result = client.chat(vec![ChatMessage { role: "user".into(), content: "test".into() }], None).await;
        assert!(result.is_err());
        let _ = server.await;
    }

    #[tokio::test]
    async fn test_ollama_client_clone() {
        let client = OllamaClient::new("http://localhost:11434".into(), "llama3.2".into());
        let _cloned = client.clone();
        assert_eq!(client.base_url, "http://localhost:11434");
        assert_eq!(client.model, "llama3.2");
    }
}
