//! Knowledge Base (RAG) tools for agent use.
//!
//! Provides `kb_search`, `kb_index`, `kb_list`, and `kb_delete` tools
//! that wrap the existing `Retriever` + `VectorStore` infrastructure.

use crate::errors::{AstrBotError, Result};
use crate::rag::{Document, Retriever, SplitStrategy, TextSplitter};
use crate::tools::{Tool, ToolDefinition, ToolParameter, ToolResult};
use crate::vector_store::VectorStore;
use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::{json, Value};
use std::sync::Arc;

/// Shared mapping: doc_id → chunk_ids. Used by `KbIndexTool` and `KbDeleteTool`.
pub type DocChunkMap = Arc<DashMap<String, Vec<String>>>;

// ── KbSearchTool ──

/// Search the knowledge base for relevant chunks.
pub struct KbSearchTool {
    definition: ToolDefinition,
    retriever: Arc<Retriever>,
}

impl KbSearchTool {
    pub fn new(retriever: Arc<Retriever>) -> Self {
        Self {
            definition: ToolDefinition {
                name: "kb_search".to_string(),
                description: "Search the knowledge base for relevant document chunks".to_string(),
                parameters: vec![ToolParameter {
                    name: "query".to_string(),
                    description: "Search query text".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                    default: None,
                    enum_values: None,
                }],
                returns: Some("array".to_string()),
                requires_confirmation: false,
            },
            retriever,
        }
    }
}

#[async_trait]
impl Tool for KbSearchTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(&self, arguments: &Value) -> Result<ToolResult> {
        let query = arguments
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AstrBotError::Validation("Missing 'query' parameter".to_string()))?;

        let results = self.retriever.retrieve(query).await?;

        let output: Vec<Value> = results
            .into_iter()
            .map(|r| {
                json!({
                    "id": r.id,
                    "score": r.score,
                    "metadata": r.metadata,
                })
            })
            .collect();

        Ok(ToolResult::Success {
            output: Value::Array(output),
        })
    }
}

// ── KbIndexTool ──

/// Add a document to the knowledge base.
pub struct KbIndexTool {
    definition: ToolDefinition,
    retriever: Arc<Retriever>,
    doc_chunks: DocChunkMap,
}

impl KbIndexTool {
    pub fn new(retriever: Arc<Retriever>, doc_chunks: DocChunkMap) -> Self {
        Self {
            definition: ToolDefinition {
                name: "kb_index".to_string(),
                description: "Add a document to the knowledge base".to_string(),
                parameters: vec![
                    ToolParameter {
                        name: "doc_id".to_string(),
                        description: "Unique document ID".to_string(),
                        param_type: "string".to_string(),
                        required: true,
                        default: None,
                        enum_values: None,
                    },
                    ToolParameter {
                        name: "title".to_string(),
                        description: "Document title".to_string(),
                        param_type: "string".to_string(),
                        required: true,
                        default: None,
                        enum_values: None,
                    },
                    ToolParameter {
                        name: "content".to_string(),
                        description: "Document content".to_string(),
                        param_type: "string".to_string(),
                        required: true,
                        default: None,
                        enum_values: None,
                    },
                    ToolParameter {
                        name: "chunk_size".to_string(),
                        description: "Chunk size in characters (default 500)".to_string(),
                        param_type: "number".to_string(),
                        required: false,
                        default: Some(json!(500)),
                        enum_values: None,
                    },
                    ToolParameter {
                        name: "overlap".to_string(),
                        description: "Chunk overlap in characters (default 50)".to_string(),
                        param_type: "number".to_string(),
                        required: false,
                        default: Some(json!(50)),
                        enum_values: None,
                    },
                ],
                returns: Some("object".to_string()),
                requires_confirmation: false,
            },
            retriever,
            doc_chunks,
        }
    }
}

#[async_trait]
impl Tool for KbIndexTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(&self, arguments: &Value) -> Result<ToolResult> {
        let doc_id = arguments
            .get("doc_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AstrBotError::Validation("Missing 'doc_id' parameter".to_string()))?;
        let title = arguments
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled");
        let content = arguments
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AstrBotError::Validation("Missing 'content' parameter".to_string()))?;
        let chunk_size = arguments
            .get("chunk_size")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(500);
        let overlap = arguments
            .get("overlap")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(50);

