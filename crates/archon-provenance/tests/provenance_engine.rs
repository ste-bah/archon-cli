use cozo::{DbInstance, ScriptMutability};
use serde_json::json;

use archon_provenance::chain;
use archon_provenance::record::{ProvenanceEdge, ProvenanceEdgeType, ProvenanceRecord};
use archon_provenance::{export_w3c, store, traverse, verify};

fn test_db() -> DbInstance {
    DbInstance::new("mem", "", "").expect("mem db")
}

fn record(record_id: &str, artifact_id: &str, parent_ids: Vec<String>) -> ProvenanceRecord {
    let parent_hashes = Vec::new();
    let parameters_json = json!({"mode": "test"});
    let chain_hash = chain::chain_hash(
        &parent_hashes,
        "test-op",
        &["input-hash".into()],
        "output-hash",
        Some("tool"),
        Some("model"),
        &parameters_json,
    );
    ProvenanceRecord {
        record_id: record_id.into(),
        artifact_id: artifact_id.into(),
        artifact_type: "test-artifact".into(),
        operation: "test-op".into(),
        input_hashes: vec!["input-hash".into()],
        output_hash: "output-hash".into(),
        parent_record_ids: parent_ids,
        tool_name: Some("tool".into()),
        agent_name: Some("agent".into()),
        model: Some("model".into()),
        parameters_json,
        timestamp: "2026-01-01T00:00:00Z".into(),
        chain_hash,
    }
}

fn ensure_doc_fixture_schema(db: &DbInstance) {
    for script in [
        r#":create doc_sources {
            document_id: String => source_path: String, media_type: String,
            content_hash: String, discovered_at: String, status: String
        }"#,
        r#":create doc_pages {
            page_id: String => document_id: String, page_number: Int,
            text_hash: String?, image_hash: String?, width: Float?, height: Float?,
            provenance_record_id: String
        }"#,
        r#":create doc_chunks {
            chunk_id: String => document_id: String, artifact_id: String,
            chunk_index: Int, page_start: Int, page_end: Int, content: String,
            content_hash: String, embedding_status: String
        }"#,
        r#":create doc_artifacts {
            artifact_id: String => document_id: String, artifact_type: String,
            content_hash: String, created_at: String, provenance_record_id: String
        }"#,
        r#":create doc_provenance_edges {
            edge_id: String => from_artifact_id: String, to_artifact_id: String,
            edge_type: String, created_at: String
        }"#,
        r#":create gt_provenance_edges {
            edge_id: String => from_artifact_id: String, to_artifact_id: String,
            edge_type: String, created_at: String
        }"#,
    ] {
        let _ = db.run_script(script, Default::default(), ScriptMutability::Mutable);
    }
}

