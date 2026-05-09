use crate::errors::{AstrBotError, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub dimensions: Option<usize>,
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>>;
    async fn health_check(&self) -> Result<bool>;
}

pub struct OpenAiEmbeddingProvider {
    id: String,
    name: String,
    config: EmbeddingConfig,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingItem {
    embedding: Vec<f32>,
    index: usize,
}
#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingResponse {
    data: Vec<OpenAiEmbeddingItem>,
    model: String,
}

impl OpenAiEmbeddingProvider {
    pub fn new(id: String, name: String, config: EmbeddingConfig) -> Self {
        Self {
            id,
            name,
            config,
            client: reqwest::Client::new(),
        }
    }
    fn auth_header(&self) -> String {
        format!("Bearer {}", self.config.api_key)
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbeddingProvider {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut body = serde_json::json!({"model": self.config.model, "input": texts});
        if let Some(dims) = self.config.dimensions {
            body["dimensions"] = serde_json::json!(dims);
        }
        let url = format!(
            "{}/v1/embeddings",
            self.config.base_url.trim_end_matches('/')
        );
        let response = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Embedding request failed: {}", e)))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Provider {
                provider: self.name.clone(),
                message: format!("HTTP {}: {}", status, text),
            });
        }
        let resp: OpenAiEmbeddingResponse = response.json().await.map_err(|e| {
            AstrBotError::Serialization(format!("Failed to parse embedding response: {}", e))
        })?;
        let mut indexed: Vec<(usize, Vec<f32>)> = resp
            .data
            .into_iter()
            .map(|item| (item.index, item.embedding))
            .collect();
        indexed.sort_by_key(|(i, _)| *i);
        Ok(indexed.into_iter().map(|(_, emb)| emb).collect())
    }
    async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/v1/models", self.config.base_url.trim_end_matches('/'));
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
    use tokio::net::TcpListener;
    async fn spawn_mock_server(response_body: String) -> (tokio::task::JoinHandle<()>, String) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{}", port);
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = vec![0u8; 4096];
            let _ = stream.read(&mut buf).await.unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        (handle, base_url)
    }
    #[tokio::test]
    async fn test_openai_embedding_success() {
        let mock_resp = serde_json::json!({"object": "list", "data": [{"index": 0, "embedding": [0.1, 0.2, 0.3]}, {"index": 1, "embedding": [0.4, 0.5, 0.6]}], "model": "text-embedding-3-small"}).to_string();
        let (_h, base_url) = spawn_mock_server(mock_resp).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        let config = EmbeddingConfig {
            api_key: "sk-test".into(),
            base_url,
            model: "text-embedding-3-small".into(),
            dimensions: None,
        };
        let provider =
            OpenAiEmbeddingProvider::new("test-openai".into(), "Test OpenAI".into(), config);
        let result = provider
            .embed(vec!["hello".into(), "world".into()])
            .await
            .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], vec![0.1, 0.2, 0.3]);
        assert_eq!(result[1], vec![0.4, 0.5, 0.6]);
    }
    #[tokio::test]
    async fn test_openai_embedding_with_dimensions() {
        let mock_resp = serde_json::json!({"object": "list", "data": [{"index": 0, "embedding": [0.1, 0.2]}], "model": "text-embedding-3-small"}).to_string();
        let (_h, base_url) = spawn_mock_server(mock_resp).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        let config = EmbeddingConfig {
            api_key: "sk-test".into(),
            base_url,
            model: "text-embedding-3-small".into(),
            dimensions: Some(2),
        };
        let provider = OpenAiEmbeddingProvider::new("test-dims".into(), "Test Dims".into(), config);
        let result = provider.embed(vec!["test".into()]).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], vec![0.1, 0.2]);
    }
    #[tokio::test]
    async fn test_openai_embedding_empty_input() {
        let config = EmbeddingConfig {
            api_key: "sk-test".into(),
            base_url: "http://localhost:9999".into(),
            model: "text-embedding-ada-002".into(),
            dimensions: None,
        };
        let provider =
            OpenAiEmbeddingProvider::new("test-empty".into(), "Test Empty".into(), config);
        assert!(provider.embed(Vec::new()).await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn test_openai_embedding_http_error() {
        let config = EmbeddingConfig {
            api_key: "sk-test".into(),
            base_url: "http://127.0.0.1:1".into(),
            model: "text-embedding-ada-002".into(),
            dimensions: None,
        };
        let provider = OpenAiEmbeddingProvider::new("test-err".into(), "Test Err".into(), config);
        assert!(provider.embed(vec!["test".into()]).await.is_err());
    }
}
