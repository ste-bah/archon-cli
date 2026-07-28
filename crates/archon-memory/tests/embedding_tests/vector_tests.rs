use super::support::{MismatchedProvider, MockProvider, synthetic_embedding};
use super::*;
// vector_search: schema init
// ---------------------------------------------------------------------------

#[test]
fn init_embedding_schema_succeeds() {
    let g = MemoryGraph::in_memory().expect("graph");
    vector_search::init_embedding_schema(g.db(), 768).expect("schema init");
}

#[test]
fn init_embedding_schema_idempotent() {
    let g = MemoryGraph::in_memory().expect("graph");
    vector_search::init_embedding_schema(g.db(), 768).expect("first");
    vector_search::init_embedding_schema(g.db(), 768).expect("second should not error");
}

#[test]
fn init_embedding_schema_rebuilds_when_dimension_changes() {
    let g = MemoryGraph::in_memory().expect("graph");
    vector_search::init_embedding_schema(g.db(), 4).expect("initial schema");
    vector_search::store_embedding(g.db(), "old-memory", &synthetic_embedding(4, 0), "mock", 4)
        .expect("store old embedding");

    vector_search::init_embedding_schema(g.db(), 6).expect("rebuild schema for new dimension");
    vector_search::store_embedding(g.db(), "new-memory", &synthetic_embedding(6, 1), "mock", 6)
        .expect("store new embedding after rebuild");

    let results =
        vector_search::search_similar(g.db(), &synthetic_embedding(6, 1), 10).expect("search");
    assert!(results.iter().any(|(id, _)| id == "new-memory"));
    assert!(!results.iter().any(|(id, _)| id == "old-memory"));
}

// ---------------------------------------------------------------------------
// vector_search: store + delete
// ---------------------------------------------------------------------------

#[test]
fn store_and_delete_embeddings() {
    let g = MemoryGraph::in_memory().expect("graph");
    vector_search::init_embedding_schema(g.db(), 4).expect("schema");

    let emb = synthetic_embedding(4, 0);
    vector_search::store_embedding(g.db(), "mem-1", &emb, "mock", 4).expect("store 1");
    vector_search::store_embedding(g.db(), "mem-2", &synthetic_embedding(4, 1), "mock", 4)
        .expect("store 2");

    // Verify stored via search_similar (should find them)
    let results = vector_search::search_similar(g.db(), &emb, 10).expect("search");
    assert_eq!(results.len(), 2);

    // Delete one and verify via search
    vector_search::delete_embedding(g.db(), "mem-1").expect("delete");
    let results = vector_search::search_similar(g.db(), &emb, 10).expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "mem-2");
}

#[test]
fn store_embedding_rejects_dimension_mismatch_before_cozo_write() {
    let g = MemoryGraph::in_memory().expect("graph");
    vector_search::init_embedding_schema(g.db(), 4).expect("schema");

    let err = vector_search::store_embedding(g.db(), "memory-1", &[0.1, 0.2, 0.3], "mock", 4)
        .expect_err("mismatched vectors must be rejected before Cozo write");

    assert!(err.to_string().contains("dimension mismatch"), "{err}");
}

#[test]
fn delete_nonexistent_embedding_is_ok() {
    let g = MemoryGraph::in_memory().expect("graph");
    vector_search::init_embedding_schema(g.db(), 4).expect("schema");
    // Should not error even if no row exists
    vector_search::delete_embedding(g.db(), "does-not-exist").expect("delete noop");
}

#[test]
fn store_memory_with_id_indexes_new_and_matching_existing_memory() {
    let g = MemoryGraph::in_memory().expect("graph");
    let dim = 4;
    g.set_embedding_provider(Arc::new(MockProvider::new(dim)))
        .expect("provider attaches");
    let content = "explicit ID content long enough to trigger embedding storage";

    let first = g
        .store_memory_with_id(
            "memory:explicit-embedding",
            content,
            "explicit embedding",
            MemoryType::Fact,
            0.7,
            &["pipeline".into()],
            "test",
            "",
        )
        .expect("create explicit memory");
    let second = g
        .store_memory_with_id(
            "memory:explicit-embedding",
            content,
            "explicit embedding",
            MemoryType::Fact,
            0.7,
            &["pipeline".into()],
            "test",
            "",
        )
        .expect("matching explicit retry");

    assert_eq!(first.id, second.id);
    let results = vector_search::search_similar(g.db(), &synthetic_embedding(dim, 0), 10)
        .expect("search stored embedding");
    assert!(
        results
            .iter()
            .any(|(id, _)| id == "memory:explicit-embedding"),
        "both first write and matching retry must ensure the explicit-ID row is indexed"
    );
}

