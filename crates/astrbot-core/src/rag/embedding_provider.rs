use crate::errors::{AstrBotError, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Unified embedding provider trait for RAG.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Provider ID
    fn id(&self) -> &str;
    /// Provider name / type label
    fn name(&self) -> &str;
    /// Embed a single text into a vector.
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    /// Embed a batch of texts.
    async fn batch_embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>>;
    /// Check if the provider endpoint is reachable.
    async fn health_check(&self) -> Result<()>;
}

// ───────────────────────────────────────────────
// OpenAI Embedding Provider
// ───────────────────────────────────────────────

/// OpenAI-compatible embedding endpoint (`/v1/embeddings`).
#[derive(Debug, Clone)]
pub struct OpenAiEmbeddingProvider {
    pub id: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    client: Client,
}

impl OpenAiEmbeddingProvider {
    pub fn new(
        id: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            client: Client::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingData {
    embedding: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingResponse {
    data: Vec<OpenAiEmbeddingData>,
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbeddingProvider {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        "openai_embedding"
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let results = self.batch_embed(vec![text.to_string()]).await?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| AstrBotError::Internal("Empty embedding response".to_string()))
    }

    async fn batch_embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/v1/embeddings", self.base_url);
        let body = json!({
            "model": self.model,
            "input": texts,
        });

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("OpenAI embed request failed: {e}")))?;

        let status = resp.status();
        let resp_json: OpenAiEmbeddingResponse = resp
            .json()
            .await
            .map_err(|e| AstrBotError::Network(format!("OpenAI embed parse failed: {e}")))?;

        if !status.is_success() {
            return Err(AstrBotError::Network(format!(
                "OpenAI embed returned HTTP {status}"
            )));
        }

        let embeddings: Vec<Vec<f32>> = resp_json.data.into_iter().map(|d| d.embedding).collect();
        Ok(embeddings)
    }

    async fn health_check(&self) -> Result<()> {
        let url = format!("{}/v1/models", self.base_url);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("OpenAI health check failed: {e}")))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(AstrBotError::Network(format!(
                "OpenAI health check returned HTTP {}",
                resp.status()
            )))
        }
    }
}

// ───────────────────────────────────────────────
// Gemini Embedding Provider
// ───────────────────────────────────────────────

/// Google Gemini embedding endpoint.
#[derive(Debug, Clone)]
pub struct GeminiEmbeddingProvider {
    pub id: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    client: Client,
}

impl GeminiEmbeddingProvider {
    pub fn new(
        id: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            base_url: "https://generativelanguage.googleapis.com".to_string(),
            api_key: api_key.into(),
            model: model.into(),
            client: Client::new(),
        }
    }

    /// Allow overriding base_url (useful for tests / proxies).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}

#[derive(Debug, Serialize)]
struct GeminiEmbedRequest {
    model: String,
    content: GeminiContent,
}

#[derive(Debug, Serialize)]
struct GeminiContent {
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize)]
struct GeminiPart {
    text: String,
}

#[derive(Debug, Deserialize)]
struct GeminiEmbedResponse {
    embedding: GeminiEmbeddingValue,
}

#[derive(Debug, Deserialize)]
struct GeminiEmbeddingValue {
    values: Vec<f32>,
}

#[async_trait]
impl EmbeddingProvider for GeminiEmbeddingProvider {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        "gemini_embedding"
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let results = self.batch_embed(vec![text.to_string()]).await?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| AstrBotError::Internal("Empty embedding response".to_string()))
    }

    async fn batch_embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let mut embeddings = Vec::with_capacity(texts.len());

        for text in texts {
            let url = format!(
                "{}/v1beta/models/{}:embedContent?key={}",
                self.base_url, self.model, self.api_key
            );
            let body = GeminiEmbedRequest {
                model: format!("models/{}", self.model),
                content: GeminiContent {
                    parts: vec![GeminiPart { text }],
                },
            };

            let resp = self
                .client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| AstrBotError::Network(format!("Gemini embed request failed: {e}")))?;

            let status = resp.status();
            let resp_json: GeminiEmbedResponse = resp
                .json()
                .await
                .map_err(|e| AstrBotError::Network(format!("Gemini embed parse failed: {e}")))?;

            if !status.is_success() {
                return Err(AstrBotError::Network(format!(
                    "Gemini embed returned HTTP {status}"
                )));
            }

            embeddings.push(resp_json.embedding.values);
        }

        Ok(embeddings)
    }

    async fn health_check(&self) -> Result<()> {
        let url = format!(
            "{}/v1beta/models/{}?key={}",
            self.base_url, self.model, self.api_key
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Gemini health check failed: {e}")))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(AstrBotError::Network(format!(
                "Gemini health check returned HTTP {}",
                resp.status()
            )))
        }
    }
}

