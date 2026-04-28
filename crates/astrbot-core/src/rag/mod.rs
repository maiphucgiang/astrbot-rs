use crate::errors::{AstrBotError, Result};
use crate::provider::Provider;
use crate::vector_store::{SearchResult, VectorStore};
use async_trait::async_trait;

#[cfg(test)]
mod tests;

/// A chunk of text with metadata
#[derive(Debug, Clone, PartialEq)]
pub struct TextChunk {
    /// Chunk ID
    pub id: String,
    /// Source document ID
    pub doc_id: String,
    /// Chunk text content
    pub text: String,
    /// Chunk index in document
    pub index: usize,
    /// Optional metadata
    pub metadata: Option<serde_json::Value>,
}

/// A document for ingestion
#[derive(Debug, Clone)]
pub struct Document {
    pub id: String,
    pub title: String,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
}

/// Text splitting strategy
pub enum SplitStrategy {
    /// Split by fixed character count
    FixedSize { chunk_size: usize, overlap: usize },
    /// Split by paragraphs (double newline)
    Paragraph,
    /// Recursive: try paragraphs, then sentences, then words
    Recursive { chunk_size: usize, overlap: usize },
}

/// Text splitter — breaks documents into chunks
pub struct TextSplitter {
    strategy: SplitStrategy,
}

impl TextSplitter {
    pub fn new(strategy: SplitStrategy) -> Self {
        Self { strategy }
    }

    /// Split a document into chunks
    pub fn split(&self,
        doc: &Document,
    ) -> Vec<TextChunk> {
        match &self.strategy {
            SplitStrategy::FixedSize { chunk_size, overlap } => {
                self.split_fixed(&doc.id, &doc.content, *chunk_size, *overlap)
            }
            SplitStrategy::Paragraph => {
                self.split_paragraph(&doc.id, &doc.content)
            }
            SplitStrategy::Recursive { chunk_size, overlap } => {
                self.split_recursive(&doc.id, &doc.content, *chunk_size, *overlap)
            }
        }
    }

    fn split_fixed(
        &self,
        doc_id: &str,
        content: &str,
        chunk_size: usize,
        overlap: usize,
    ) -> Vec<TextChunk> {
        let mut chunks = Vec::new();
        let mut start = 0;
        let mut index = 0;

        while start < content.len() {
            let end = (start + chunk_size).min(content.len());
            let text = content[start..end].to_string();

            chunks.push(TextChunk {
                id: format!("{}-chunk-{}", doc_id, index),
                doc_id: doc_id.to_string(),
                text,
                index,
                metadata: None,
            });

            start += chunk_size - overlap;
            if start >= end && start < content.len() {
                start = end; // prevent infinite loop
            }
            index += 1;
        }

        chunks
    }

    fn split_paragraph(
        &self,
        doc_id: &str,
        content: &str,
    ) -> Vec<TextChunk> {
        let paragraphs: Vec<&str> = content.split("\n\n").filter(|s| !s.trim().is_empty()).collect();
        paragraphs.into_iter().enumerate().map(|(index, text)| {
            TextChunk {
                id: format!("{}-chunk-{}", doc_id, index),
                doc_id: doc_id.to_string(),
                text: text.trim().to_string(),
                index,
                metadata: None,
            }
        }).collect()
    }

    fn split_recursive(
        &self,
        doc_id: &str,
        content: &str,
        chunk_size: usize,
        overlap: usize,
    ) -> Vec<TextChunk> {
        // First try paragraphs
        let paragraphs: Vec<&str> = content.split("\n\n").filter(|s| !s.trim().is_empty()).collect();
        let mut chunks = Vec::new();
        let mut current = String::new();
        let mut index = 0;

        for para in paragraphs {
            if current.len() + para.len() > chunk_size && !current.is_empty() {
                chunks.push(TextChunk {
                    id: format!("{}-chunk-{}", doc_id, index),
                    doc_id: doc_id.to_string(),
                    text: current.trim().to_string(),
                    index,
                    metadata: None,
                });
                index += 1;
                current = String::new();
            }
            current.push_str(para);
            current.push('\n');
        }

        if !current.is_empty() {
            // If remaining text is too long, fall back to fixed-size
            if current.len() > chunk_size {
                chunks.extend(self.split_fixed(doc_id, &current, chunk_size, overlap));
            } else {
                chunks.push(TextChunk {
                    id: format!("{}-chunk-{}", doc_id, index),
                    doc_id: doc_id.to_string(),
                    text: current.trim().to_string(),
                    index,
                    metadata: None,
                });
            }
        }

        chunks
    }
}

/// A record in the embedding store
#[derive(Debug, Clone)]
pub struct EmbeddingRecord {
    pub chunk_id: String,
    pub doc_id: String,
    pub text: String,
    pub embedding: Vec<f32>,
}