#[test]
fn store_memory_keeps_row_when_embedding_indexing_fails() {
    let g = MemoryGraph::in_memory().expect("graph");
    g.set_embedding_provider(Arc::new(MismatchedProvider {
        declared_dim: 4,
        actual_dim: 3,
    }))
    .expect("provider attaches");

    let id = g
        .store_memory(
            "long enough content to trigger embedding",
            "survives embedding failure",
            MemoryType::Fact,
            0.7,
            &["pipeline".into()],
            "test",
            "",
        )
        .expect("memory text should survive embedding failure");

    let stored = g.get_memory(&id).expect("stored memory remains readable");
    assert_eq!(stored.title, "survives embedding failure");
    assert_eq!(stored.content, "long enough content to trigger embedding");
}

#[test]
fn update_memory_keeps_new_content_when_embedding_indexing_fails() {
    let g = MemoryGraph::in_memory().expect("graph");
    g.set_embedding_provider(Arc::new(MockProvider::new(4)))
        .expect("provider attaches");
    let id = g
        .store_memory(
            "initial long content for embedding",
            "update survives embedding failure",
            MemoryType::Fact,
            0.7,
            &["pipeline".into()],
            "test",
            "",
        )
        .expect("initial store");

    g.set_embedding_provider(Arc::new(MismatchedProvider {
        declared_dim: 4,
        actual_dim: 3,
    }))
    .expect("mismatched provider attaches");
    g.update_memory(&id, Some("updated long content still persists"), None)
        .expect("update should not fail just because embedding indexing fails");

    let stored = g.get_memory(&id).expect("stored memory remains readable");
    assert_eq!(stored.content, "updated long content still persists");
}

// ---------------------------------------------------------------------------
// vector_search: search_similar
// ---------------------------------------------------------------------------

#[test]
fn search_similar_returns_nearest() {
    let g = MemoryGraph::in_memory().expect("graph");
    let dim = 4;
    vector_search::init_embedding_schema(g.db(), dim).expect("schema");

    // Store three embeddings
    for i in 0..3 {
        let id = format!("mem-{i}");
        vector_search::store_embedding(g.db(), &id, &synthetic_embedding(dim, i), "mock", dim)
            .expect("store");
    }

    // Query with embedding identical to mem-0
    let query = synthetic_embedding(dim, 0);
    let results = vector_search::search_similar(g.db(), &query, 2).expect("search");
    assert!(!results.is_empty(), "should return at least one result");
    // The closest match should be mem-0 (distance ≈ 0)
    assert_eq!(results[0].0, "mem-0");
}

#[test]
fn search_similar_respects_top_k() {
    let g = MemoryGraph::in_memory().expect("graph");
    let dim = 4;
    vector_search::init_embedding_schema(g.db(), dim).expect("schema");

    for i in 0..10 {
        let id = format!("mem-{i}");
        vector_search::store_embedding(g.db(), &id, &synthetic_embedding(dim, i), "mock", dim)
            .expect("store");
    }

    let query = synthetic_embedding(dim, 0);
    let results = vector_search::search_similar(g.db(), &query, 3).expect("search");
    assert!(results.len() <= 3);
}

#[test]
fn search_similar_empty_db_returns_empty() {
    let g = MemoryGraph::in_memory().expect("graph");
    let dim = 4;
    vector_search::init_embedding_schema(g.db(), dim).expect("schema");

    let query = synthetic_embedding(dim, 0);
    let results = vector_search::search_similar(g.db(), &query, 5).expect("search");
    assert!(results.is_empty());
}

// ---------------------------------------------------------------------------
