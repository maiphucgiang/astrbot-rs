//! Knowledge Base Manager — create / delete / list KBs + provide per-KB tool instances.

use crate::errors::{AstrBotError, Result};
use crate::provider::Provider;
use crate::rag::embedding::EmbeddingProvider;
use crate::rag::Retriever;
use crate::tools::kb_tools::{DocChunkMap, KbDeleteTool, KbIndexTool, KbSearchTool};
use crate::tools::{Tool, ToolResult};
use crate::vector_store::VectorStore;
use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;

/// Configuration for a single knowledge base.
#[derive(Debug, Clone)]
pub struct KbConfig {
    pub collection: String,
    pub top_k: usize,
    pub chunk_size: usize,
    pub overlap: usize,
}

impl Default for KbConfig {
    fn default() -> Self {
        Self {
            collection: "default".to_string(),
            top_k: 5,
            chunk_size: 500,
            overlap: 50,
        }
    }
}

/// Adapter that wraps a `Provider` into an `EmbeddingProvider`.
struct ProviderEmbeddingAdapter {
    provider: Arc<dyn Provider>,
    model: String,
}

#[async_trait]
impl EmbeddingProvider for ProviderEmbeddingAdapter {
    fn id(&self) -> &str {
        self.provider.id()
    }
    fn name(&self) -> &str {
        self.provider.name()
    }
    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        self.provider
            .embedding(texts, Some(self.model.clone()))
            .await
    }
    async fn health_check(&self) -> Result<bool> {
        self.provider.health_check().await
    }
}

/// Manager for multiple knowledge bases.
pub struct KbManager {
    store: Arc<dyn VectorStore>,
    embedding: Arc<dyn crate::rag::embedding::EmbeddingProvider>,
    configs: DashMap<String, KbConfig>,
    kb_doc_chunks: DashMap<String, DocChunkMap>,
}

impl KbManager {
    pub fn new(
        store: Arc<dyn VectorStore>,
        embedding: Arc<dyn crate::rag::embedding::EmbeddingProvider>,
    ) -> Self {
        Self {
            store,
            embedding,
            configs: DashMap::new(),
            kb_doc_chunks: DashMap::new(),
        }
    }

    pub async fn create_kb(&self, name: &str, config: KbConfig) -> Result<()> {
        if self.configs.contains_key(name) {
            return Err(AstrBotError::Validation(format!(
                "Knowledge base '{}' already exists",
                name
            )));
        }
        self.configs.insert(name.to_string(), config);
        self.kb_doc_chunks
            .insert(name.to_string(), Arc::new(DashMap::new()));
        Ok(())
    }

    pub async fn delete_kb(&self, name: &str) -> Result<()> {
        let config = self.configs.remove(name).map(|(_, c)| c).ok_or_else(|| {
            AstrBotError::NotFound(format!("Knowledge base '{}' not found", name))
        })?;

        if let Some((_, doc_chunks)) = self.kb_doc_chunks.remove(name) {
            for entry in doc_chunks.iter() {
                for chunk_id in entry.value() {
                    let _ = self.store.delete(&config.collection, chunk_id).await;
                }
            }
        }

        Ok(())
    }