// ───────────────────────────────────────────────
// Tests
// ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Tiny one-shot HTTP/1.1 mock server.
    async fn run_mock_http_server(response_body: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((mut socket, _)) => {
                        let mut buf = vec![0u8; 2048];
                        let _ = socket.read(&mut buf).await;
                        let http_response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
                            response_body.len(),
                            response_body
                        );
                        let _ = socket.write_all(http_response.as_bytes()).await;
                    }
                    Err(_) => break,
                }
            }
        });
        port
    }

    // ---------- OpenAI tests ----------

    #[tokio::test]
    async fn test_openai_embed_single() {
        let body = r#"{"data":[{"embedding":[0.1,0.2,0.3]}]}"#;
        let port = run_mock_http_server(body).await;
        let provider = OpenAiEmbeddingProvider::new(
            "openai-1",
            format!("http://127.0.0.1:{}", port),
            "sk-test",
            "text-embedding-3-small",
        );
        let emb = provider.embed("hello").await.unwrap();
        assert_eq!(emb, vec![0.1_f32, 0.2, 0.3]);
    }

    #[tokio::test]
    async fn test_openai_batch_embed() {
        let body = r#"{"data":[{"embedding":[0.1,0.2]},{"embedding":[0.3,0.4]}]}"#;
        let port = run_mock_http_server(body).await;
        let provider = OpenAiEmbeddingProvider::new(
            "openai-2",
            format!("http://127.0.0.1:{}", port),
            "sk-test",
            "text-embedding-3-small",
        );
        let embs = provider
            .batch_embed(vec!["a".into(), "b".into()])
            .await
            .unwrap();
        assert_eq!(embs.len(), 2);
        assert_eq!(embs[0], vec![0.1_f32, 0.2]);
        assert_eq!(embs[1], vec![0.3_f32, 0.4]);
    }

    #[tokio::test]
    async fn test_openai_health_check() {
        let body = r#"{"data":[]}"#;
        let port = run_mock_http_server(body).await;
        let provider = OpenAiEmbeddingProvider::new(
            "openai-3",
            format!("http://127.0.0.1:{}", port),
            "sk-test",
            "text-embedding-3-small",
        );
        assert!(provider.health_check().await.is_ok());
    }

    // ---------- Gemini tests ----------

    #[tokio::test]
    async fn test_gemini_embed() {
        let body = r#"{"embedding":{"values":[0.5,0.6,0.7]}}"#;
        let port = run_mock_http_server(body).await;
        let provider = GeminiEmbeddingProvider::new("gemini-1", "test-key", "embedding-001")
            .with_base_url(format!("http://127.0.0.1:{}", port));
        let emb = provider.embed("world").await.unwrap();
        assert_eq!(emb, vec![0.5_f32, 0.6, 0.7]);
    }

    #[tokio::test]
    async fn test_gemini_batch_embed() {
        let body = r#"{"embedding":{"values":[0.8,0.9]}}"#;
        let port = run_mock_http_server(body).await;
        let provider = GeminiEmbeddingProvider::new("gemini-2", "test-key", "embedding-001")
            .with_base_url(format!("http://127.0.0.1:{}", port));
        let embs = provider
            .batch_embed(vec!["x".into(), "y".into()])
            .await
            .unwrap();
        assert_eq!(embs.len(), 2);
        assert_eq!(embs[0], vec![0.8_f32, 0.9]);
        assert_eq!(embs[1], vec![0.8_f32, 0.9]);
    }

    #[tokio::test]
    async fn test_trait_id_and_name() {
        let o = OpenAiEmbeddingProvider::new("o1", "https://api.openai.com", "k", "m");
        assert_eq!(o.id(), "o1");
        assert_eq!(o.name(), "openai_embedding");

        let g = GeminiEmbeddingProvider::new("g1", "k", "m");
        assert_eq!(g.id(), "g1");
        assert_eq!(g.name(), "gemini_embedding");
    }
}