        let doc = Document {
            id: doc_id.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            metadata: None,
        };

        let splitter = TextSplitter::new(SplitStrategy::FixedSize {
            chunk_size,
            overlap,
        });
        let chunks = splitter.split(&doc);
        let chunk_ids: Vec<String> = chunks.iter().map(|c| c.id.clone()).collect();

        self.retriever.index_document(&doc, &splitter).await?;
        self.doc_chunks.insert(doc_id.to_string(), chunk_ids);

        Ok(ToolResult::Success {
            output: json!({
                "doc_id": doc_id,
                "chunks_indexed": chunks.len(),
            }),
        })
    }
}

// ── KbListTool ──

/// List all knowledge base collections.
pub struct KbListTool {
    definition: ToolDefinition,
    store: Arc<dyn VectorStore>,
}

impl KbListTool {
    pub fn new(store: Arc<dyn VectorStore>) -> Self {
        Self {
            definition: ToolDefinition {
                name: "kb_list".to_string(),
                description: "List all knowledge base collections".to_string(),
                parameters: vec![],
                returns: Some("array".to_string()),
                requires_confirmation: false,
            },
            store,
        }
    }
}

#[async_trait]
impl Tool for KbListTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(&self, _arguments: &Value) -> Result<ToolResult> {
        let collections = self.store.list_collections().await?;
        Ok(ToolResult::Success {
            output: Value::Array(collections.into_iter().map(|c| Value::String(c)).collect()),
        })
    }
}

// ── KbDeleteTool ──

/// Delete a document and all its chunks from the knowledge base.
pub struct KbDeleteTool {
    definition: ToolDefinition,
    store: Arc<dyn VectorStore>,
    collection: String,
    doc_chunks: DocChunkMap,
}

impl KbDeleteTool {
    pub fn new(
        store: Arc<dyn VectorStore>,
        collection: impl Into<String>,
        doc_chunks: DocChunkMap,
    ) -> Self {
        Self {
            definition: ToolDefinition {
                name: "kb_delete".to_string(),
                description: "Delete a document and all its chunks from the knowledge base"
                    .to_string(),
                parameters: vec![ToolParameter {
                    name: "doc_id".to_string(),
                    description: "Document ID to delete".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                    default: None,
                    enum_values: None,
                }],
                returns: Some("object".to_string()),
                requires_confirmation: true,
            },
            store,
            collection: collection.into(),
            doc_chunks,
        }
    }
}

#[async_trait]
impl Tool for KbDeleteTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(&self, arguments: &Value) -> Result<ToolResult> {
        let doc_id = arguments
            .get("doc_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AstrBotError::Validation("Missing 'doc_id' parameter".to_string()))?;

        let removed = if let Some((_, chunk_ids)) = self.doc_chunks.remove(doc_id) {
            for chunk_id in &chunk_ids {
                let _ = self.store.delete(&self.collection, chunk_id).await;
            }
            chunk_ids.len()
        } else {
            0
        };

        Ok(ToolResult::Success {
            output: json!({
                "doc_id": doc_id,
                "chunks_removed": removed,
            }),
        })
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rag::embedding::{EmbeddingConfig, EmbeddingProvider};
    use crate::rag::rerank::{RerankProvider, RerankResult};
    use crate::vector_store::MemoryVectorStore;
    use async_trait::async_trait;
    use std::sync::Arc;

    struct MockEmbeddingProvider;
    #[async_trait]
    impl EmbeddingProvider for MockEmbeddingProvider {
        fn id(&self) -> &str {
            "mock-emb"
        }
        fn name(&self) -> &str {
            "Mock Embedding"
        }
        async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
            Ok(texts
                .into_iter()
                .map(|t| {
                    let hash = t.bytes().fold(0u32, |acc, b| acc.wrapping_add(b as u32));
                    let mut vec = vec![0.0f32; 8];
                    vec[(hash % 8) as usize] = 1.0;
                    vec
                })
                .collect())
        }
        async fn health_check(&self) -> Result<bool> {
            Ok(true)
        }
    }