    pub fn list_kbs(&self) -> Vec<String> {
        self.configs
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    pub fn has_kb(&self, name: &str) -> bool {
        self.configs.contains_key(name)
    }

    pub fn get_config(&self, name: &str) -> Option<KbConfig> {
        self.configs.get(name).map(|entry| entry.clone())
    }

    pub fn get_kb_tools(&self, name: &str) -> Option<(KbSearchTool, KbIndexTool, KbDeleteTool)> {
        let config = self.configs.get(name)?.clone();
        let doc_chunks = self.kb_doc_chunks.get(name)?.clone();

        let embedding = self.embedding.clone();

        let retriever = Arc::new(Retriever::new(
            embedding,
            self.store.clone(),
            config.collection.clone(),
            config.top_k,
        ));

        let search = KbSearchTool::new(retriever.clone());
        let index = KbIndexTool::new(retriever, doc_chunks.clone());
        let delete = KbDeleteTool::new(self.store.clone(), config.collection, doc_chunks);

        Some((search, index, delete))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MockProvider;
    use crate::vector_store::MemoryVectorStore;
    use serde_json::json;

    fn test_manager() -> (Arc<MockProvider>, Arc<dyn VectorStore>, KbManager) {
        let provider = Arc::new(MockProvider::new("mock-emb", "MockEmbedding"));
        let embedding: Arc<dyn EmbeddingProvider> = Arc::new(ProviderEmbeddingAdapter {
            provider: provider.clone(),
            model: "mock-model".to_string(),
        });
        let store: Arc<dyn VectorStore> = Arc::new(MemoryVectorStore::new());
        let manager = KbManager::new(store.clone(), embedding);
        (provider, store, manager)
    }

    #[tokio::test]
    async fn test_create_and_list_kb() {
        let (_, _, manager) = test_manager();
        manager
            .create_kb(
                "docs",
                KbConfig {
                    collection: "docs_coll".to_string(),
                    top_k: 3,
                    chunk_size: 100,
                    overlap: 10,
                },
            )
            .await
            .unwrap();

        let list = manager.list_kbs();
        assert!(list.contains(&"docs".to_string()));
        assert!(manager.has_kb("docs"));
    }

    #[tokio::test]
    async fn test_duplicate_create_fails() {
        let (_, _, manager) = test_manager();
        let config = KbConfig {
            collection: "dup_coll".to_string(),
            top_k: 5,
            chunk_size: 500,
            overlap: 50,
        };
        manager.create_kb("dup", config.clone()).await.unwrap();
        let result = manager.create_kb("dup", config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_kb_removes_chunks() {
        let (_, store, manager) = test_manager();
        let config = KbConfig {
            collection: "del_coll".to_string(),
            top_k: 5,
            chunk_size: 50,
            overlap: 5,
        };
        manager.create_kb("del", config).await.unwrap();

        let tools = manager.get_kb_tools("del").unwrap();
        let (search, index, _delete) = tools;

        index
            .execute(&json!({
                "doc_id": "d1",
                "title": "T",
                "content": "hello world rust memory safety",
                "chunk_size": 30,
                "overlap": 5,
            }))
            .await
            .unwrap();

        let r = search.execute(&json!({ "query": "rust" })).await.unwrap();
        match r {
            ToolResult::Success { output } => {
                let arr = output.as_array().unwrap();
                assert!(!arr.is_empty());
            }
            _ => panic!("Expected success"),
        }

        manager.delete_kb("del").await.unwrap();
        assert!(!manager.has_kb("del"));

        let r = search.execute(&json!({ "query": "rust" })).await.unwrap();
        match r {
            ToolResult::Success { output } => {
                let arr = output.as_array().unwrap();
                assert!(arr.is_empty());
            }
            _ => panic!("Expected success"),
        }
    }

    #[tokio::test]
    async fn test_delete_kb_cleans_doc_chunks() {
        let (_, _store, manager) = test_manager();
        let config = KbConfig {
            collection: "dc_coll".to_string(),
            top_k: 5,
            chunk_size: 50,
            overlap: 5,
        };
        manager.create_kb("dc", config).await.unwrap();

        let tools = manager.get_kb_tools("dc").unwrap();
        let (_search, index, _delete) = tools;

        index
            .execute(&json!({
                "doc_id": "doc1",
                "title": "T",
                "content": "some text here",
                "chunk_size": 20,
                "overlap": 2,
            }))
            .await
            .unwrap();

        {
            let dc = manager.kb_doc_chunks.get("dc").unwrap();
            assert!(dc.contains_key("doc1"));
        }

        manager.delete_kb("dc").await.unwrap();

        assert!(!manager.kb_doc_chunks.contains_key("dc"));
    }

    #[tokio::test]
    async fn test_get_kb_tools_missing_kb() {
        let (_, _, manager) = test_manager();
        assert!(manager.get_kb_tools("nonexistent").is_none());
    }

    #[tokio::test]
    async fn test_kb_tools_index_and_search() {
        let (_, _, manager) = test_manager();
        let config = KbConfig {
            collection: "kb_coll".to_string(),
            top_k: 3,
            chunk_size: 30,
            overlap: 5,
        };
        manager.create_kb("kb1", config).await.unwrap();

        let tools = manager.get_kb_tools("kb1").unwrap();
        let (search, index, _delete) = tools;

        index
            .execute(&json!({
                "doc_id": "doc1",
                "title": "Rust",
                "content": "Rust provides memory safety without garbage collection.",
                "chunk_size": 30,
                "overlap": 5,
            }))
            .await
            .unwrap();

        let r = search
            .execute(&json!({ "query": "memory safety" }))
            .await
            .unwrap();
        match r {
            ToolResult::Success { output } => {
                let arr = output.as_array().unwrap();
                assert!(!arr.is_empty());
                let found = arr.iter().any(|v| {
                    v.get("metadata")
                        .and_then(|m| m.get("doc_id"))
                        .and_then(|d| d.as_str())
                        == Some("doc1")
                });
                assert!(found, "Expected to find doc1 in search results");
            }
            _ => panic!("Expected success"),
        }
    }
}
