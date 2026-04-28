use super::*;
use serde_json::json;

#[tokio::test]
async fn test_memory_vector_store_crud() {
    let store = MemoryVectorStore::new();
    let collection = "test_collection";

    // Upsert
    store
        .upsert(
            collection,
            "doc1",
            vec![1.0, 0.0, 0.0],
            Some(json!({"title": "hello"})),
        )
        .await
        .unwrap();

    // Search
    let results = store.search(collection, vec![1.0, 0.0, 0.0], 1).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "doc1");
    assert!(results[0].score > 0.99);

    // Delete
    store.delete(collection, "doc1").await.unwrap();
    let results = store.search(collection, vec![1.0, 0.0, 0.0], 1).await.unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_memory_vector_store_cosine_search() {
    let store = MemoryVectorStore::new();
    let collection = "cosine_test";

    store
        .upsert(collection, "a", vec![1.0, 0.0, 0.0], None)
        .await
        .unwrap();
    store
        .upsert(collection, "b", vec![0.0, 1.0, 0.0], None)
        .await
        .unwrap();
    store
        .upsert(collection, "c", vec![0.5, 0.5, 0.0], None)
        .await
        .unwrap();

    // Query matches "a" best
    let results = store.search(collection, vec![1.0, 0.0, 0.0], 2).await.unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].id, "a");
    assert!(results[0].score > results[1].score);

    // Query matches "c" best (equal components)
    let results = store.search(collection, vec![1.0, 1.0, 0.0], 1).await.unwrap();
    assert_eq!(results[0].id, "c");
}

#[tokio::test]
async fn test_memory_vector_store_delete() {
    let store = MemoryVectorStore::new();
    let collection = "delete_test";

    store
        .upsert(collection, "x", vec![1.0, 0.0], None)
        .await
        .unwrap();
    store
        .upsert(collection, "y", vec![0.0, 1.0], None)
        .await
        .unwrap();

    // Delete one
    store.delete(collection, "x").await.unwrap();
    let results = store.search(collection, vec![1.0, 0.0], 10).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "y");

    // Delete from empty/nonexistent collection does not panic
    store.delete("no_such_collection", "zzz").await.unwrap();
}

#[tokio::test]
async fn test_vector_store_registry() {
    let registry = VectorStoreRegistry::new();

    // Initially empty
    assert!(registry.default().is_none());

    // Register a store
    let store = Arc::new(MemoryVectorStore::new());
    registry.register("memory", store.clone());

    // First registered becomes default
    let default = registry.default().unwrap();
    default
        .upsert("reg_test", "r1", vec![1.0, 0.0], None)
        .await
        .unwrap();

    // Get by name
    let got = registry.get("memory").unwrap();
    let results = got.search("reg_test", vec![1.0, 0.0], 1).await.unwrap();
    assert_eq!(results[0].id, "r1");

    // Set default
    registry.register("pg", store.clone());
    registry.set_default("pg");
    let default_name = registry.default().unwrap();
    default_name
        .upsert("reg_test2", "r2", vec![0.0, 1.0], None)
        .await
        .unwrap();

    // List
    let names = registry.list();
    assert!(names.contains(&"memory".to_string()));
    assert!(names.contains(&"pg".to_string()));
}

#[tokio::test]
async fn test_search_result_ordering() {
    let store = MemoryVectorStore::new();
    let collection = "ordering_test";

    // Insert vectors with varying similarity to query [1,0,0]
    store.upsert(collection, "far", vec![0.0, 1.0, 0.0], None).await.unwrap();
    store.upsert(collection, "close", vec![0.9, 0.1, 0.0], None).await.unwrap();
    store.upsert(collection, "exact", vec![1.0, 0.0, 0.0], None).await.unwrap();
    store.upsert(collection, "mid", vec![0.5, 0.5, 0.0], None).await.unwrap();

    let results = store.search(collection, vec![1.0, 0.0, 0.0], 4).await.unwrap();
    assert_eq!(results.len(), 4);

    // Must be sorted descending by score
    for i in 0..results.len() - 1 {
        assert!(
            results[i].score >= results[i + 1].score,
            "results not sorted: {:?}",
            results
        );
    }

    assert_eq!(results[0].id, "exact");
    assert_eq!(results[1].id, "close");
}

#[tokio::test]
async fn test_memory_vector_store_list_collections() {
    let store = MemoryVectorStore::new();
    store.upsert("coll_a", "id1", vec![1.0], None).await.unwrap();
    store.upsert("coll_b", "id2", vec![1.0], None).await.unwrap();

    let collections = store.list_collections().await.unwrap();
    assert!(collections.contains(&"coll_a".to_string()));
    assert!(collections.contains(&"coll_b".to_string()));
}

#[tokio::test]
async fn test_memory_vector_store_upsert_overwrite() {
    let store = MemoryVectorStore::new();
    let collection = "upsert_test";

    store
        .upsert(collection, "same_id", vec![1.0, 0.0], Some(json!({"v": 1})))
        .await
        .unwrap();
    store
        .upsert(collection, "same_id", vec![0.0, 1.0], Some(json!({"v": 2})))
        .await
        .unwrap();

    let results = store.search(collection, vec![0.0, 1.0], 1).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "same_id");
    // Score should be ~1.0 against [0,1]
    assert!(results[0].score > 0.99);
    assert_eq!(
        results[0].metadata.as_ref().unwrap()["v"],
        2
    );
}
