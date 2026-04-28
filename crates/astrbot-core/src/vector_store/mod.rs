use crate::errors::{AstrBotError, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::Value;
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// Result of a vector search
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: String,
    pub score: f32,
    pub metadata: Option<Value>,
}

/// Trait for vector storage backends
#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Upsert a vector record into a collection
    async fn upsert(
        &self,
        collection: &str,
        id: &str,
        vector: Vec<f32>,
        metadata: Option<Value>,
    ) -> Result<()>;

    /// Search for top-k similar vectors in a collection
    async fn search(
        &self,
        collection: &str,
        query: Vec<f32>,
        top_k: usize,
    ) -> Result<Vec<SearchResult>>;

    /// Delete a record by ID from a collection
    async fn delete(&self, collection: &str, id: &str) -> Result<()>;

    /// List all collections
    async fn list_collections(&self) -> Result<Vec<String>>;
}

// ───────────────────────────────────────────────
// MemoryVectorStore
// ───────────────────────────────────────────────

/// In-memory vector store for testing
pub struct MemoryVectorStore {
    data: DashMap<String, Vec<(String, Vec<f32>, Option<Value>)>>,
}

impl Default for MemoryVectorStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryVectorStore {
    pub fn new() -> Self {
        Self {
            data: DashMap::new(),
        }
    }

    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm_a > 0.0 && norm_b > 0.0 {
            dot / (norm_a * norm_b)
        } else {
            0.0
        }
    }
}

#[async_trait]
impl VectorStore for MemoryVectorStore {
    async fn upsert(
        &self,
        collection: &str,
        id: &str,
        vector: Vec<f32>,
        metadata: Option<Value>,
    ) -> Result<()> {
        let mut entry = self.data.entry(collection.to_string()).or_default();
        // Remove existing record with same id
        entry.retain(|(existing_id, _, _)| existing_id != id);
        entry.push((id.to_string(), vector, metadata));
        Ok(())
    }

    async fn search(
        &self,
        collection: &str,
        query: Vec<f32>,
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        let entry = self.data.get(collection);
        let records = match entry {
            Some(e) => e.clone(),
            None => return Ok(Vec::new()),
        };

        let mut scored: Vec<SearchResult> = records
            .into_iter()
            .map(|(id, vec, meta)| {
                let score = Self::cosine_similarity(&query, &vec);
                SearchResult {
                    id,
                    score,
                    metadata: meta,
                }
            })
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(top_k);
        Ok(scored)
    }

    async fn delete(&self, collection: &str, id: &str) -> Result<()> {
        if let Some(mut entry) = self.data.get_mut(collection) {
            entry.retain(|(existing_id, _, _)| existing_id != id);
        }
        Ok(())
    }

    async fn list_collections(&self) -> Result<Vec<String>> {
        let collections: Vec<String> = self.data.iter().map(|e| e.key().clone()).collect();
        Ok(collections)
    }
}

// ───────────────────────────────────────────────
// PgVectorStore
// ───────────────────────────────────────────────

/// PostgreSQL vector store using pgvector extension
/// Embeddings are stored as JSON text arrays (to avoid extra pgvector crate dependency)
pub struct PgVectorStore {
    pool: sqlx::PgPool,
}

impl PgVectorStore {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Ensure pgvector extension is installed and collection table exists
    pub async fn init_collection(&self, collection: &str, dim: usize) -> Result<()> {
        sqlx::query("CREATE EXTENSION IF NOT EXISTS pgvector")
            .execute(&self.pool)
            .await
            .map_err(|e| AstrBotError::Internal(format!("pgvector extension check failed: {}", e)))?;

        let table_name = Self::sanitize_name(collection);
        let sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (
                id TEXT PRIMARY KEY,
                embedding TEXT NOT NULL,
                metadata JSONB
            )",
            table_name
        );
        sqlx::query(&sql)
            .execute(&self.pool)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Table creation failed: {}", e)))?;

        // Create index on embedding using pgvector if available
        let idx_sql = format!(
            "CREATE INDEX IF NOT EXISTS idx_{}_embedding ON {} USING ivfflat (embedding::vector({}) vector_cosine_ops)",
            table_name, table_name, dim
        );
        // Ignore index creation errors (pgvector may not be fully configured)
        let _ = sqlx::query(&idx_sql).execute(&self.pool).await;

        Ok(())
    }

    fn sanitize_name(name: &str) -> String {
        name.chars()
            .filter(|c| c.is_alphanumeric() || *c == '_')
            .collect()
    }

    fn embedding_to_json(embedding: &[f32]) -> String {
        serde_json::to_string(embedding).unwrap_or_else(|_| "[]".to_string())
    }

    fn embedding_from_json(json_str: &str) -> Result<Vec<f32>> {
        serde_json::from_str(json_str)
            .map_err(|e| AstrBotError::Serialization(format!("Embedding parse error: {}", e)))
    }
}

