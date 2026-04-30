use crate::errors::{AstrBotError, Result};
use crate::rag::document::{Document, TextChunk};
use crate::rag::embedding::EmbeddingProvider;
use crate::rag::rerank::RerankProvider;
use crate::rag::splitter::TextSplitter;
use crate::vector_store::{SearchResult, VectorStore};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Chunk { pub id: String, pub text: String, pub score: f32, pub metadata: Option<Value> }

pub struct Retriever {
    embedding: Arc<dyn EmbeddingProvider>,
    rerank: Option<Arc<dyn RerankProvider>>,
    store: Arc<dyn VectorStore>,
    collection: String,
    top_k: usize,
}

impl Retriever {
    pub fn new(embedding: Arc<dyn EmbeddingProvider>, store: Arc<dyn VectorStore>, collection: impl Into<String>, top_k: usize) -> Self {
        Self { embedding, rerank: None, store, collection: collection.into(), top_k }
    }
    pub fn with_rerank(mut self, rerank: Arc<dyn RerankProvider>) -> Self { self.rerank = Some(rerank); self }

    pub async fn index_document(&self, doc: &Document, splitter: &TextSplitter) -> Result<()> {
        let chunks = splitter.split(doc);
        if chunks.is_empty() { return Ok(()); }
        let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        let embeddings = self.embedding.embed(texts).await?;
        if embeddings.len() != chunks.len() {
            return Err(AstrBotError::Internal(format!("Embedding count mismatch: {} chunks vs {} embeddings", chunks.len(), embeddings.len())));
        }
        for (chunk, emb) in chunks.into_iter().zip(embeddings.into_iter()) {
            let metadata = serde_json::json!({"doc_id": chunk.doc_id, "text": chunk.text, "index": chunk.index});
            self.store.upsert(&self.collection, &chunk.id, emb, Some(metadata)).await?;
        }
        Ok(())
    }

    pub async fn retrieve(&self, query: &str) -> Result<Vec<Chunk>> {
        let query_embeddings = self.embedding.embed(vec![query.to_string()]).await?;
        let query_emb = query_embeddings.into_iter().next().ok_or_else(|| AstrBotError::Internal("Query embedding returned empty".to_string()))?;
        let mut results = self.store.search(&self.collection, query_emb, self.top_k).await?;
        if let Some(reranker) = &self.rerank {
            if !results.is_empty() {
                let docs: Vec<String> = results.iter().map(|r| r.metadata.as_ref().and_then(|m| m.get("text").and_then(|t| t.as_str())).unwrap_or(&r.id).to_string()).collect();
                let reranked = reranker.rerank(query, docs).await?;
                let mut scored_results: Vec<(SearchResult, f32)> = results.into_iter().enumerate().filter_map(|(idx, sr)| {
                    reranked.iter().find(|r| r.index == idx).map(|r| (sr, r.score))
                }).collect();
                scored_results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                results = scored_results.into_iter().map(|(sr, _)| sr).collect();
            }
        }
        Ok(results.into_iter().map(|sr| Chunk {
            id: sr.id,
            text: sr.metadata.as_ref().and_then(|m| m.get("text").and_then(|t| t.as_str())).unwrap_or("").to_string(),
            score: sr.score, metadata: sr.metadata,
        }).collect())
    }

    pub async fn delete_document(&self, doc_id: &str) -> Result<()> {
        self.store.delete(&self.collection, doc_id).await?; Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::Result;
    use crate::rag::embedding::{EmbeddingConfig, EmbeddingProvider, OpenAiEmbeddingProvider};
    use crate::rag::rerank::{RerankConfig, RerankProvider, RerankResult};
    use crate::vector_store::MemoryVectorStore;
    use async_trait::async_trait;

    struct MockEmbeddingProvider;
    #[async_trait] impl EmbeddingProvider for MockEmbeddingProvider {
        fn id(&self) -> &str { "mock-emb" }
        fn name(&self) -> &str { "Mock Embedding" }
        async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
            Ok(texts.into_iter().map(|t| {
                let hash = t.bytes().fold(0u32, |acc, b| acc.wrapping_add(b as u32));
                let mut vec = vec![0.0f32; 8]; vec[(hash % 8) as usize] = 1.0; vec
            }).collect())
        }
        async fn health_check(&self) -> Result<bool> { Ok(true) }
    }

    struct MockRerankProvider;
    #[async_trait] impl RerankProvider for MockRerankProvider {
        fn id(&self) -> &str { "mock-rerank" }
        fn name(&self) -> &str { "Mock Rerank" }
        async fn rerank(&self, _query: &str, documents: Vec<String>) -> Result<Vec<RerankResult>> {
            let n = documents.len();
            let mut results: Vec<RerankResult> = documents.into_iter().enumerate().map(|(idx, doc)|
                RerankResult { document: doc, score: (n - idx) as f32, index: idx }).collect();
            results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
            Ok(results)
        }
        async fn health_check(&self) -> Result<bool> { Ok(true) }
    }

    #[tokio::test] async fn test_retriever_index_and_query() {
        let retriever = Retriever::new(Arc::new(MockEmbeddingProvider), Arc::new(MemoryVectorStore::new()), "test-col", 3);
        let doc = Document { id: "doc-1".to_string(), title: "Test".to_string(), content: "Hello world this is a test document".to_string(), metadata: None };
        let splitter = TextSplitter::new(crate::rag::splitter::SplitStrategy::FixedSize { chunk_size: 20, overlap: 5 });
        retriever.index_document(&doc, &splitter).await.unwrap();
        assert!(!retriever.retrieve("test").await.unwrap().is_empty());
    }
    #[tokio::test] async fn test_retriever_with_rerank() {
        let retriever = Retriever::new(Arc::new(MockEmbeddingProvider), Arc::new(MemoryVectorStore::new()), "test-col-rerank", 3)
            .with_rerank(Arc::new(MockRerankProvider));
        let doc = Document { id: "doc-2".to_string(), title: "Rerank Test".to_string(), content: "First chunk\n\nSecond chunk\n\nThird chunk".to_string(), metadata: None };
        let splitter = TextSplitter::new(crate::rag::splitter::SplitStrategy::Paragraph);
        retriever.index_document(&doc, &splitter).await.unwrap();
        assert!(!retriever.retrieve("chunk").await.unwrap().is_empty());
    }
    #[tokio::test] async fn test_retriever_empty_document() {
        let retriever = Retriever::new(Arc::new(MockEmbeddingProvider), Arc::new(MemoryVectorStore::new()), "test-empty", 3);
        let doc = Document { id: "doc-empty".to_string(), title: "Empty".to_string(), content: "".to_string(), metadata: None };
        let splitter = TextSplitter::new(crate::rag::splitter::SplitStrategy::FixedSize { chunk_size: 10, overlap: 2 });
        retriever.index_document(&doc, &splitter).await.unwrap();
        assert!(retriever.retrieve("anything").await.unwrap().is_empty());
    }
}
