use crate::errors::{AstrBotError, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// Search result types
// ---------------------------------------------------------------------------

/// A single web search result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchResult {
    /// Result title
    pub title: String,
    /// Result URL
    pub url: String,
    /// Snippet / summary
    pub snippet: String,
    /// Source domain (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Publish date if known
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_date: Option<String>,
    /// Relevance score (provider-specific, higher = more relevant)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
}

impl SearchResult {
    /// Quick constructor for tests and internal use.
    pub fn new(
        title: impl Into<String>,
        url: impl Into<String>,
        snippet: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            url: url.into(),
            snippet: snippet.into(),
            source: None,
            published_date: None,
            score: None,
        }
    }
}

// ---------------------------------------------------------------------------
// SearchEngine trait
// ---------------------------------------------------------------------------

/// Abstract search engine — can be backed by Tavily, Brave, Bing, SerpAPI, etc.
#[async_trait]
pub trait SearchEngine: Send + Sync {
    /// Execute a search query and return ranked results.
    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>>;
    /// Quick health check — should return true if the engine is usable.
    async fn health_check(&self) -> Result<bool>;
}

// ---------------------------------------------------------------------------
// Tavily search (real HTTP implementation)
// ---------------------------------------------------------------------------

/// Tavily API client.
///
/// Performs real POST to `https://api.tavily.com/search`.
/// A valid `TAVILY_API_KEY` is required for non-error responses.
pub struct TavilySearch {
    api_key: String,
    client: reqwest::Client,
    base_url: String,
}

impl TavilySearch {
    /// Create a new Tavily search client.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            client: reqwest::Client::new(),
            base_url: "https://api.tavily.com".to_string(),
        }
    }

    /// Create with a custom HTTP client (e.g. shared connection pool).
    pub fn with_client(api_key: impl Into<String>, client: reqwest::Client) -> Self {
        Self {
            api_key: api_key.into(),
            client,
            base_url: "https://api.tavily.com".to_string(),
        }
    }

    /// Set a custom base URL (useful for proxies / enterprise endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}

#[derive(Debug, Serialize)]
struct TavilyRequest {
    api_key: String,
    query: String,
    search_depth: String,
    max_results: usize,
    include_answer: bool,
}

#[derive(Debug, Deserialize)]
struct TavilyResponse {
    query: String,
    #[allow(dead_code)]
    answer: Option<String>,
    results: Vec<TavilyResultItem>,
}

#[derive(Debug, Deserialize)]
struct TavilyResultItem {
    title: String,
    url: String,
    content: String,
    #[serde(default)]
    score: Option<f32>,
    #[serde(default)]
    published_date: Option<String>,
}