#[async_trait]
impl VectorStore for PgVectorStore {
    async fn upsert(
        &self,
        collection: &str,
        id: &str,
        vector: Vec<f32>,
        metadata: Option<Value>,
    ) -> Result<()> {
        let table_name = Self::sanitize_name(collection);
        let embedding_json = Self::embedding_to_json(&vector);
        let metadata_json = metadata.map(|m| m.to_string()).unwrap_or_default();

        let sql = format!(
            "INSERT INTO {} (id, embedding, metadata) VALUES ($1, $2, $3)
             ON CONFLICT (id) DO UPDATE SET embedding = EXCLUDED.embedding, metadata = EXCLUDED.metadata",
            table_name
        );

        sqlx::query(&sql)
            .bind(id)
            .bind(embedding_json)
            .bind(metadata_json)
            .execute(&self.pool)
            .await
            .map_err(|e| AstrBotError::Internal(format!("PgVector upsert failed: {}", e)))?;

        Ok(())
    }

    async fn search(
        &self,
        collection: &str,
        query: Vec<f32>,
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        let table_name = Self::sanitize_name(collection);
        let sql = format!("SELECT id, embedding, metadata FROM {}", table_name);

        let rows = sqlx::query_as::<_, (String, String, Option<String>)>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AstrBotError::Internal(format!("PgVector search failed: {}", e)))?;

        let mut scored: Vec<SearchResult> = Vec::new();
        for (id, emb_json, meta_json) in rows {
            let embedding = Self::embedding_from_json(&emb_json)?;
            let score = MemoryVectorStore::cosine_similarity(&query, &embedding);
            let metadata = meta_json
                .and_then(|s| serde_json::from_str(&s).ok());
            scored.push(SearchResult { id, score, metadata });
        }

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(top_k);
        Ok(scored)
    }

    async fn delete(&self, collection: &str, id: &str) -> Result<()> {
        let table_name = Self::sanitize_name(collection);
        let sql = format!("DELETE FROM {} WHERE id = $1", table_name);

        sqlx::query(&sql)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| AstrBotError::Internal(format!("PgVector delete failed: {}", e)))?;

        Ok(())
    }

    async fn list_collections(&self) -> Result<Vec<String>> {
        let rows = sqlx::query_as::<_, (String,)>(
            "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public'"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AstrBotError::Internal(format!("PgVector list_collections failed: {}", e)))?;

        Ok(rows.into_iter().map(|r| r.0).collect())
    }
}

// ───────────────────────────────────────────────
// MilvusStore
// ───────────────────────────────────────────────

/// Milvus vector store using REST API v2
pub struct MilvusStore {
    client: reqwest::Client,
    base_url: String,
    token: Option<String>,
}

impl MilvusStore {
    pub fn new(base_url: impl Into<String>, token: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            token,
        }
    }

    fn auth_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        if let Some(token) = &self.token {
            if let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token)) {
                headers.insert(reqwest::header::AUTHORIZATION, val);
            }
        }
        headers
    }

    async fn request<T: serde::Serialize, R: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<R> {
        let url = format!("{}{}", self.base_url.trim_end_matches('/'), path);
        let mut req = self.client.post(&url).json(body);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }

        let resp = req.send().await.map_err(|e| {
            AstrBotError::Internal(format!("Milvus HTTP request failed: {}", e))
        })?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Internal(format!(
                "Milvus API error {}: {}",
                status, text
            )));
        }

        let result = resp.json::<R>().await.map_err(|e| {
            AstrBotError::Internal(format!("Milvus JSON parse error: {}", e))
        })?;
        Ok(result)
    }
}

// Milvus API response structures
#[derive(Debug, serde::Deserialize)]
struct MilvusBaseResponse {
    code: i32,
    message: String,
}

#[derive(Debug, serde::Deserialize)]
struct MilvusListCollectionsResponse {
    code: i32,
    data: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize)]
struct MilvusSearchResult {
    id: String,
    score: f32,
    #[serde(rename = "entity")]
    entity: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize)]
struct MilvusSearchResponse {
    code: i32,
    data: Option<Vec<Vec<MilvusSearchResult>>>,
}

