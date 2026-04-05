//! Integration tests for the embedding / vector search / hybrid search subsystem.
//!
//! These tests use CozoDB in-memory backend and synthetic embeddings so they
//! run without network access or model downloads.

use archon_memory::embedding::{
    EmbeddingConfig, EmbeddingProvider, EmbeddingProviderKind, create_provider,
};
use archon_memory::graph::MemoryGraph;
use archon_memory::hybrid_search;
use archon_memory::types::{MemoryError, MemoryType};
use archon_memory::vector_search;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Deterministic pseudo-embedding (not random, reproducible).
fn synthetic_embedding(dim: usize, seed: usize) -> Vec<f32> {
    (0..dim).map(|i| ((i + seed) as f32 * 0.1).sin()).collect()
}

/// A trivial provider that returns fixed-dimension synthetic embeddings.
struct MockProvider {
    dim: usize,
}

impl MockProvider {
    fn new(dim: usize) -> Self {
        Self { dim }
    }
}

impl EmbeddingProvider for MockProvider {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError> {
        Ok(texts
            .iter()
            .enumerate()
            .map(|(i, _)| synthetic_embedding(self.dim, i))
            .collect())
    }

    fn dimensions(&self) -> usize {
        self.dim
    }
}

// ---------------------------------------------------------------------------
// EmbeddingProvider trait contract
// ---------------------------------------------------------------------------

#[test]
fn provider_trait_embed_returns_correct_count() {
    let provider = MockProvider::new(768);
    let texts = vec!["hello".into(), "world".into(), "foo".into()];
    let vecs = provider.embed(&texts).expect("embed should succeed");
    assert_eq!(vecs.len(), 3);
}

#[test]
fn provider_trait_dimensions_match_vectors() {
    let provider = MockProvider::new(768);
    let texts = vec!["test".into()];
    let vecs = provider.embed(&texts).expect("embed should succeed");
    assert_eq!(vecs[0].len(), provider.dimensions());
}

#[test]
fn provider_trait_empty_input_returns_empty() {
    let provider = MockProvider::new(768);
    let vecs = provider.embed(&[]).expect("embed should succeed");
    assert!(vecs.is_empty());
}

// ---------------------------------------------------------------------------
// EmbeddingConfig + factory
// ---------------------------------------------------------------------------

#[test]
fn config_default_values() {
    let cfg = EmbeddingConfig::default();
    assert_eq!(cfg.provider, EmbeddingProviderKind::Auto);
    assert!((cfg.hybrid_alpha - 0.3).abs() < f32::EPSILON);
}

#[test]
fn factory_creates_local_when_no_api_key() {
    // Ensure OPENAI_API_KEY and ARCHON_MEMORY_OPENAIKEY are NOT set for this test
    // (the factory should fall back to local when keys are absent).
    // We only verify the factory doesn't error; actual model loading is lazy.
    // SAFETY: tests run single-threaded via --test-threads=1 or in isolation.
    unsafe {
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("ARCHON_MEMORY_OPENAIKEY");
    }
    let cfg = EmbeddingConfig {
        provider: EmbeddingProviderKind::Auto,
        hybrid_alpha: 0.3,
    };
    let provider = create_provider(&cfg).expect("factory should succeed for local");
    // Local provider returns 768 dimensions
    assert_eq!(provider.dimensions(), 768);
}

#[test]
fn factory_rejects_openai_without_key() {
    // SAFETY: tests run single-threaded via --test-threads=1 or in isolation.
    unsafe {
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("ARCHON_MEMORY_OPENAIKEY");
    }
    let cfg = EmbeddingConfig {
        provider: EmbeddingProviderKind::OpenAI,
        hybrid_alpha: 0.3,
    };
    let result = create_provider(&cfg);
    assert!(result.is_err(), "openai provider without key should fail");
}

// ---------------------------------------------------------------------------
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

// ---------------------------------------------------------------------------
// vector_search: store + count
// ---------------------------------------------------------------------------

#[test]
fn store_and_count_embeddings() {
    let g = MemoryGraph::in_memory().expect("graph");
    vector_search::init_embedding_schema(g.db(), 4).expect("schema");

    let emb = synthetic_embedding(4, 0);
    vector_search::store_embedding(g.db(), "mem-1", &emb, "mock", 4).expect("store 1");
    vector_search::store_embedding(g.db(), "mem-2", &synthetic_embedding(4, 1), "mock", 4)
        .expect("store 2");

    let count = vector_search::embedding_count(g.db()).expect("count");
    assert_eq!(count, 2);
}

// ---------------------------------------------------------------------------
// vector_search: delete
// ---------------------------------------------------------------------------

#[test]
fn delete_embedding_removes_row() {
    let g = MemoryGraph::in_memory().expect("graph");
    vector_search::init_embedding_schema(g.db(), 4).expect("schema");

    vector_search::store_embedding(g.db(), "mem-del", &synthetic_embedding(4, 0), "mock", 4)
        .expect("store");
    assert_eq!(vector_search::embedding_count(g.db()).expect("c"), 1);

    vector_search::delete_embedding(g.db(), "mem-del").expect("delete");
    assert_eq!(vector_search::embedding_count(g.db()).expect("c"), 0);
}

#[test]
fn delete_nonexistent_embedding_is_ok() {
    let g = MemoryGraph::in_memory().expect("graph");
    vector_search::init_embedding_schema(g.db(), 4).expect("schema");
    // Should not error even if no row exists
    vector_search::delete_embedding(g.db(), "does-not-exist").expect("delete noop");
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
// vector_search: drop_embeddings
// ---------------------------------------------------------------------------

#[test]
fn drop_embeddings_clears_everything() {
    let g = MemoryGraph::in_memory().expect("graph");
    let dim = 4;
    vector_search::init_embedding_schema(g.db(), dim).expect("schema");
    vector_search::store_embedding(g.db(), "x", &synthetic_embedding(dim, 0), "mock", dim)
        .expect("store");

    vector_search::drop_embeddings(g.db()).expect("drop");
    // After drop, re-init should work and count should be 0
    vector_search::init_embedding_schema(g.db(), dim).expect("re-init");
    assert_eq!(vector_search::embedding_count(g.db()).expect("c"), 0);
}

// ---------------------------------------------------------------------------
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

    let count = vector_search::embedding_count(g.db()).expect("count");
    assert_eq!(count, 1);
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

    let count = vector_search::embedding_count(g.db()).expect("count");
    assert_eq!(count, 0, "short text should not be embedded");
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

    assert_eq!(vector_search::embedding_count(g.db()).expect("c"), 1);
    g.delete_memory(&id).expect("delete");
    assert_eq!(vector_search::embedding_count(g.db()).expect("c"), 0);
}

// ---------------------------------------------------------------------------
// OpenAI provider: error handling (no HTTP mocking, just unit-level)
// ---------------------------------------------------------------------------

#[test]
fn openai_provider_requires_api_key() {
    use archon_memory::embedding::openai::OpenAIEmbedding;
    // Empty key should fail on construction
    let result = OpenAIEmbedding::new("");
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// EmbeddingConfig defaults (archon-core config test lives in archon-core)
// ---------------------------------------------------------------------------

#[test]
fn embedding_config_serde_round_trip() {
    let cfg = EmbeddingConfig {
        provider: EmbeddingProviderKind::Local,
        hybrid_alpha: 0.5,
    };
    let json = serde_json::to_string(&cfg).expect("serialize");
    let parsed: EmbeddingConfig = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed.provider, EmbeddingProviderKind::Local);
    assert!((parsed.hybrid_alpha - 0.5).abs() < f32::EPSILON);
}
