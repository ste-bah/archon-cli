//! The three real store ports, each driven against a real store.
//!
//! The code-index fixture follows `requirement_trace::leann_source::tests`: the
//! test builds the index explicitly with one raw `:put` and a constant embedder,
//! which is the out-of-band step a recall must never take itself. No network, no
//! model download.

use std::collections::BTreeMap;

use archon_leann::indexer::{EmbeddingConfig, EmbeddingProviderKind, Indexer};
use archon_memory::embedding::EmbeddingProvider;
use archon_memory::types::MemoryError;
use cozo::{DataValue, ScriptMutability};

use super::*;

const DIM: usize = 4;

struct ConstantEmbedder;

impl EmbeddingProvider for ConstantEmbedder {
    fn embed(&self, texts: &[String]) -> std::result::Result<Vec<Vec<f32>>, MemoryError> {
        Ok(texts.iter().map(|_| vec![1.0, 0.0, 0.0, 0.0]).collect())
    }

    fn dimensions(&self) -> usize {
        DIM
    }
}

#[test]
fn memory_records_carry_id_content_and_project_scope() {
    let graph = archon_memory::MemoryGraph::in_memory().expect("in-memory graph");
    graph
        .store_memory(
            "Audit log retention is thirty days.",
            "audit retention",
            archon_memory::MemoryType::Fact,
            0.8,
            &["audit".to_string()],
            "test",
            "F:/repo",
        )
        .expect("store memory");

    let records = MemoryStore::new(graph)
        .search("audit retention", 5)
        .expect("memory search");

    assert_eq!(records.len(), 1);
    assert!(records[0].content.contains("thirty days"));
    assert_eq!(records[0].container.as_deref(), Some("F:/repo"));
    // No score: `recall_memories` does not expose one, which is exactly why the
    // unified score is rank-derived rather than fused.
    assert_eq!(records[0].score, None);
    assert!(records[0].created_at.is_some());
}

#[test]
fn docs_records_carry_chunk_document_and_score() {
    let db = DbInstance::new("mem", "", "").expect("cozo");
    archon_docs::schema::ensure_doc_schema(&db).expect("doc schema");
    archon_docs::store::insert_chunk(
        &db,
        &archon_docs::models::ChunkArtifact {
            chunk_id: "c1".into(),
            document_id: "doc-1".into(),
            artifact_id: "artifact-c1".into(),
            chunk_index: 0,
            page_start: 1,
            page_end: 1,
            content: "Audit log retention is thirty days.".into(),
            content_hash: "hash-c1".into(),
            embedding_status: "pending".into(),
        },
    )
    .expect("insert chunk");

    let records = DocsStore::new(Arc::new(db))
        .search("audit retention", 5)
        .expect("docs search");

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, "c1");
    assert_eq!(records[0].container.as_deref(), Some("doc-1"));
    assert!(records[0].score.is_some());
}

/// An index with the schema created and one chunk in it.
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

    let mut params: BTreeMap<String, DataValue> = BTreeMap::new();
    params.insert("id".into(), DataValue::from("c1"));
    params.insert(
        "fp".into(),
        DataValue::from("crates\\archon-knowledge\\src\\recall.rs"),
    );
    params.insert("ls".into(), DataValue::from(10i64));
    params.insert("le".into(), DataValue::from(42i64));
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
            <- [[$id, $fp, "rust", $ls, $le, "fn recall() {}", "deadbeef", 0.0, $emb]]
        :put code_chunks {
            chunk_id => file_path, language, line_start, line_end, chunk_content, file_hash,
            indexed_at, embedding
        }
        "#,
        params,
        ScriptMutability::Mutable,
    )
    .expect("put chunk");
    (dir, db)
}

#[test]
fn code_records_use_the_span_as_id_and_a_slash_path_as_container() {
    let (_dir, db) = fixture_index();
    let records = CodeIndexStore::with_embedder(db, Arc::new(ConstantEmbedder))
        .search("recall facade", 5)
        .expect("code search");

    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].id, "crates/archon-knowledge/src/recall.rs:10-42",
        "the span must identify the record"
    );
    assert_eq!(
        records[0].container.as_deref(),
        Some("crates/archon-knowledge/src/recall.rs"),
        "backslashes must be normalised so provenance joins across platforms"
    );
    assert!(records[0].score.is_some());
}

/// A missing index must fail before any embedding provider is built, and the
/// message must name the path the operator has to index.
#[test]
fn a_missing_code_index_fails_without_touching_an_embedder() {
    let dir = tempfile::tempdir().expect("tempdir");
    let absent = dir.path().join("leann.db");
    // `CodeIndexStore` holds a `Search`, which is not `Debug`, so unwrap the
    // error by hand rather than through `expect_err`.
    let message = match CodeIndexStore::open(&absent) {
        Ok(_) => panic!("opening a missing index must be refused"),
        Err(error) => error.to_string(),
    };
    assert!(message.contains("no code index at"), "{message}");
    assert!(message.contains("out of band"), "{message}");
    assert!(!absent.exists(), "opening a missing index created it");
}
