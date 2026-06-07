use super::*;
use crate::retrieval::{
    RetrievalWeights, SearchMode, index_chunk, index_pending_chunks, reindex_all, search,
};

#[test]
#[serial_test::serial(docs_global_state)]
fn search_surfaces_failed_indexing() {
    let db = test_db();
    setup_with_provider(&db, 4);

    let chunk1 = ChunkArtifact {
        chunk_id: "failed-chunk-a".into(),
        document_id: "doc-failed".into(),
        artifact_id: "art-1".into(),
        chunk_index: 0,
        page_start: 1,
        page_end: 1,
        content: "content a".into(),
        content_hash: "hash-a".into(),
        embedding_status: "failed".into(),
    };
    let chunk2 = ChunkArtifact {
        chunk_id: "failed-chunk-b".into(),
        document_id: "doc-failed".into(),
        artifact_id: "art-1".into(),
        chunk_index: 1,
        page_start: 1,
        page_end: 1,
        content: "content b".into(),
        content_hash: "hash-b".into(),
        embedding_status: "failed".into(),
    };
    store::insert_chunk(&db, &chunk1).unwrap();
    store::insert_chunk(&db, &chunk2).unwrap();

    let err = search(&db, "test query", 5).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("2 chunks failed"), "message was: {msg}");
    assert!(
        msg.contains("archon docs model-status"),
        "message was: {msg}"
    );
}

#[test]
#[serial_test::serial(docs_global_state)]
fn index_and_search_known_chunk() {
    let db = test_db();
    setup_with_provider(&db, 4);

    let chunk = ChunkArtifact {
        chunk_id: "chunk-search-1".into(),
        document_id: "doc-search".into(),
        artifact_id: "art-1".into(),
        chunk_index: 0,
        page_start: 1,
        page_end: 1,
        content: "The quick brown fox jumps over the lazy dog".into(),
        content_hash: "abc".into(),
        embedding_status: "pending".into(),
    };
    store::insert_chunk(&db, &chunk).unwrap();
    index_chunk(&db, &chunk).unwrap();

    assert_eq!(store::count_indexed_chunks(&db).unwrap(), 1);
    let results = search(&db, "quick brown fox", 5).unwrap();
    assert!(!results.results.is_empty(), "search should return results");
    assert_eq!(results.total_indexed_chunks, 1);
}

#[test]
#[serial_test::serial(docs_global_state)]
fn search_does_not_mutate() {
    let db = test_db();
    setup_with_provider(&db, 4);
    let chunk = ChunkArtifact {
        chunk_id: "chunk-no-mutate".into(),
        document_id: "doc-no-mutate".into(),
        artifact_id: "art-1".into(),
        chunk_index: 0,
        page_start: 1,
        page_end: 1,
        content: "content for non-mutating search test".into(),
        content_hash: "h1".into(),
        embedding_status: "pending".into(),
    };
    store::insert_chunk(&db, &chunk).unwrap();

    assert_eq!(store::count_embeddings(&db).unwrap(), 0);
    let results = search(&db, "test query", 5).unwrap();
    assert!(!results.results.is_empty());
    assert_eq!(results.total_indexed_chunks, 0);
    assert_eq!(results.results[0].chunk_id, "chunk-no-mutate");
    assert_eq!(store::count_embeddings(&db).unwrap(), 0);

    let chunk_after = store::get_chunk_by_id(&db, "chunk-no-mutate")
        .unwrap()
        .unwrap();
    assert_eq!(chunk_after.embedding_status, "pending");
}

