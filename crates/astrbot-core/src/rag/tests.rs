use crate::rag::{
    Document, DocumentParser, EmbeddingRecord, EmbeddingStore, MemoryEmbeddingStore, SplitStrategy,
    TextChunk, TextSplitter, Retriever,
};
use crate::provider::{ChatMessage, ChatConfig, ChatResponse, ChatStreamChunk, ModelInfo, TokenUsage, Provider};
use crate::errors::Result;
use crate::vector_store::MemoryVectorStore;
use async_trait::async_trait;
use futures_util::Stream;

// A mock provider for testing RAG
struct MockProvider;

#[async_trait]
impl Provider for MockProvider {
    fn id(&self) -> &str { "mock" }
    fn name(&self) -> &str { "mock" }
    async fn models(&self) -> Result<Vec<String>> { Ok(vec!["mock".to_string()]) }
    async fn chat(&self, _messages: Vec<ChatMessage>, _config: ChatConfig) -> Result<ChatResponse> {
        Ok(ChatResponse { content: "ok".to_string(), model: "mock".to_string(), usage: None, reasoning: None })
    }
    async fn chat_stream(&self, _messages: Vec<ChatMessage>, _config: ChatConfig)
        -> Result<Box<dyn Stream<Item = Result<ChatStreamChunk>> + Send>> {
        unimplemented!()
    }
    async fn embedding(&self, texts: Vec<String>, _model: Option<String>) -> Result<Vec<Vec<f32>>> {
        // Return simple deterministic embeddings for testing
        Ok(texts.into_iter().enumerate().map(|(i, t)| {
            let mut vec = vec![0.0f32; 4];
            vec[i % 4] = t.len() as f32;
            vec
        }).collect())
    }
    async fn model_info(&self, _model: &str) -> Result<ModelInfo> {
        Ok(ModelInfo { name: "mock".to_string(), context_length: 4096, supports_streaming: true, supports_vision: false, supports_function_calling: false })
    }
    async fn health_check(&self) -> Result<bool> { Ok(true) }
}

#[tokio::test]
async fn test_text_splitter_fixed() {
    let doc = Document {
        id: "d1".to_string(),
        title: "test".to_string(),
        content: "Hello world this is a test document for splitting.".to_string(),
        metadata: None,
    };
    let splitter = TextSplitter::new(SplitStrategy::FixedSize { chunk_size: 10, overlap: 2 });
    let chunks = splitter.split(&doc);
    assert!(!chunks.is_empty());
    assert_eq!(chunks[0].doc_id, "d1");
    assert_eq!(chunks[0].index, 0);
}

#[tokio::test]
async fn test_text_splitter_paragraph() {
    let doc = Document {
        id: "d2".to_string(),
        title: "test".to_string(),
        content: "Para one.\n\nPara two.\n\nPara three.".to_string(),
        metadata: None,
    };
    let splitter = TextSplitter::new(SplitStrategy::Paragraph);
    let chunks = splitter.split(&doc);
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].text, "Para one.");
    assert_eq!(chunks[1].text, "Para two.");
    assert_eq!(chunks[2].text, "Para three.");
}

#[tokio::test]
async fn test_memory_embedding_store() {
    let mut store = MemoryEmbeddingStore::new();
    let records = vec![
        EmbeddingRecord { chunk_id: "c1".to_string(), doc_id: "d1".to_string(), text: "hello".to_string(), embedding: vec![1.0, 0.0, 0.0, 0.0] },
        EmbeddingRecord { chunk_id: "c2".to_string(), doc_id: "d1".to_string(), text: "world".to_string(), embedding: vec![0.0, 1.0, 0.0, 0.0] },
    ];
    store.store(records).await.unwrap();
    assert_eq!(store.count(), 2);

    let results = store.search(&[1.0, 0.0, 0.0, 0.0], 1).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0.chunk_id, "c1");

    store.delete_by_doc("d1").await.unwrap();
    assert_eq!(store.count(), 0);
}

#[tokio::test]
async fn test_document_parser() {
    let doc = DocumentParser::parse_text("hello world", "my doc");
    assert_eq!(doc.title, "my doc");
    assert_eq!(doc.content, "hello world");

    let json_str = r#"{"title": "json doc", "content": "from json"}"#;
    let doc2 = DocumentParser::parse_json(json_str).unwrap();
    assert_eq!(doc2.title, "json doc");
    assert_eq!(doc2.content, "from json");
}

#[tokio::test]
async fn test_retriever_index_and_query() {
    let provider = std::sync::Arc::new(MockProvider);
    let store = std::sync::Arc::new(MemoryVectorStore::new());
    let retriever = Retriever::new(provider, "mock", store, "rag_test", 2);

    let doc = Document {
        id: "rd1".to_string(),
        title: "Rust guide".to_string(),
        content: "Rust is a systems programming language.\n\nIt is memory safe.".to_string(),
        metadata: None,
    };
    let splitter = TextSplitter::new(SplitStrategy::Paragraph);
    retriever.index_document(&doc, &splitter).await.unwrap();

    let results = retriever.query("memory safe").await.unwrap();
    assert!(!results.is_empty());
}
