//! Persisted evidence for the `kb_nodes` ingest path and its semantic index.
//!
//! This used to also drive `Compiler` and `QueryEngine`. Both now work on the
//! document store, so what is left here is exactly the surface that
//! [`super::ingest`] still owns and nothing in the CLI reaches: node rows,
//! content-hash ownership, embedding rows and the HNSW index. It is kept
//! deliberately — it is the only proof that the orphaned half still behaves
//! while the decision to remove it is outstanding.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use archon_docs::embed::LocalEmbeddingProvider;
use archon_docs::errors::DocsError;
use cozo::{DbInstance, ScriptMutability};

use super::ingest::Ingester;
use super::schema::ensure_kb_schema;

struct EvidenceEmbedder;

impl LocalEmbeddingProvider for EvidenceEmbedder {
    fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        Ok(chunks.iter().map(|chunk| vector_for(chunk)).collect())
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError> {
        Ok(vector_for(query))
    }

    fn dimension(&self) -> usize {
        2
    }

    fn backend_name(&self) -> &'static str {
        "issue90-runtime-evidence"
    }
}

#[tokio::test]
async fn persisted_kb_runtime_evidence() {
    let started = Instant::now();
    let temp = tempfile::tempdir().expect("temporary persisted database");
    let path = temp.path().join("issue90-kb.sqlite");
    let path = path.to_string_lossy().into_owned();
    let ingested = populate_persisted_kb(&path, temp.path()).await;
    let physical = read_physical_state(&path);
    assert_evidence(ingested, &physical);
    print_evidence(&physical, started.elapsed().as_millis());
}

async fn populate_persisted_kb(path: &str, fixture_dir: &std::path::Path) -> usize {
    let db = sqlite_db(path);
    ensure_kb_schema(&db).expect("create KB schema");
    let ingester = Ingester::with_embedder(db.clone(), Arc::new(EvidenceEmbedder))
        .expect("configure deterministic embedder");
    ingest_evidence_files(&ingester, fixture_dir).await
}

async fn ingest_evidence_files(ingester: &Ingester, fixture_dir: &std::path::Path) -> usize {
    let mut created = 0;
    for (index, (title, content)) in evidence_chunks().iter().enumerate() {
        let path = fixture_dir.join(format!("{index}-{title}.txt"));
        std::fs::write(&path, content).expect("write local KB fixture");
        let result = ingester
            .ingest_text(&path, "evidence")
            .await
            .expect("production persisted ingest");
        created += result.nodes_created;
    }
    assert_eq!(created, evidence_chunks().len());
    created
}

fn evidence_chunks() -> Vec<(String, String)> {
    let lexical = (0..7).map(|index| {
        (
            format!("lexical-{index}"),
            format!("MiXeDCase lexical marker {index}"),
        )
    });
    lexical
        .chain([
            ("semantic".into(), "semantic target".into()),
            ("other".into(), "unrelated sibling".into()),
        ])
        .collect()
}

struct PhysicalState {
    nodes: usize,
    embeddings: usize,
    hashes: usize,
    hnsw: bool,
}

fn read_physical_state(path: &str) -> PhysicalState {
    let db = sqlite_db(path);
    let state = PhysicalState {
        nodes: count_rows(&db, "kb_nodes", "node_id"),
        embeddings: count_rows(&db, "kb_embeddings", "node_id"),
        hashes: count_rows(&db, "kb_content_hashes", "content_hash"),
        hnsw: has_semantic_index(&db),
    };
    assert_exact_raw_ownership(&db);
    assert_no_orphans(&db);
    state
}

fn assert_evidence(ingested: usize, state: &PhysicalState) {
    assert_eq!(ingested, 9);
    assert_eq!(state.nodes, 9);
    assert_eq!(state.embeddings, 9);
    assert_eq!(state.hashes, 9);
    assert!(state.hnsw);
}

fn print_evidence(state: &PhysicalState, elapsed_ms: u128) {
    println!(
        "EVIDENCE kb_runtime nodes={} embeddings={} hashes={} hnsw={} elapsed_ms={elapsed_ms}",
        state.nodes, state.embeddings, state.hashes, state.hnsw,
    );
}

fn vector_for(text: &str) -> Vec<f32> {
    match text.to_lowercase().as_str() {
        "semantic query" => vec![1.0, 0.5],
        text if text.contains("semantic") => vec![1.0, 0.0],
        _ => vec![0.0, 1.0],
    }
}

fn sqlite_db(path: &str) -> DbInstance {
    DbInstance::new("sqlite", path, "").expect("open persisted Cozo database")
}

fn count_rows(db: &DbInstance, relation: &str, key: &str) -> usize {
    let result = db
        .run_script(
            &format!("?[count({key})] := *{relation}{{{key}}}"),
            BTreeMap::new(),
            ScriptMutability::Immutable,
        )
        .expect("count persisted relation rows");
    result.rows[0][0].get_int().unwrap_or_default() as usize
}

fn assert_exact_raw_ownership(db: &DbInstance) {
    let expected: BTreeSet<_> = evidence_chunks()
        .into_iter()
        .map(|(_, content)| (content_hash(&content), content))
        .collect();
    let actual: BTreeSet<_> = query_string_pairs(
        db,
        "?[hash, node_id, content] := *kb_content_hashes{content_hash: hash, node_id}, \
         *kb_nodes{node_id, node_type, content_hash: hash, content}, node_type = 'raw'",
    )
    .into_iter()
    .collect();
    assert_eq!(actual.len(), expected.len());
    assert!(
        actual
            .iter()
            .all(|(hash, _, content)| expected.contains(&(hash.clone(), content.clone())))
    );
}

fn query_string_pairs(db: &DbInstance, query: &str) -> Vec<(String, String, String)> {
    db.run_script(query, BTreeMap::new(), ScriptMutability::Immutable)
        .expect("read exact persisted relationships")
        .rows
        .into_iter()
        .map(|row| {
            (
                row[0].get_str().unwrap_or_default().to_owned(),
                row[1].get_str().unwrap_or_default().to_owned(),
                row[2].get_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

fn assert_no_orphans(db: &DbInstance) {
    for (query, message) in [
        (
            "?[hash] := *kb_content_hashes{content_hash: hash, node_id}, not *kb_nodes{node_id}",
            "content hashes must own real nodes",
        ),
        (
            "?[node] := *kb_embeddings{node_id: node}, not *kb_nodes{node_id: node}",
            "embeddings must own real nodes",
        ),
        (
            "?[node] := *kb_nodes{node_id: node, node_type}, node_type = 'raw', not *kb_content_hashes{content_hash, node_id: node}",
            "raw nodes must have hash owners",
        ),
    ] {
        let rows = db
            .run_script(query, BTreeMap::new(), ScriptMutability::Immutable)
            .expect("verify persisted relation integrity");
        assert!(rows.rows.is_empty(), "{message}");
    }
}

fn content_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};

    format!("{:x}", Sha256::digest(content.as_bytes()))
}

fn has_semantic_index(db: &DbInstance) -> bool {
    let indices = db
        .run_script(
            "::indices kb_embeddings",
            BTreeMap::new(),
            ScriptMutability::Immutable,
        )
        .expect("list persisted embedding indexes");
    indices.rows.iter().any(|row| {
        row.iter()
            .any(|value| value.get_str() == Some("semantic_idx"))
    })
}
