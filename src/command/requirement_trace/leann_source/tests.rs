//! The real adapter against fixture chunks.
//!
//! The index is built *by the test, explicitly*, which is the out-of-band step
//! the report itself must never take: `ensure_schema` plus one raw `:put` of a
//! known vector. No file is chunked, no repository is walked, and the embedder
//! is a constant — so this runs in milliseconds and never touches a network or
//! downloads a model, while still driving the real `search_with_filter` and the
//! real `SearchResult` → [`CodeHit`] mapping.
//!
//! The constant vector is deliberate, but no longer because of zero vectors —
//! the built-in Mock provider now derives a unit vector per text (#145). These
//! tests need a query vector *they chose*, so that the expected neighbour is
//! known in advance rather than being whatever the query string happens to hash
//! to.

use std::collections::BTreeMap;
use std::sync::Arc;

use archon_leann::indexer::{EmbeddingConfig, EmbeddingProviderKind, Indexer};
use archon_memory::embedding::EmbeddingProvider;
use archon_memory::types::MemoryError;
use cozo::{DataValue, DbInstance, ScriptMutability};

use super::*;

const DIM: usize = 4;

struct ConstantEmbedder;

impl EmbeddingProvider for ConstantEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError> {
        Ok(texts.iter().map(|_| vec![1.0, 0.0, 0.0, 0.0]).collect())
    }

    fn dimensions(&self) -> usize {
        DIM
    }
}

/// An index with the schema created and two chunks in it.
fn fixture_index() -> (tempfile::TempDir, DbInstance) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("leann.db");
    let db =
        DbInstance::new("sqlite", db_path.to_string_lossy().as_ref(), "").expect("sqlite cozo");
    let indexer = Indexer::new(
        db.clone(),
        EmbeddingConfig {
            provider: EmbeddingProviderKind::Mock,
            dimension: DIM,
        },
        None,
    )
    .expect("indexer");
    indexer.ensure_schema().expect("ensure_schema");

    put_chunk(&db, "c1", "crates/archon-trading/src/data_lake.rs", 10, 42);
    put_chunk(&db, "c2", "src/command/trading_data.rs", 5, 9);
    (dir, db)
}

fn put_chunk(db: &DbInstance, id: &str, path: &str, start: i64, end: i64) {
    let mut params: BTreeMap<String, DataValue> = BTreeMap::new();
    params.insert("id".into(), DataValue::from(id));
    params.insert("fp".into(), DataValue::from(path));
    params.insert("ls".into(), DataValue::from(start));
    params.insert("le".into(), DataValue::from(end));
    params.insert(
        "emb".into(),
        DataValue::List(vec![
            DataValue::from(1.0),
            DataValue::from(0.0),
            DataValue::from(0.0),
            DataValue::from(0.0),
        ]),
    );
    db.run_script(
        r#"
        ?[chunk_id, file_path, language, line_start, line_end, chunk_content, file_hash,
          indexed_at, embedding]
            <- [[$id, $fp, "rust", $ls, $le, "fn anything() {}", "deadbeef", 0.0, $emb]]
        :put code_chunks {
            chunk_id => file_path, language, line_start, line_end, chunk_content, file_hash,
            indexed_at, embedding
        }
        "#,
        params,
        ScriptMutability::Mutable,
    )
    .expect("put chunk");
}

#[test]
fn the_adapter_maps_a_real_search_result_onto_a_code_hit() {
    let (_dir, db) = fixture_index();
    let source = LeannCodeSearch::with_embedder(db, Arc::new(ConstantEmbedder));

    let hits = source
        .search("ingest polygon natively", 5, None)
        .expect("search");
    assert_eq!(hits.len(), 2);
    let lake = hits
        .iter()
        .find(|h| h.file_path == "crates/archon-trading/src/data_lake.rs")
        .expect("data_lake hit");
    assert_eq!(lake.line_start, 10);
    assert_eq!(lake.line_end, 42);
    assert_eq!(lake.language, "rust");
    // Carried, never consulted.
    assert!(lake.relevance_score.is_finite());
}

#[test]
fn the_path_pattern_reaches_the_real_index_filter() {
    let (_dir, db) = fixture_index();
    let source = LeannCodeSearch::with_embedder(db, Arc::new(ConstantEmbedder));

    let scoped = source
        .search("ingest", 5, Some("src/command/trading_data.rs"))
        .expect("search");
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].file_path, "src/command/trading_data.rs");

    assert!(
        source
            .search("ingest", 5, Some("no/such/path.rs"))
            .expect("search")
            .is_empty()
    );
}

#[test]
fn opening_a_path_with_no_index_names_the_out_of_band_constraint_and_writes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let absent = dir.path().join("leann.db");
    // `LeannCodeSearch` holds a `Search`, which is not `Debug`, so unwrap the
    // error by hand rather than through `expect_err`.
    let message = match LeannCodeSearch::open(&absent, Default::default()) {
        Ok(_) => panic!("opening a missing index must be refused"),
        Err(err) => err.to_string(),
    };
    assert!(message.contains("no code index"), "{message}");
    assert!(message.contains("out of band"), "{message}");
    // The refusal must not have created the database it was asked to read.
    assert!(!absent.exists(), "opening a missing index created it");
}
