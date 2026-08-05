use cozo::DbInstance;

use super::*;
use crate::models::{
    ChunkArtifact, DocumentStatus, OcrRun, OcrStatus, PageArtifact, ProvenanceEdge, SourceDocument,
};

fn test_db() -> DbInstance {
    DbInstance::new("mem", "", Default::default()).unwrap()
}

fn test_doc(id: &str) -> SourceDocument {
    SourceDocument {
        document_id: id.to_string(),
        source_path: "/tmp/test.txt".to_string(),
        media_type: "text/plain".to_string(),
        content_hash: "abc123".to_string(),
        discovered_at: "2026-01-01T00:00:00Z".to_string(),
        status: DocumentStatus::Discovered,
    }
}

#[test]
fn test_insert_and_readback_doc_source() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    let doc = test_doc("insert-readback");
    insert_doc_source(&db, &doc).unwrap();
    let got = get_doc_source(&db, "insert-readback").unwrap().unwrap();
    assert_eq!(got.document_id, doc.document_id);
    assert_eq!(got.status, DocumentStatus::Discovered);
}

#[test]
fn test_update_status() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    let doc = test_doc("update-status");
    insert_doc_source(&db, &doc).unwrap();

    update_doc_status(&db, "update-status", &DocumentStatus::Ingested).unwrap();

    let got = get_doc_source(&db, "update-status").unwrap().unwrap();
    assert_eq!(
        got.status,
        DocumentStatus::Ingested,
        ":update must change status"
    );
}

#[test]
fn test_insert_and_readback_chunk() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    let chunk = ChunkArtifact {
        chunk_id: "chunk-test-0".into(),
        document_id: "test-doc".into(),
        artifact_id: "art-1".into(),
        chunk_index: 0,
        page_start: 1,
        page_end: 1,
        content: "hello".into(),
        content_hash: "hash123".into(),
        embedding_status: "pending".into(),
    };
    insert_chunk(&db, &chunk).unwrap();
    let chunks = list_chunks_for_doc(&db, "test-doc").unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].chunk_id, "chunk-test-0");
}

#[test]
fn test_insert_and_readback_page() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    let page = PageArtifact {
        page_id: "page-test-1".into(),
        document_id: "test-doc".into(),
        page_number: 1,
        text_hash: Some("txthash".into()),
        image_hash: None,
        width: None,
        height: None,
        provenance_record_id: String::new(),
    };
    insert_page(&db, &page).unwrap();
    let pages = list_pages_for_doc(&db, "test-doc").unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].page_number, 1);
}

#[test]
fn test_insert_and_readback_ocr_run() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    let run = OcrRun {
        ocr_run_id: "ocr-test-1".into(),
        document_id: "test-doc".into(),
        provider: "local".into(),
        mode: "text/plain".into(),
        status: OcrStatus::Completed,
        started_at: "2026-01-01T00:00:00Z".into(),
        completed_at: Some("2026-01-01T00:00:01Z".into()),
        duration_ms: Some(100),
    };
    insert_ocr_run(&db, &run).unwrap();
    let runs = list_ocr_runs_for_doc(&db, "test-doc").unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, OcrStatus::Completed);
}

#[test]
fn test_insert_and_readback_provenance_edge() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    let edge = ProvenanceEdge {
        edge_id: "edge-test-1".into(),
        from_artifact_id: "chunk-1".into(),
        to_artifact_id: "page-1".into(),
        edge_type: crate::models::ProvenanceEdgeType::ExtractedFrom,
        created_at: "2026-01-01T00:00:00Z".into(),
    };
    insert_provenance_edge(&db, &edge).unwrap();
    let edges = list_provenance_from(&db, "chunk-1").unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to_artifact_id, "page-1");
}

