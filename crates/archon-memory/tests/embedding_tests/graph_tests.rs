use super::support::{MismatchedProvider, MockProvider, synthetic_embedding};
use super::*;
// MemoryGraph integration: set_embedding_provider
// ---------------------------------------------------------------------------

#[test]
fn graph_with_provider_stores_embeddings_on_store() {
    let g = MemoryGraph::in_memory().expect("graph");
    let provider = std::sync::Arc::new(MockProvider::new(4));
    g.set_embedding_provider(provider.clone())
        .expect("set provider");

    let _id = g
        .store_memory(
            "enough content to embed",
            "Test",
            MemoryType::Fact,
            0.5,
            &[],
            "test",
            "",
        )
        .expect("store");

    // Verify embedding was stored via search_similar
    let query = synthetic_embedding(4, 0);
    let results = vector_search::search_similar(g.db(), &query, 10).expect("search");
    assert_eq!(results.len(), 1);
}

#[test]
fn graph_store_memory_degrades_when_embedding_store_fails() {
    let g = MemoryGraph::in_memory().expect("graph");
    let provider = std::sync::Arc::new(MismatchedProvider {
        declared_dim: 4,
        actual_dim: 3,
    });
    g.set_embedding_provider(provider).expect("set provider");

    let id = g
        .store_memory(
            "enough content to trigger embedding",
            "Bad embedding",
            MemoryType::Fact,
            0.5,
            &[],
            "test",
            "",
        )
        .expect("memory row should persist even when embedding indexing fails");

    let stored = g.get_memory(&id).expect("memory remains readable");
    assert_eq!(stored.title, "Bad embedding");
    assert_eq!(
        g.memory_count().expect("count"),
        1,
        "embedding failure must not delete the authoritative memory row"
    );
}

#[test]
fn graph_skips_embedding_for_short_text() {
    let g = MemoryGraph::in_memory().expect("graph");
    let provider = std::sync::Arc::new(MockProvider::new(4));
    g.set_embedding_provider(provider.clone())
        .expect("set provider");

    // Text < 10 chars should be skipped
    let _id = g
        .store_memory("short", "S", MemoryType::Fact, 0.5, &[], "test", "")
        .expect("store");

    // Verify no embedding was stored via search_similar returning empty
    let query = synthetic_embedding(4, 0);
    let results = vector_search::search_similar(g.db(), &query, 10).expect("search");
    assert!(results.is_empty(), "short text should not be embedded");
}

#[test]
fn graph_recall_uses_hybrid_when_provider_set() {
    let g = MemoryGraph::in_memory().expect("graph");
    let provider = std::sync::Arc::new(MockProvider::new(4));
    g.set_embedding_provider(provider.clone())
        .expect("set provider");

    g.store_memory(
        "rust programming language systems",
        "Rust",
        MemoryType::Fact,
        0.8,
        &["rust".into()],
        "test",
        "",
    )
    .expect("store");

    let results = g.recall_memories("rust", 10).expect("recall");
    assert!(!results.is_empty());
}

#[test]
fn graph_recall_works_without_provider() {
    let g = MemoryGraph::in_memory().expect("graph");

    g.store_memory(
        "fallback keyword only search",
        "Fallback",
        MemoryType::Fact,
        0.5,
        &[],
        "test",
        "",
    )
    .expect("store");

    let results = g.recall_memories("fallback", 10).expect("recall");
    assert!(!results.is_empty());
}

#[test]
fn graph_delete_removes_embedding_too() {
    let g = MemoryGraph::in_memory().expect("graph");
    let provider = std::sync::Arc::new(MockProvider::new(4));
    g.set_embedding_provider(provider.clone())
        .expect("set provider");

    let id = g
        .store_memory(
            "content to be deleted later",
            "Delete me",
            MemoryType::Fact,
            0.5,
            &[],
            "test",
            "",
        )
        .expect("store");

    // Verify embedding exists
    let query = synthetic_embedding(4, 0);
    let before = vector_search::search_similar(g.db(), &query, 10).expect("search");
    assert_eq!(before.len(), 1);

    g.delete_memory(&id).expect("delete");

    // Verify embedding was removed
    let after = vector_search::search_similar(g.db(), &query, 10).expect("search");
    assert!(after.is_empty());
}

// ---------------------------------------------------------------------------
