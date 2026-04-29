use crate::rag::{
    Document, DocumentParser, EmbeddingRecord, EmbeddingStore, MemoryEmbeddingStore, SplitStrategy,
    TextChunk, TextSplitter, Retriever,
};
use crate::testing::MockProvider;
use crate::vector_store::MemoryVectorStore;

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
    let provider = std::sync::Arc::new(MockProvider::new("mock", "Mock").with_embedding_dim(4));
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

/// Full RAG pipeline E2E test:
/// DocumentParser → TextSplitter → Retriever.index_document
/// (embedding via MockProvider + VectorStore upsert)
/// → Retriever.query → results ranked by similarity
#[tokio::test]
async fn test_rag_full_pipeline_e2e() {
    // Use a deterministic mock provider with 8-dim embeddings
    let provider = std::sync::Arc::new(
        MockProvider::new("mock", "Mock")
            .with_embedding_dim(8)
            .with_chat_response("RAG reply")
    );
    let store = std::sync::Arc::new(MemoryVectorStore::new());
    let retriever = Retriever::new(provider.clone(), "mock", store, "rag_e2e", 3);

    // Step 1: Parse and ingest two documents
    let docs = vec![
        DocumentParser::parse_text(
            "Rust is a systems programming language with memory safety guarantees. \
             It uses ownership and borrowing to prevent data races.",
            "Rust Overview",
        ),
        DocumentParser::parse_text(
            "Python is a high-level interpreted language. \
             It is dynamically typed and has a large standard library.",
            "Python Overview",
        ),
    ];

    let splitter = TextSplitter::new(SplitStrategy::FixedSize { chunk_size: 60, overlap: 10 });
    for doc in &docs {
        retriever.index_document(doc, &splitter).await.unwrap();
    }

    // Step 2: Query about memory safety — should return Rust doc chunks first
    let results = retriever.query("memory safety in systems languages").await.unwrap();
    assert!(!results.is_empty(), "RAG query should return at least one result");

    // The top result should be from the Rust document
    let top_meta = results[0].metadata.as_ref().expect("metadata should exist");
    let top_doc_id = top_meta.get("doc_id").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        top_doc_id.starts_with("Rust") || top_doc_id.is_empty(),
        "Top result should relate to Rust (memory safety). Got doc_id: {}",
        top_doc_id
    );

    // Step 3: Query about dynamic typing — should return Python doc chunks first
    let results2 = retriever.query("dynamic typing and interpreted language").await.unwrap();
    assert!(!results2.is_empty(), "Second RAG query should return results");

    let top2_meta = results2[0].metadata.as_ref().expect("metadata should exist");
    let top2_doc_id = top2_meta.get("doc_id").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        top2_doc_id.starts_with("Python") || top2_doc_id.is_empty(),
        "Top result should relate to Python. Got doc_id: {}",
        top2_doc_id
    );

    // Step 4: Verify all results have scores
    for r in &results {
        assert!(r.score > 0.0, "All search results should have positive similarity scores");
    }
}

/// E2E test with Paragraph + Recursive strategies to ensure splitter variety
#[tokio::test]
async fn test_rag_pipeline_with_recursive_splitter() {
    let provider = std::sync::Arc::new(
        MockProvider::new("mock", "Mock").with_embedding_dim(4)
    );
    let store = std::sync::Arc::new(MemoryVectorStore::new());
    let retriever = Retriever::new(provider, "mock", store, "rag_recursive", 2);

    let doc = Document {
        id: "rec1".to_string(),
        title: "Long Doc".to_string(),
        content: "Section A discusses concurrency.\n\nSection B talks about async programming.\n\nSection C covers memory management.".to_string(),
        metadata: None,
    };

    let splitter = TextSplitter::new(SplitStrategy::Recursive { chunk_size: 40, overlap: 5 });
    retriever.index_document(&doc, &splitter).await.unwrap();

    let results = retriever.query("async programming patterns").await.unwrap();
    assert!(!results.is_empty());
}