#[test]
fn test_hash_exists_and_get_by_hash() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    let doc = test_doc("hash-doc");
    insert_doc_source(&db, &doc).unwrap();
    assert!(hash_exists_in_sources(&db, "abc123").unwrap());
    assert!(!hash_exists_in_sources(&db, "nonexistent").unwrap());

    let found = get_doc_by_hash(&db, "abc123").unwrap().unwrap();
    assert_eq!(found.document_id, "hash-doc");
}

#[test]
fn test_insert_and_readback_vector() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    crate::schema::ensure_vec_schema(&db, 3, None).unwrap();

    let emb = vec![1.0_f32, 0.0, 0.0];
    insert_chunk_embedding(&db, "chunk-vec-1", &emb, "test-provider").unwrap();

    let got = get_chunk_embedding(&db, "chunk-vec-1").unwrap().unwrap();
    assert_eq!(got.len(), 3);
    assert!((got[0] - 1.0).abs() < 1e-6);

    let count = count_embeddings(&db).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_bulk_vector_and_status_updates() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    crate::schema::ensure_vec_schema(&db, 3, None).unwrap();

    let chunk_a = ChunkArtifact {
        chunk_id: "bulk-chunk-a".into(),
        document_id: "bulk-doc".into(),
        artifact_id: "art-1".into(),
        chunk_index: 0,
        page_start: 1,
        page_end: 1,
        content: "alpha".into(),
        content_hash: "hash-a".into(),
        embedding_status: "pending".into(),
    };
    let chunk_b = ChunkArtifact {
        chunk_id: "bulk-chunk-b".into(),
        document_id: "bulk-doc".into(),
        artifact_id: "art-1".into(),
        chunk_index: 1,
        page_start: 1,
        page_end: 1,
        content: "beta".into(),
        content_hash: "hash-b".into(),
        embedding_status: "pending".into(),
    };
    insert_chunk(&db, &chunk_a).unwrap();
    insert_chunk(&db, &chunk_b).unwrap();

    let emb_a = vec![1.0_f32, 0.0, 0.0];
    let emb_b = vec![0.0_f32, 1.0, 0.0];
    insert_chunk_embeddings(
        &db,
        &[
            ChunkEmbeddingInput {
                chunk_id: &chunk_a.chunk_id,
                embedding: &emb_a,
            },
            ChunkEmbeddingInput {
                chunk_id: &chunk_b.chunk_id,
                embedding: &emb_b,
            },
        ],
        "bulk-provider",
    )
    .unwrap();
    update_chunk_embedding_statuses(&db, &[&chunk_a, &chunk_b], "indexed").unwrap();

    assert_eq!(count_embeddings(&db).unwrap(), 2);
    assert_eq!(count_pending_chunks(&db).unwrap(), 0);
    let chunks = list_chunks_for_doc(&db, "bulk-doc").unwrap();
    assert!(
        chunks
            .iter()
            .all(|chunk| chunk.embedding_status == "indexed")
    );
}

#[test]
fn test_vector_roundtrip_preserves_norm() {
    let db = test_db();
    // Dimension 4 to test L2 norm explicitly
    crate::schema::ensure_doc_schema(&db).unwrap();
    crate::schema::ensure_vec_schema(&db, 4, None).unwrap();

    // Embedding with L2 norm = 5.0 (3-4-5 triangle in 2D plus zeros)
    let emb = [3.0_f32, 4.0_f32, 0.0, 0.0];
    // Normalise before storing (caller responsibility)
    let norm = (emb.iter().map(|x| x * x).sum::<f32>()).sqrt();
    let normalized: Vec<f32> = emb.iter().map(|x| x / norm).collect();
    insert_chunk_embedding(&db, "chunk-norm-1", &normalized, "test").unwrap();

    let got = get_chunk_embedding(&db, "chunk-norm-1").unwrap().unwrap();
    let got_norm: f32 = got.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(
        (got_norm - 1.0).abs() < 1e-6,
        "stored vector must be unit length"
    );
}