#[test]
#[serial_test::serial(docs_global_state)]
fn index_pending_command_indexes_pending_only() {
    let db = test_db();
    setup_with_provider(&db, 4);
    let chunk_done = ChunkArtifact {
        chunk_id: "chunk-done".into(),
        document_id: "doc-mixed".into(),
        artifact_id: "art-1".into(),
        chunk_index: 0,
        page_start: 1,
        page_end: 1,
        content: "already done chunk".into(),
        content_hash: "hd".into(),
        embedding_status: "indexed".into(),
    };
    store::insert_chunk(&db, &chunk_done).unwrap();
    let chunk_pending = ChunkArtifact {
        chunk_id: "chunk-pending".into(),
        document_id: "doc-mixed".into(),
        artifact_id: "art-2".into(),
        chunk_index: 1,
        page_start: 1,
        page_end: 1,
        content: "pending chunk for indexing".into(),
        content_hash: "hp".into(),
        embedding_status: "pending".into(),
    };
    store::insert_chunk(&db, &chunk_pending).unwrap();

    let result = index_pending_chunks(&db).unwrap();
    assert_eq!(result.indexed, 1);
    assert_eq!(result.failed, 0);
    assert_eq!(result.skipped, 0);

    let pending_after = store::get_chunk_by_id(&db, "chunk-pending")
        .unwrap()
        .unwrap();
    assert_eq!(pending_after.embedding_status, "indexed");
    let done_after = store::get_chunk_by_id(&db, "chunk-done").unwrap().unwrap();
    assert_eq!(done_after.embedding_status, "indexed");

    let vector_store = crate::vector_store::DocVectorStore::open_default().unwrap();
    assert!(vector_store.has_vector("mock", "chunk-pending").unwrap());
    assert!(!vector_store.has_vector("mock", "chunk-done").unwrap());
}

#[test]
#[serial_test::serial(docs_global_state)]
fn get_chunk_by_id_roundtrip() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    let chunk = ChunkArtifact {
        chunk_id: "chunk-roundtrip-1".into(),
        document_id: "doc-roundtrip".into(),
        artifact_id: "art-1".into(),
        chunk_index: 2,
        page_start: 5,
        page_end: 7,
        content: "roundtrip test content".into(),
        content_hash: "rthash".into(),
        embedding_status: "pending".into(),
    };
    store::insert_chunk(&db, &chunk).unwrap();

    let found = store::get_chunk_by_id(&db, "chunk-roundtrip-1")
        .unwrap()
        .unwrap();
    assert_eq!(found.chunk_id, chunk.chunk_id);
    assert_eq!(found.document_id, chunk.document_id);
    assert_eq!(found.content, chunk.content);
    assert_eq!(found.page_start, 5);
    assert_eq!(found.page_end, 7);
    assert_eq!(found.chunk_index, 2);
    assert_eq!(found.content_hash, "rthash");
    assert_eq!(found.embedding_status, "pending");
    assert!(
        store::get_chunk_by_id(&db, "no-such-chunk")
            .unwrap()
            .is_none()
    );
}

#[test]
#[serial_test::serial(docs_global_state)]
fn index_result_counts_indexed_and_failed() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    crate::embed::clear_provider();
    crate::embed::set_provider(Box::new(SelectiveMockProvider { dim: 4 }));

    let chunk_ok = ChunkArtifact {
        chunk_id: "chunk-ok".into(),
        document_id: "doc-result".into(),
        artifact_id: "art-1".into(),
        chunk_index: 0,
        page_start: 1,
        page_end: 1,
        content: "normal content".into(),
        content_hash: "h1".into(),
        embedding_status: "pending".into(),
    };
    store::insert_chunk(&db, &chunk_ok).unwrap();
    let chunk_fail = ChunkArtifact {
        chunk_id: "chunk-fail".into(),
        document_id: "doc-result".into(),
        artifact_id: "art-2".into(),
        chunk_index: 1,
        page_start: 2,
        page_end: 2,
        content: "content with FAIL trigger".into(),
        content_hash: "h2".into(),
        embedding_status: "pending".into(),
    };
    store::insert_chunk(&db, &chunk_fail).unwrap();

    let result = index_pending_chunks(&db).unwrap();
    assert_eq!(result.indexed, 1);
    assert_eq!(result.failed, 1);
    assert_eq!(result.skipped, 0);
    let failed_after = store::get_chunk_by_id(&db, "chunk-fail").unwrap().unwrap();
    assert_eq!(failed_after.embedding_status, "failed");
}

#[test]
#[serial_test::serial(docs_global_state)]
fn reindex_all_indexes_everything() {
    let db = test_db();
    setup_with_provider(&db, 4);
    insert_test_chunk(&db, "chunk-indexed", "already indexed");
    insert_test_chunk(&db, "chunk-pending2", "pending work");

    let result = reindex_all(&db).unwrap();
    assert_eq!(result.indexed, 2);
    assert_eq!(result.failed, 0);
    assert_eq!(result.skipped, 0);
}

#[test]
#[serial_test::serial(docs_global_state)]
fn mode_imports_do_not_affect_indexing_tests() {
    assert_eq!(SearchMode::Hybrid.as_str(), "hybrid");
    assert_eq!(RetrievalWeights::default().exact, 0.45);
}
