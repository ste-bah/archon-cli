//! Tests for the search module (TASK-PIPE-D04).
//!
//! Validates: empty index search, relevance sorting, threshold filtering,
//! deduplication by file_path, language filter, path_pattern filter,
//! and SearchResult field population.
//!
//! Because Mock embedder returns zero-vectors (causing NaN cosine distance),
//! tests that need actual results use a `ConstantEmbeddingProvider` that
//! returns a fixed non-zero vector for every input.

use std::collections::BTreeMap;
use std::sync::Arc;

use cozo::{DataValue, DbInstance, ScriptMutability, Vector};
use ndarray::Array1;

use archon_leann::indexer::{EmbeddingConfig, EmbeddingProviderKind, Indexer};
use archon_leann::search::{DEFAULT_MIN_SIMILARITY, Search};
use archon_memory::embedding::EmbeddingProvider;
use archon_memory::types::MemoryError;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create an in-memory CozoDB instance.
fn test_db() -> DbInstance {
    DbInstance::new("mem", "", Default::default()).expect("in-memory CozoDB")
}

/// Create an Indexer with Mock embedding (dim=4) and ensure schema exists.
fn setup_indexer(db: DbInstance) -> Indexer {
    let config = EmbeddingConfig {
        provider: EmbeddingProviderKind::Mock,
        dimension: 4,
    };
    let indexer = Indexer::new(db, config, None).expect("indexer creation");
    indexer.ensure_schema().expect("ensure_schema");
    indexer
}

/// An embedding provider that always returns the same fixed non-zero vector.
/// This avoids NaN cosine distance issues with zero-vectors.
struct ConstantEmbeddingProvider {
    vector: Vec<f32>,
}

impl ConstantEmbeddingProvider {
    fn new(vector: Vec<f32>) -> Self {
        Self { vector }
    }
}

impl EmbeddingProvider for ConstantEmbeddingProvider {
    fn embed(&self, texts: &[String]) -> std::result::Result<Vec<Vec<f32>>, MemoryError> {
        Ok(texts.iter().map(|_| self.vector.clone()).collect())
    }
    fn dimensions(&self) -> usize {
        self.vector.len()
    }
}

/// Insert a single chunk directly into CozoDB with the given embedding vector.
fn insert_chunk(
    db: &DbInstance,
    chunk_id: &str,
    file_path: &str,
    language: &str,
    line_start: i64,
    line_end: i64,
    content: &str,
    embedding: Vec<f32>,
) {
    let arr = Array1::from_vec(embedding);
    let mut params = BTreeMap::new();
    params.insert("id".to_string(), DataValue::from(chunk_id));
    params.insert("fp".to_string(), DataValue::from(file_path));
    params.insert("lang".to_string(), DataValue::from(language));
    params.insert("ls".to_string(), DataValue::from(line_start));
    params.insert("le".to_string(), DataValue::from(line_end));
    params.insert("cc".to_string(), DataValue::from(content));
    params.insert("fh".to_string(), DataValue::from("test_hash"));
    params.insert("ts".to_string(), DataValue::from(0.0f64));
    params.insert("emb".to_string(), DataValue::Vec(Vector::F32(arr)));

    db.run_script(
        "?[chunk_id, file_path, language, line_start, line_end, chunk_content, file_hash, indexed_at, embedding] \
         <- [[$id, $fp, $lang, $ls, $le, $cc, $fh, $ts, $emb]]
         :put code_chunks { chunk_id => file_path, language, line_start, line_end, chunk_content, file_hash, indexed_at, embedding }",
        params,
        ScriptMutability::Mutable,
    )
    .expect("insert chunk");
}