#[test]
fn test_ocr_run_single_row_after_completion() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();

    // Insert initial Running run
    let run = OcrRun {
        ocr_run_id: "ocr-single".into(),
        document_id: "test-doc".into(),
        provider: "local".into(),
        mode: "text/plain".into(),
        status: OcrStatus::Running,
        started_at: "2026-01-01T00:00:00Z".into(),
        completed_at: None,
        duration_ms: None,
    };
    insert_ocr_run(&db, &run).unwrap();
    assert_eq!(list_ocr_runs_for_doc(&db, "test-doc").unwrap().len(), 1);

    // Update to Completed — must not create a second row
    update_ocr_run_completion(
        &db,
        "ocr-single",
        &OcrStatus::Completed,
        "2026-01-01T00:00:05Z",
        500,
    )
    .unwrap();

    let runs = list_ocr_runs_for_doc(&db, "test-doc").unwrap();
    assert_eq!(runs.len(), 1, "update must not create a second row");
    assert_eq!(runs[0].status, OcrStatus::Completed);
    assert_eq!(runs[0].duration_ms, Some(500));
    assert_eq!(
        runs[0].completed_at.as_deref(),
        Some("2026-01-01T00:00:05Z")
    );
}

#[test]
fn test_count_failed_chunks_roundtrip() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();

    assert_eq!(count_failed_chunks(&db).unwrap(), 0);

    let chunk = ChunkArtifact {
        chunk_id: "failed-chunk-1".into(),
        document_id: "test-doc".into(),
        artifact_id: "art-1".into(),
        chunk_index: 0,
        page_start: 1,
        page_end: 1,
        content: "test".into(),
        content_hash: "abc".into(),
        embedding_status: "failed".into(),
    };
    insert_chunk(&db, &chunk).unwrap();
    assert_eq!(count_failed_chunks(&db).unwrap(), 1);

    let chunk2 = ChunkArtifact {
        chunk_id: "failed-chunk-2".into(),
        document_id: "test-doc".into(),
        artifact_id: "art-1".into(),
        chunk_index: 1,
        page_start: 1,
        page_end: 1,
        content: "test2".into(),
        content_hash: "def".into(),
        embedding_status: "failed".into(),
    };
    insert_chunk(&db, &chunk2).unwrap();
    assert_eq!(count_failed_chunks(&db).unwrap(), 2);
}

#[test]
fn unindexed_ingested_chunk_audit_counts_only_ingested_docs() {
    // Tier-1b completion audit: pending/failed chunks count ONLY when their document
    // is Ingested — a Failed document's chunks must not trip the post-index gate.
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    let mut doc_a = test_doc("audit-ingested");
    doc_a.status = DocumentStatus::Ingested;
    insert_doc_source(&db, &doc_a).unwrap();
    let mut doc_b = test_doc("audit-failed");
    doc_b.status = DocumentStatus::Failed;
    insert_doc_source(&db, &doc_b).unwrap();

    let chunk = |cid: &str, did: &str, status: &str| ChunkArtifact {
        chunk_id: cid.into(),
        document_id: did.into(),
        artifact_id: format!("ocr-{did}"),
        chunk_index: 0,
        page_start: 1,
        page_end: 1,
        content: "body".into(),
        content_hash: "h".into(),
        embedding_status: status.into(),
    };
    insert_chunk(&db, &chunk("c-a-0", "audit-ingested", "pending")).unwrap();
    insert_chunk(&db, &chunk("c-a-1", "audit-ingested", "indexed")).unwrap();
    insert_chunk(&db, &chunk("c-b-0", "audit-failed", "pending")).unwrap();

    assert_eq!(
        count_unindexed_ingested_chunks(&db).unwrap(),
        1,
        "one pending chunk on the Ingested doc; the Failed doc's chunk is exempt"
    );
    let a0 = chunk("c-a-0", "audit-ingested", "pending");
    update_chunk_embedding_statuses(&db, &[&a0], "indexed").unwrap();
    assert_eq!(count_unindexed_ingested_chunks(&db).unwrap(), 0);
}