/// Interface for vector embedding storage and retrieval
#[async_trait]
pub trait EmbeddingStore: Send + Sync {
    /// Store a batch of records
    async fn store(&mut self,
        records: Vec<EmbeddingRecord>,
    ) -> Result<()>;
    /// Search for top-k similar embeddings
    async fn search(
        &self,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<( EmbeddingRecord, f32 )>>;
    /// Delete all records for a document
    async fn delete_by_doc(&mut self,
        doc_id: &str,
    ) -> Result<()>;
    /// Get total record count
    fn count(&self) -> usize;
}

/// Simple in-memory embedding store using cosine similarity
///
/// Production use should switch to Faiss, pgvector, or Qdrant.
pub struct MemoryEmbeddingStore {
    records: Vec<EmbeddingRecord>,
}

impl Default for MemoryEmbeddingStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryEmbeddingStore {
    pub fn new() -> Self {
        Self { records: Vec::new() }
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
impl EmbeddingStore for MemoryEmbeddingStore {
    async fn store(&mut self,
        records: Vec<EmbeddingRecord>,
    ) -> Result<()> {
        self.records.extend(records);
        Ok(())
    }

    async fn search(
        &self,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<(EmbeddingRecord, f32)>> {
        let mut scored: Vec<(EmbeddingRecord, f32)> = self.records.iter()
            .map(|r| (r.clone(), Self::cosine_similarity(query_embedding, &r.embedding)))
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        Ok(scored)
    }

    async fn delete_by_doc(&mut self,
        doc_id: &str,
    ) -> Result<()> {
        self.records.retain(|r| r.doc_id != doc_id);
        Ok(())
    }

    fn count(&self) -> usize {
        self.records.len()
    }
}

/// Retriever — orchestrates embedding generation + vector search
pub struct Retriever {
    provider: std::sync::Arc<dyn Provider>,
    embedding_model: String,
    store: std::sync::Arc<dyn VectorStore>,
    collection: String,
    top_k: usize,
}

impl Retriever {
    pub fn new(
        provider: std::sync::Arc<dyn Provider>,
        embedding_model: impl Into<String>,
        store: std::sync::Arc<dyn VectorStore>,
        collection: impl Into<String>,
        top_k: usize,
    ) -> Self {
        Self {
            provider,
            embedding_model: embedding_model.into(),
            store,
            collection: collection.into(),
            top_k,
        }
    }

    /// Index a document: split → embed → store
    pub async fn index_document(
        &self,
        doc: &Document,
        splitter: &TextSplitter,
    ) -> Result<()> {
        let chunks = splitter.split(doc);
        if chunks.is_empty() {
            return Ok(());
        }

        let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        let embeddings = self.provider.embedding(texts, Some(self.embedding_model.clone())).await?;

        if embeddings.len() != chunks.len() {
            return Err(AstrBotError::Internal(format!(
                "Embedding count mismatch: {} chunks vs {} embeddings",
                chunks.len(), embeddings.len()
            )));
        }

        for (chunk, emb) in chunks.into_iter().zip(embeddings.into_iter()) {
            let metadata = serde_json::json!({
                "doc_id": chunk.doc_id,
                "text": chunk.text,
                "index": chunk.index,
            });
            self.store.upsert(
                &self.collection,
                &chunk.id,
                emb,
                Some(metadata),
            ).await?;
        }

        Ok(())
    }

    /// Query: embed query → search store → return top-k results
    pub async fn query(&self, query: &str) -> Result<Vec<SearchResult>> {
        let embeddings = self.provider.embedding(
            vec![query.to_string()],
            Some(self.embedding_model.clone()),
        ).await?;

        let query_emb = embeddings.into_iter().next()
            .ok_or_else(|| AstrBotError::Internal("Query embedding returned empty".to_string()))?;

        self.store.search(&self.collection, query_emb, self.top_k).await
    }
}

/// Document parser — extracts text from various formats
///
/// Skeleton: supports plain text and JSON metadata only.
/// Full implementation would use `calamine` for Excel, `lopdf` for PDF, etc.
pub struct DocumentParser;

impl DocumentParser {
    /// Parse raw text (no transformation)
    pub fn parse_text(content: &str, title: impl Into<String>) -> Document {
        Document {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.into(),
            content: content.to_string(),
            metadata: None,
        }
    }

    /// Parse JSON content (expects {"title": "...", "content": "..."})
    pub fn parse_json(json_str: &str) -> Result<Document> {
        let val: serde_json::Value = serde_json::from_str(json_str)
            .map_err(|e| AstrBotError::Serialization(format!("JSON parse error: {}", e)))?;

        let title = val.get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("untitled")
            .to_string();
        let content = val.get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(Document {
            id: uuid::Uuid::new_v4().to_string(),
            title,
            content,
            metadata: Some(val),
        })
    }
}
