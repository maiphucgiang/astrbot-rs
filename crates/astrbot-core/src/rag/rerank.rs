use crate::errors::{AstrBotError, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankConfig { pub api_key: String, pub app_id: String, pub base_url: String }

#[derive(Debug, Clone, PartialEq)]
pub struct RerankResult { pub document: String, pub score: f32, pub index: usize }

#[async_trait]
pub trait RerankProvider: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    async fn rerank(&self, query: &str, documents: Vec<String>) -> Result<Vec<RerankResult>>;
    async fn health_check(&self) -> Result<bool>;
}

pub struct BailianRerankProvider { id: String, name: String, config: RerankConfig, client: reqwest::Client }
#[derive(Debug, Deserialize)] struct BailianRawResult { index: usize, score: f32 }
#[derive(Debug, Deserialize)] struct BailianCompletionResponse { output: BailianOutput }
#[derive(Debug, Deserialize)] struct BailianOutput { text: String }

impl BailianRerankProvider {
    pub fn new(id: String, name: String, config: RerankConfig) -> Self { Self { id, name, config, client: reqwest::Client::new() } }
    fn auth_header(&self) -> String { format!("Bearer {}", self.config.api_key) }
    fn build_prompt(&self, query: &str, documents: &[String]) -> String { format!("{}\n{}", query, documents.join("\n")) }
}

#[async_trait] impl RerankProvider for BailianRerankProvider {
    fn id(&self) -> &str { &self.id }
    fn name(&self) -> &str { &self.name }
    async fn rerank(&self, query: &str, documents: Vec<String>) -> Result<Vec<RerankResult>> {
        if documents.is_empty() { return Ok(Vec::new()); }
        let url = format!("{}/api/v1/apps/{}/completion", self.config.base_url.trim_end_matches('/'), self.config.app_id);
        let body = serde_json::json!({"input": {"prompt": self.build_prompt(query, &documents)}, "parameters": {}});
        let response = self.client.post(&url).header("Authorization", self.auth_header()).header("Content-Type", "application/json").json(&body).send().await
            .map_err(|e| AstrBotError::Network(format!("Rerank request failed: {}", e)))?;
        if !response.status().is_success() {
            let status = response.status(); let text = response.text().await.unwrap_or_default();
            return Err(AstrBotError::Provider { provider: self.name.clone(), message: format!("HTTP {}: {}", status, text) });
        }
        let resp: BailianCompletionResponse = response.json().await.map_err(|e| AstrBotError::Serialization(format!("Failed to parse rerank response: {}", e)))?;
        let raw_results: Vec<BailianRawResult> = serde_json::from_str(&resp.output.text).map_err(|e|
            AstrBotError::Serialization(format!("Failed to parse rerank output text as JSON: {}. Text: {}", e, resp.output.text)))?;
        let mut results: Vec<RerankResult> = raw_results.into_iter().filter_map(|raw|
            documents.get(raw.index).map(|doc| RerankResult { document: doc.clone(), score: raw.score, index: raw.index })).collect();
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        Ok(results)
    }
    async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/api/v1/apps/{}/completion", self.config.base_url.trim_end_matches('/'), self.config.app_id);
        match self.client.get(&url).header("Authorization", self.auth_header()).send().await {
            Ok(resp) => Ok(resp.status().is_success()), Err(_) => Ok(false),
        }
    }
}

#[cfg(test)] mod tests {
    use super::*; use tokio::io::{AsyncReadExt, AsyncWriteExt}; use tokio::net::TcpListener;
    async fn spawn_mock_server(response_body: String) -> (tokio::task::JoinHandle<()>, String) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port(); let base_url = format!("http://127.0.0.1:{}", port);
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096]; let _ = stream.read(&mut buf).await.unwrap();
            let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}", response_body.len(), response_body);
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        (handle, base_url)
    }
    #[tokio::test] async fn test_bailian_rerank_success() {
        let mock_resp = serde_json::json!({"output": {"text": "[{\"index\": 1, \"score\": 0.95}, {\"index\": 0, \"score\": 0.82}]"}, "usage": {"total_tokens": 50}}).to_string();
        let (_h, base_url) = spawn_mock_server(mock_resp).await; tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        let config = RerankConfig { api_key: "sk-test".into(), app_id: "app-123".into(), base_url };
        let provider = BailianRerankProvider::new("test-bailian".into(), "Test Bailian".into(), config);
        let result = provider.rerank("什么是重排序", vec!["量子计算是前沿领域".into(), "重排序模型用于搜索引擎".into()]).await.unwrap();
        assert_eq!(result.len(), 2); assert_eq!(result[0].index, 1); assert_eq!(result[0].document, "重排序模型用于搜索引擎");
        assert!((result[0].score - 0.95).abs() < f32::EPSILON);
    }
    #[tokio::test] async fn test_bailian_rerank_empty_documents() {
        let config = RerankConfig { api_key: "sk-test".into(), app_id: "app-123".into(), base_url: "http://localhost:9999".into() };
        let provider = BailianRerankProvider::new("test-empty".into(), "Test Empty".into(), config);
        assert!(provider.rerank("query", Vec::new()).await.unwrap().is_empty());
    }
    #[tokio::test] async fn test_bailian_rerank_http_error() {
        let config = RerankConfig { api_key: "sk-test".into(), app_id: "app-123".into(), base_url: "http://127.0.0.1:1".into() };
        let provider = BailianRerankProvider::new("test-err".into(), "Test Err".into(), config);
        assert!(provider.rerank("query", vec!["doc1".into()]).await.is_err());
    }
    #[tokio::test] async fn test_bailian_rerank_partial_results() {
        let mock_resp = serde_json::json!({"output": {"text": "[{\"index\": 2, \"score\": 0.99}]"}}).to_string();
        let (_h, base_url) = spawn_mock_server(mock_resp).await; tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        let config = RerankConfig { api_key: "sk-test".into(), app_id: "app-123".into(), base_url };
        let provider = BailianRerankProvider::new("test-partial".into(), "Test Partial".into(), config);
        let result = provider.rerank("query", vec!["doc0".into(), "doc1".into(), "doc2".into()]).await.unwrap();
        assert_eq!(result.len(), 1); assert_eq!(result[0].index, 2); assert_eq!(result[0].document, "doc2");
        assert!((result[0].score - 0.99).abs() < f32::EPSILON);
    }
}