    fn test_retriever(
        collection: &str,
    ) -> (
        Arc<dyn EmbeddingProvider>,
        Arc<dyn VectorStore>,
        Arc<Retriever>,
    ) {
        let embedding: Arc<dyn EmbeddingProvider> = Arc::new(MockEmbeddingProvider);
        let store: Arc<dyn VectorStore> = Arc::new(MemoryVectorStore::new());
        let retriever = Arc::new(Retriever::new(
            embedding.clone(),
            store.clone(),
            collection,
            5,
        ));
        (embedding, store, retriever)
    }

    #[tokio::test]
    async fn test_kb_index_and_search() {
        let (_, _store, retriever) = test_retriever("test_coll");
        let doc_chunks: DocChunkMap = Arc::new(DashMap::new());

        let index_tool = KbIndexTool::new(retriever.clone(), doc_chunks.clone());
        let search_tool = KbSearchTool::new(retriever);

        let r = index_tool
            .execute(&json!({
                "doc_id": "doc1",
                "title": "Rust Guide",
                "content": "Rust is a systems programming language. It guarantees memory safety.",
                "chunk_size": 30,
                "overlap": 5,
            }))
            .await
            .unwrap();
        assert!(matches!(r, ToolResult::Success { .. }));

        let r = search_tool
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

    #[tokio::test]
    async fn test_kb_list() {
        let (_, store, retriever) = test_retriever("list_coll");
        let doc_chunks: DocChunkMap = Arc::new(DashMap::new());

        let index_tool = KbIndexTool::new(retriever, doc_chunks);
        let list_tool = KbListTool::new(store);

        index_tool
            .execute(&json!({
                "doc_id": "d1",
                "title": "T",
                "content": "hello world",
                "chunk_size": 100,
                "overlap": 10,
            }))
            .await
            .unwrap();

        let r = list_tool.execute(&json!({})).await.unwrap();
        match r {
            ToolResult::Success { output } => {
                let arr = output.as_array().unwrap();
                assert!(arr.iter().any(|v| v.as_str() == Some("list_coll")));
            }
            _ => panic!("Expected success"),
        }
    }

    #[tokio::test]
    async fn test_kb_delete() {
        let (_, store, retriever) = test_retriever("del_coll");
        let doc_chunks: DocChunkMap = Arc::new(DashMap::new());

        let index_tool = KbIndexTool::new(retriever.clone(), doc_chunks.clone());
        let delete_tool = KbDeleteTool::new(store, "del_coll", doc_chunks.clone());
        let search_tool = KbSearchTool::new(retriever);

        index_tool
            .execute(&json!({
                "doc_id": "del_doc",
                "title": "Delete Me",
                "content": "This document will be deleted.",
                "chunk_size": 50,
                "overlap": 5,
            }))
            .await
            .unwrap();

        let r = search_tool
            .execute(&json!({ "query": "deleted" }))
            .await
            .unwrap();
        match r {
            ToolResult::Success { output } => {
                let arr = output.as_array().unwrap();
                assert!(!arr.is_empty());
            }
            _ => panic!("Expected success"),
        }

        let r = delete_tool
            .execute(&json!({ "doc_id": "del_doc" }))
            .await
            .unwrap();
        match r {
            ToolResult::Success { output } => {
                assert_eq!(
                    output.get("chunks_removed").and_then(|v| v.as_u64()),
                    Some(1)
                );
            }
            _ => panic!("Expected success"),
        }

        let r = search_tool
            .execute(&json!({ "query": "deleted" }))
            .await
            .unwrap();
        match r {
            ToolResult::Success { output } => {
                let arr = output.as_array().unwrap();
                let found = arr.iter().any(|v| {
                    v.get("metadata")
                        .and_then(|m| m.get("doc_id"))
                        .and_then(|d| d.as_str())
                        == Some("del_doc")
                });
                assert!(!found, "Expected del_doc to be gone after delete");
            }
            _ => panic!("Expected success"),
        }
    }
}