#[async_trait]
impl SearchEngine for TavilySearch {
    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        let url = format!("{}/search", self.base_url);
        let body = TavilyRequest {
            api_key: self.api_key.clone(),
            query: query.to_string(),
            search_depth: "basic".to_string(),
            max_results,
            include_answer: false,
        };

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Tavily request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            error!("[TavilySearch] HTTP {} — {}", status, text);
            return Err(AstrBotError::Network(format!(
                "Tavily returned HTTP {}: {}",
                status, text
            )));
        }

        let payload: TavilyResponse = response
            .json()
            .await
            .map_err(|e| AstrBotError::Serialization(format!("Tavily JSON parse error: {}", e)))?;

        let results: Vec<SearchResult> = payload
            .results
            .into_iter()
            .map(|item| SearchResult {
                title: item.title,
                url: item.url.clone(),
                snippet: item.content,
                source: extract_domain(&item.url),
                published_date: item.published_date,
                score: item.score,
            })
            .collect();

        info!(
            "[TavilySearch] query='{}' returned {} results",
            payload.query,
            results.len()
        );
        Ok(results)
    }

    async fn health_check(&self) -> Result<bool> {
        // Tavily has no dedicated health endpoint; we do a lightweight search
        // with a nonsense query and expect either results or a known error.
        match self.search("healthcheck", 1).await {
            Ok(_) => Ok(true),
            Err(AstrBotError::Network(ref msg)) if msg.contains("401") || msg.contains("403") => {
                warn!("[TavilySearch] health check: auth issue (expected with bad key)");
                Ok(false)
            }
            Err(e) => {
                warn!("[TavilySearch] health check failed: {}", e);
                Ok(false)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Brave search (real HTTP implementation)
// ---------------------------------------------------------------------------

/// Brave Search API client.
///
/// Wired to Brave's Web Search API.
/// A real `BRAVE_API_KEY` is needed for live calls.
pub struct BraveSearch {
    api_key: String,
    client: reqwest::Client,
    base_url: String,
}

impl BraveSearch {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            client: reqwest::Client::new(),
            base_url: "https://api.search.brave.com/res/v1/web/search".to_string(),
        }
    }

    pub fn with_client(api_key: impl Into<String>, client: reqwest::Client) -> Self {
        Self {
            api_key: api_key.into(),
            client,
            base_url: "https://api.search.brave.com/res/v1/web/search".to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct BraveResponse {
    web: BraveWebResults,
}

#[derive(Debug, Deserialize)]
struct BraveWebResults {
    results: Vec<BraveResultItem>,
}

#[derive(Debug, Deserialize)]
struct BraveResultItem {
    title: String,
    url: String,
    description: String,
}

#[async_trait]
impl SearchEngine for BraveSearch {
    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        let response = self
            .client
            .get(&self.base_url)
            .query(&[("q", query), ("count", &max_results.to_string())])
            .header("X-Subscription-Token", &self.api_key)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Brave request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            error!("[BraveSearch] HTTP {} — {}", status, text);
            return Err(AstrBotError::Network(format!(
                "Brave returned HTTP {}: {}",
                status, text
            )));
        }

        let payload: BraveResponse = response
            .json()
            .await
            .map_err(|e| AstrBotError::Serialization(format!("Brave JSON parse error: {}", e)))?;

        let results: Vec<SearchResult> = payload
            .web
            .results
            .into_iter()
            .map(|item| SearchResult {
                title: item.title,
                url: item.url.clone(),
                snippet: item.description,
                source: extract_domain(&item.url),
                published_date: None,
                score: None,
            })
            .collect();

        info!(
            "[BraveSearch] query='{}' returned {} results",
            query,
            results.len()
        );
        Ok(results)
    }

    async fn health_check(&self) -> Result<bool> {
        // Similar to Tavily: a lightweight probe query
        match self.search("healthcheck", 1).await {
            Ok(_) => Ok(true),
            Err(AstrBotError::Network(ref msg)) if msg.contains("401") || msg.contains("403") => {
                warn!("[BraveSearch] health check: auth issue");
                Ok(false)
            }
            Err(e) => {
                warn!("[BraveSearch] health check failed: {}", e);
                Ok(false)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pipeline context injection helper
// ---------------------------------------------------------------------------

/// Format search results into a system-prompt-friendly context block.
///
/// This can be injected into the Pipeline context before the LLM call.
pub fn format_search_context(query: &str, results: &[SearchResult]) -> String {
    let mut lines = vec![
        format!("## Web search results for \"{}\"", query),
        String::new(),
    ];
    for (i, r) in results.iter().enumerate() {
        lines.push(format!("### [{}] {}", i + 1, r.title));
        lines.push(format!("Source: {}", r.url));
        lines.push(r.snippet.clone());
        lines.push(String::new());
    }
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn extract_domain(url: &str) -> Option<String> {
    url.split("//")
        .nth(1)?
        .split('/')
        .next()
        .map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_result_new() {
        let r = SearchResult::new("Rust", "https://rust-lang.org", "Safe systems programming");
        assert_eq!(r.title, "Rust");
        assert_eq!(r.url, "https://rust-lang.org");
        assert_eq!(r.snippet, "Safe systems programming");
    }

    #[test]
    fn test_format_search_context() {
        let results = vec![
            SearchResult::new(
                "Rust Book",
                "https://doc.rust-lang.org/book/",
                "The Rust programming language book.",
            ),
            SearchResult::new("Crates.io", "https://crates.io/", "Rust package registry."),
        ];
        let ctx = format_search_context("rust tutorial", &results);
        assert!(ctx.contains("Rust Book"));
        assert!(ctx.contains("https://doc.rust-lang.org/book/"));
        assert!(ctx.contains("Crates.io"));
        assert!(ctx.contains("## Web search results"));
    }

    #[test]
    fn test_extract_domain() {
        assert_eq!(
            extract_domain("https://api.tavily.com/search"),
            Some("api.tavily.com".to_string())
        );
        assert_eq!(
            extract_domain("http://example.com/path"),
            Some("example.com".to_string())
        );
        assert_eq!(extract_domain("not-a-url"), None);
    }

    #[tokio::test]
    async fn test_tavily_search_without_key() {
        // Without a valid key the real HTTP call will fail with 401/403,
        // but the struct and trait wiring should compile and execute.
        let engine = TavilySearch::new("invalid-key-for-test");
        let result = engine.search("rust", 3).await;
        // We expect an auth error; assert the error type is Network
        assert!(
            matches!(result, Err(AstrBotError::Network(_))),
            "Expected network/auth error, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_tavily_health_check_returns_bool() {
        let engine = TavilySearch::new("invalid-key-for-test");
        let ok = engine.health_check().await.unwrap();
        // With a bad key health_check should return false, not Err
        assert!(!ok);
    }

    #[tokio::test]
    async fn test_brave_search_without_key() {
        let engine = BraveSearch::new("invalid-key-for-test");
        let result = engine.search("rust", 3).await;
        assert!(
            matches!(result, Err(AstrBotError::Network(_))),
            "Expected network/auth error, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_brave_health_check_returns_bool() {
        let engine = BraveSearch::new("invalid-key-for-test");
        let ok = engine.health_check().await.unwrap();
        assert!(!ok);
    }
}