#[async_trait]
impl VectorStore for MilvusStore {
    async fn upsert(
        &self,
        collection: &str,
        id: &str,
        vector: Vec<f32>,
        metadata: Option<Value>,
    ) -> Result<()> {
        let mut data = serde_json::Map::new();
        data.insert("id".to_string(), Value::String(id.to_string()));
        data.insert("vector".to_string(), Value::Array(
            vector.into_iter().map(|v| Value::from(v)).collect()
        ));
        if let Some(meta) = metadata {
            for (k, v) in meta.as_object().unwrap_or(&serde_json::Map::new()) {
                data.insert(k.clone(), v.clone());
            }
        }

        let body = serde_json::json!({
            "collectionName": collection,
            "data": [data],
        });

        let resp: MilvusBaseResponse = self
            .request("/v2/vectordb/entities/insert", &body)
            .await?;

        if resp.code != 0 {
            return Err(AstrBotError::Internal(format!(
                "Milvus upsert failed: {}",
                resp.message
            )));
        }
        Ok(())
    }

    async fn search(
        &self,
        collection: &str,
        query: Vec<f32>,
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        let body = serde_json::json!({
            "collectionName": collection,
            "data": [query],
            "annsField": "vector",
            "limit": top_k,
            "outputFields": ["*"],
        });

        let resp: MilvusSearchResponse = self
            .request("/v2/vectordb/entities/search", &body)
            .await?;

        if resp.code != 0 {
            return Err(AstrBotError::Internal(format!(
                "Milvus search failed: code={}",
                resp.code
            )));
        }

        let mut results = Vec::new();
        if let Some(batch) = resp.data {
            for group in batch {
                for hit in group {
                    let metadata = hit.entity.clone();
                    results.push(SearchResult {
                        id: hit.id,
                        score: hit.score,
                        metadata,
                    });
                }
            }
        }

        // Sort by score desc
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(results)
    }

    async fn delete(&self, collection: &str, id: &str) -> Result<()> {
        let body = serde_json::json!({
            "collectionName": collection,
            "filter": format!("id == '{}'", id),
        });

        let resp: MilvusBaseResponse = self
            .request("/v2/vectordb/entities/delete", &body)
            .await?;

        if resp.code != 0 {
            return Err(AstrBotError::Internal(format!(
                "Milvus delete failed: {}",
                resp.message
            )));
        }
        Ok(())
    }

    async fn list_collections(&self) -> Result<Vec<String>> {
        let body = serde_json::json!({});
        let resp: MilvusListCollectionsResponse = self
            .request("/v2/vectordb/collections/list", &body)
            .await?;

        if resp.code != 0 {
            return Err(AstrBotError::Internal(format!(
                "Milvus list_collections failed: code={}",
                resp.code
            )));
        }

        Ok(resp.data.unwrap_or_default())
    }
}

// ───────────────────────────────────────────────
// VectorStoreRegistry
// ───────────────────────────────────────────────

/// Registry for managing multiple vector store backends
pub struct VectorStoreRegistry {
    stores: DashMap<String, Arc<dyn VectorStore>>,
    default: std::sync::RwLock<String>,
}

impl Default for VectorStoreRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl VectorStoreRegistry {
    pub fn new() -> Self {
        let stores = DashMap::new();
        let default = std::sync::RwLock::new("memory".to_string());
        Self { stores, default }
    }

    /// Register a named vector store
    pub fn register(&self, name: impl Into<String>, store: Arc<dyn VectorStore>) {
        let name = name.into();
        self.stores.insert(name.clone(), store);
        // If this is the first store, set it as default
        if self.stores.len() == 1 {
            if let Ok(mut d) = self.default.write() {
                *d = name;
            }
        }
    }

    /// Get a store by name
    pub fn get(&self, name: &str) -> Option<Arc<dyn VectorStore>> {
        self.stores.get(name).map(|s| s.clone())
    }

    /// Get the default store
    pub fn default(&self) -> Option<Arc<dyn VectorStore>> {
        let name = self.default.read().ok()?;
        self.stores.get(&*name).map(|s| s.clone())
    }

    /// Set the default store name
    pub fn set_default(&self, name: impl Into<String>) {
        let name = name.into();
        if let Ok(mut d) = self.default.write() {
            *d = name;
        }
    }

    /// List registered store names
    pub fn list(&self) -> Vec<String> {
        self.stores.iter().map(|e| e.key().clone()).collect()
    }
}

// Manual implementation of Clone for VectorStoreRegistry
impl Clone for VectorStoreRegistry {
    fn clone(&self) -> Self {
        let new = Self::new();
        for entry in self.stores.iter() {
            new.stores.insert(entry.key().clone(), entry.value().clone());
        }
        if let Ok(name) = self.default.read() {
            if let Ok(mut d) = new.default.write() {
                *d = name.clone();
            }
        }
        new
    }
}