fn seed_doc_trace(db: &DbInstance) {
    ensure_doc_fixture_schema(db);
    db.run_script(
        "?[document_id, source_path, media_type, content_hash, discovered_at, status] <- \
         [['doc-1', '/tmp/source.pdf', 'application/pdf', 'hash-source', 'now', 'Ingested']] \
         :put doc_sources { document_id => source_path, media_type, content_hash, discovered_at, status }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .expect("source");
    db.run_script(
        "?[page_id, document_id, page_number, text_hash, image_hash, width, height, provenance_record_id] <- \
         [['page-doc-1-1', 'doc-1', 1, 'hash-page', null, null, null, 'prov-page']] \
         :put doc_pages { page_id => document_id, page_number, text_hash, image_hash, width, height, provenance_record_id }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .expect("page");
    db.run_script(
        "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] <- \
         [['chunk-1', 'doc-1', 'artifact-ocr', 0, 1, 1, 'Important text', 'hash-chunk', 'indexed']] \
         :put doc_chunks { chunk_id => document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .expect("chunk");
}

#[test]
fn schema_creation_is_idempotent() {
    let db = test_db();
    store::ensure_schema(&db).unwrap();
    store::ensure_schema(&db).unwrap();
}

#[test]
fn record_round_trips_through_cozo() {
    let db = test_db();
    let rec = record("prov-1", "artifact-1", Vec::new());
    store::insert_record(&db, &rec).unwrap();
    let loaded = store::get_record(&db, "prov-1").unwrap().unwrap();
    assert_eq!(loaded.artifact_id, "artifact-1");
    assert_eq!(loaded.parameters_json["mode"], "test");
}

#[test]
fn record_can_be_loaded_by_artifact() {
    let db = test_db();
    store::insert_record(&db, &record("prov-1", "artifact-1", Vec::new())).unwrap();
    let loaded = store::get_record_by_artifact(&db, "artifact-1")
        .unwrap()
        .unwrap();
    assert_eq!(loaded.record_id, "prov-1");
}

#[test]
fn edge_round_trips_through_cozo() {
    let db = test_db();
    let edge = ProvenanceEdge::new("child", "parent", ProvenanceEdgeType::DerivedFrom);
    store::insert_edge(&db, &edge).unwrap();
    let edges = store::list_edges_from(&db, "child").unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to_artifact_id, "parent");
}

#[test]
fn chain_verification_passes_for_valid_record() {
    let db = test_db();
    store::insert_record(&db, &record("prov-1", "artifact-1", Vec::new())).unwrap();
    let report = verify::verify_record_chain(&db, "prov-1").unwrap();
    assert!(report.valid);
}

#[test]
fn chain_verification_fails_for_tampered_hash() {
    let db = test_db();
    let mut rec = record("prov-1", "artifact-1", Vec::new());
    rec.chain_hash = "tampered".into();
    store::insert_record(&db, &rec).unwrap();
    let report = verify::verify_record_chain(&db, "prov-1").unwrap();
    assert!(!report.valid);
}

#[test]
fn chain_verification_reports_missing_parent() {
    let db = test_db();
    store::insert_record(&db, &record("prov-1", "artifact-1", vec!["missing".into()])).unwrap();
    let report = verify::verify_record_chain(&db, "prov-1").unwrap();
    assert_eq!(report.missing_parent_record_ids, vec!["missing"]);
    assert!(!report.valid);
}

#[test]
fn artifact_verification_fails_when_stored_chain_is_invalid() {
    let db = test_db();
    seed_doc_trace(&db);
    store::insert_edge(
        &db,
        &ProvenanceEdge::new("artifact-1", "doc-1", ProvenanceEdgeType::Cites),
    )
    .unwrap();
    let mut rec = record("prov-1", "artifact-1", Vec::new());
    rec.chain_hash = "tampered".into();
    store::insert_record(&db, &rec).unwrap();

    let report = verify::verify_artifact(&db, "artifact-1").unwrap();
    assert!(!report.valid);
    assert_eq!(report.chain_record_id.as_deref(), Some("prov-1"));
    assert_eq!(report.chain_valid, Some(false));
    assert!(report.reaches_source);
}

#[test]
fn trace_follows_generic_edges_to_source_document() {
    let db = test_db();
    seed_doc_trace(&db);
    let edge = ProvenanceEdge::new("answer-1", "doc-1", ProvenanceEdgeType::Cites);
    store::insert_edge(&db, &edge).unwrap();
    let trace = traverse::trace_artifact(&db, "answer-1").unwrap();
    assert!(trace.reaches_source());
    assert!(trace.nodes.iter().any(|n| n.artifact_id == "doc-1"));
}

#[test]
fn trace_follows_chunk_to_page_to_source() {
    let db = test_db();
    seed_doc_trace(&db);
    let trace = traverse::trace_artifact(&db, "chunk-1").unwrap();
    let ids: Vec<_> = trace.nodes.iter().map(|n| n.artifact_id.as_str()).collect();
    assert!(ids.contains(&"page-doc-1-1"));
    assert!(ids.contains(&"doc-1"));
    assert!(trace.reaches_source());
}

#[test]
fn synthetic_doc_edge_ids_are_stable() {
    let db = test_db();
    seed_doc_trace(&db);
    let first = traverse::trace_artifact(&db, "chunk-1").unwrap();
    let second = traverse::trace_artifact(&db, "chunk-1").unwrap();
    let first_ids: Vec<_> = first
        .edges
        .iter()
        .map(|edge| edge.edge_id.clone())
        .collect();
    let second_ids: Vec<_> = second
        .edges
        .iter()
        .map(|edge| edge.edge_id.clone())
        .collect();
    assert_eq!(first_ids, second_ids);
}

#[test]
fn trace_reads_gametheory_provenance_edges() {
    let db = test_db();
    ensure_doc_fixture_schema(&db);
    db.run_script(
        "?[edge_id, from_artifact_id, to_artifact_id, edge_type, created_at] <- \
         [['edge-gt-1', 'final-report:run-1', 'section:run-1:payoffs', 'contains', 'now'], \
          ['edge-gt-2', 'section:run-1:payoffs', 'specialist:run-1:payoff-elicitor', 'DerivedFrom', 'now']] \
         :put gt_provenance_edges { edge_id => from_artifact_id, to_artifact_id, edge_type, created_at }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    let trace = traverse::trace_artifact(&db, "final-report:run-1").unwrap();
    assert_eq!(trace.edges.len(), 2);
    assert!(
        trace
            .edges
            .iter()
            .any(|edge| edge.edge_type == ProvenanceEdgeType::Contains)
    );
    assert!(
        trace
            .nodes
            .iter()
            .any(|n| n.artifact_id == "specialist:run-1:payoff-elicitor")
    );
}

#[test]
fn export_jsonld_contains_w3c_context() {
    let db = test_db();
    seed_doc_trace(&db);
    let trace = traverse::trace_artifact(&db, "chunk-1").unwrap();
    let json = export_w3c::export_trace_jsonld(&trace);
    assert_eq!(json["@context"]["prov"], "http://www.w3.org/ns/prov#");
    assert_eq!(json["@type"], "prov:Bundle");
}

#[test]
fn export_jsonld_contains_entity_hashes() {
    let db = test_db();
    seed_doc_trace(&db);
    let trace = traverse::trace_artifact(&db, "chunk-1").unwrap();
    let json = export_w3c::export_trace_jsonld(&trace);
    assert_eq!(json["entity"]["doc-1"]["archon:contentHash"], "hash-source");
}

#[test]
fn artifact_verification_passes_when_trace_reaches_source() {
    let db = test_db();
    seed_doc_trace(&db);
    let report = verify::verify_artifact(&db, "chunk-1").unwrap();
    assert!(report.valid);
    assert!(report.reaches_source);
}

#[test]
fn artifact_verification_fails_without_lineage() {
    let db = test_db();
    let report = verify::verify_artifact(&db, "orphan").unwrap();
    assert!(!report.valid);
    assert_eq!(report.edge_count, 0);
}
