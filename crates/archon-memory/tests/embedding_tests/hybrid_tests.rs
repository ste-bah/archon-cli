use super::support::{MockProvider, synthetic_embedding};
use super::*;
// hybrid_search: merge logic
// ---------------------------------------------------------------------------

#[test]
fn hybrid_search_with_mock_provider() {
    let g = MemoryGraph::in_memory().expect("graph");
    let dim = 4;

    // Store some memories via MemoryGraph
    let id1 = g
        .store_memory(
            "rust programming language systems",
            "Rust lang",
            MemoryType::Fact,
            0.8,
            &["rust".into(), "programming".into()],
            "test",
            "",
        )
        .expect("store 1");
    let id2 = g
        .store_memory(
            "python scripting language data science",
            "Python lang",
            MemoryType::Fact,
            0.6,
            &["python".into(), "programming".into()],
            "test",
            "",
        )
        .expect("store 2");

    // Init vector schema and store embeddings for both
    vector_search::init_embedding_schema(g.db(), dim).expect("schema");
    vector_search::store_embedding(g.db(), &id1, &synthetic_embedding(dim, 0), "mock", dim)
        .expect("emb 1");
    vector_search::store_embedding(g.db(), &id2, &synthetic_embedding(dim, 1), "mock", dim)
        .expect("emb 2");

    let provider = MockProvider::new(dim);
    let results = hybrid_search::hybrid_search(g.db(), "rust programming", &provider, 0.3, 10)
        .expect("hybrid");

    // Both memories match "programming" keyword; at least one should be returned
    assert!(!results.is_empty());
}

#[test]
fn hybrid_search_alpha_zero_is_pure_vector() {
    let g = MemoryGraph::in_memory().expect("graph");
    let dim = 4;

    let id1 = g
        .store_memory(
            "alpha test content",
            "Alpha",
            MemoryType::Fact,
            0.5,
            &[],
            "t",
            "",
        )
        .expect("s1");

    vector_search::init_embedding_schema(g.db(), dim).expect("schema");
    vector_search::store_embedding(g.db(), &id1, &synthetic_embedding(dim, 0), "mock", dim)
        .expect("emb");

    let provider = MockProvider::new(dim);
    // alpha=0 means keyword weight is 0 → pure vector search
    let results = hybrid_search::hybrid_search(g.db(), "zzz_no_keyword_match", &provider, 0.0, 10)
        .expect("hybrid");
    // Should still find via vector even if keyword doesn't match
    assert!(!results.is_empty());
}

#[test]
fn hybrid_search_alpha_one_is_pure_keyword() {
    let g = MemoryGraph::in_memory().expect("graph");
    let dim = 4;

    let _id1 = g
        .store_memory(
            "keyword searchable content here",
            "KW",
            MemoryType::Fact,
            0.5,
            &[],
            "t",
            "",
        )
        .expect("s1");

    // Do NOT store any embeddings — pure keyword mode (alpha=1.0)
    // We don't even need the vector schema for alpha=1.0

    let provider = MockProvider::new(dim);
    let results = hybrid_search::hybrid_search(g.db(), "keyword searchable", &provider, 1.0, 10)
        .expect("hybrid");
    assert!(!results.is_empty());
}

// ---------------------------------------------------------------------------