/// Build a Search that uses a constant non-zero embedding (matches all
/// inserted chunks that also use the same constant vector).
fn make_nonzero_search(db: DbInstance) -> Search {
    // Use [1, 0, 0, 0] — a valid unit vector for cosine similarity
    let embedder: Arc<dyn EmbeddingProvider> =
        Arc::new(ConstantEmbeddingProvider::new(vec![1.0, 0.0, 0.0, 0.0]));
    Search::new(db, embedder)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

mod search_tests {
    use super::*;

    // 1. search_code on empty index returns empty Vec
    #[test]
    fn search_code_empty_index_returns_empty() {
        let db = test_db();
        let indexer = setup_indexer(db.clone());
        // Use non-zero embedder so the query itself is valid
        let search = make_nonzero_search(db);
        let _ = indexer; // schema must be set up before searching

        let results = search.search_code("fn main", 10).expect("search_code");
        assert!(
            results.is_empty(),
            "empty index should return empty results"
        );
    }

    // 2. search_code returns results sorted by relevance_score descending.
    //
    // We insert chunks with different embeddings relative to the query vector [1,0,0,0].
    // - chunk-a: [1,0,0,0] — distance 0, relevance 1.0 (identical)
    // - chunk-b: [0,1,0,0] — distance 1.0 (orthogonal), relevance 0.5
    //
    // The query also uses [1,0,0,0] (ConstantEmbeddingProvider), so chunk-a should rank higher.
    #[test]
    fn search_code_results_sorted_by_relevance_descending() {
        let db = test_db();
        let indexer = setup_indexer(db.clone());

        // Identical to query: relevance = 1.0
        insert_chunk(
            &db,
            "chunk-a",
            "/a.rs",
            "rust",
            1,
            5,
            "fn a() {}",
            vec![1.0, 0.0, 0.0, 0.0],
        );
        // Orthogonal to query: relevance = 0.5
        insert_chunk(
            &db,
            "chunk-b",
            "/b.rs",
            "rust",
            1,
            5,
            "fn b() {}",
            vec![0.0, 1.0, 0.0, 0.0],
        );
        let _ = indexer;

        let search = make_nonzero_search(db);
        let results = search.search_code("query text", 10).expect("search_code");
        assert!(!results.is_empty(), "should find results");

        // Check sorted descending
        for window in results.windows(2) {
            assert!(
                window[0].relevance_score >= window[1].relevance_score,
                "results should be sorted descending: {} >= {}",
                window[0].relevance_score,
                window[1].relevance_score
            );
        }

        // First result should be the identical-vector chunk
        assert!(
            results[0].relevance_score > 0.9,
            "top result should have high relevance, got {}",
            results[0].relevance_score
        );
    }

    // 3. search_code filters results below 0.3 threshold.
    //
    // Insert a chunk whose embedding is the near-opposite of the query.
    // Opposite vector: [-1, 0, 0, 0] — cosine distance = 2.0, relevance = 0.0.
    // Also insert a chunk with [1,0,0,0] (identical, relevance = 1.0) to ensure
    // the search returns at least something to validate threshold is working.
    #[test]
    fn search_code_threshold_filters_low_relevance() {
        let db = test_db();
        let indexer = setup_indexer(db.clone());

        // Near-opposite: relevance ≈ 0.0
        insert_chunk(
            &db,
            "opposite",
            "/opp.rs",
            "rust",
            1,
            5,
            "fn opposite() {}",
            vec![-1.0, 0.0, 0.0, 0.0],
        );
        // Identical: relevance = 1.0
        insert_chunk(
            &db,
            "identical",
            "/same.rs",
            "rust",
            1,
            5,
            "fn identical() {}",
            vec![1.0, 0.0, 0.0, 0.0],
        );
        let _ = indexer;

        let search = make_nonzero_search(db);
        let results = search.search_code("query", 10).expect("search_code");

        // All returned results must meet the threshold
        for r in &results {
            assert!(
                r.relevance_score >= DEFAULT_MIN_SIMILARITY,
                "relevance_score {} should be >= threshold {}",
                r.relevance_score,
                DEFAULT_MIN_SIMILARITY
            );
        }

        // The opposite chunk (relevance ≈ 0.0) should be excluded
        let has_opposite = results
            .iter()
            .any(|r| r.file_path.to_string_lossy().contains("/opp.rs"));
        assert!(
            !has_opposite,
            "opposite-vector chunk should be filtered out by threshold"
        );
    }

    // 4. find_similar_code deduplicates by file_path (keeps highest score per file).
    //
    // Insert two chunks for the same file path. After dedup we should see exactly
    // one result per unique file path.
    #[test]
    fn find_similar_code_deduplicates_by_file_path() {
        let db = test_db();
        let indexer = setup_indexer(db.clone());

        // Two chunks for the same file
        insert_chunk(
            &db,
            "dup-1",
            "/dup.rs",
            "rust",
            1,
            5,
            "fn first() {}",
            vec![1.0, 0.0, 0.0, 0.0],
        );
        insert_chunk(
            &db,
            "dup-2",
            "/dup.rs",
            "rust",
            10,
            15,
            "fn second() {}",
            vec![1.0, 0.0, 0.0, 0.0],
        );
        // One chunk for a different file
        insert_chunk(
            &db,
            "other-1",
            "/other.rs",
            "rust",
            1,
            5,
            "fn other() {}",
            vec![1.0, 0.0, 0.0, 0.0],
        );
        let _ = indexer;

        let search = make_nonzero_search(db);
        let results = search
            .find_similar_code("fn example() {}", 10)
            .expect("find_similar_code");

        // All file paths should be unique
        let mut paths: Vec<String> = results
            .iter()
            .map(|r| r.file_path.to_string_lossy().to_string())
            .collect();
        paths.sort();
        paths.dedup();

        let unique_count = paths.len();
        let result_count = results.len();
        assert_eq!(
            unique_count, result_count,
            "find_similar_code should deduplicate by file_path: got {} results for {} unique paths",
            result_count, unique_count
        );
        assert!(result_count > 0, "should find at least one result");
    }

    // 5. search_with_filter with language filter only returns matching language chunks.
    #[test]
    fn search_with_filter_language_filter_works() {
        let db = test_db();
        let indexer = setup_indexer(db.clone());

        insert_chunk(
            &db,
            "rust-1",
            "/a.rs",
            "rust",
            1,
            5,
            "fn rust_fn() {}",
            vec![1.0, 0.0, 0.0, 0.0],
        );
        insert_chunk(
            &db,
            "py-1",
            "/b.py",
            "python",
            1,
            5,
            "def py_fn(): pass",
            vec![1.0, 0.0, 0.0, 0.0],
        );
        let _ = indexer;

        let search = make_nonzero_search(db);
        let results = search
            .search_with_filter("query", 10, Some("rust"), None)
            .expect("search_with_filter");

        assert!(!results.is_empty(), "should find at least one rust chunk");

        for r in &results {
            assert_eq!(
                r.language, "rust",
                "language filter should only return rust chunks, got: {}",
                r.language
            );
        }
    }

    // 6. search_with_filter with path_pattern filter works.
    #[test]
    fn search_with_filter_path_pattern_works() {
        let db = test_db();
        let indexer = setup_indexer(db.clone());

        insert_chunk(
            &db,
            "src-1",
            "/project/src/main.rs",
            "rust",
            1,
            5,
            "fn main() {}",
            vec![1.0, 0.0, 0.0, 0.0],
        );
        insert_chunk(
            &db,
            "test-1",
            "/project/tests/foo_test.rs",
            "rust",
            1,
            5,
            "fn test_foo() {}",
            vec![1.0, 0.0, 0.0, 0.0],
        );
        let _ = indexer;

        let search = make_nonzero_search(db);
        let results = search
            .search_with_filter("query", 10, None, Some("/project/src"))
            .expect("search_with_filter");

        assert!(
            !results.is_empty(),
            "should find at least one /project/src chunk"
        );

        for r in &results {
            let path_str = r.file_path.to_string_lossy();
            assert!(
                path_str.contains("/project/src"),
                "path_pattern filter should only return matching paths, got: {}",
                path_str
            );
        }
    }

    // 7. All SearchResult fields populated correctly.
    #[test]
    fn search_result_fields_populated() {
        let db = test_db();
        let indexer = setup_indexer(db.clone());

        insert_chunk(
            &db,
            "fields-1",
            "/src/lib.rs",
            "rust",
            10,
            20,
            "pub fn hello() -> &'static str { \"hello\" }",
            vec![1.0, 0.0, 0.0, 0.0],
        );
        let _ = indexer;

        let search = make_nonzero_search(db);
        let results = search
            .search_code("hello function", 5)
            .expect("search_code");
        assert!(!results.is_empty(), "should find inserted chunk");

        let r = &results[0];
        assert!(
            !r.file_path.as_os_str().is_empty(),
            "file_path should be populated"
        );
        assert!(!r.content.is_empty(), "content should be populated");
        assert!(!r.language.is_empty(), "language should be populated");
        assert!(
            r.line_end >= r.line_start,
            "line_end should be >= line_start"
        );
        assert!(
            r.relevance_score >= 0.0 && r.relevance_score <= 1.0,
            "relevance_score should be in [0.0, 1.0], got {}",
            r.relevance_score
        );
    }
}
