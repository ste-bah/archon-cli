use cozo::DbInstance;

use crate::index_queue::{
    count_pending, failed_rows, lease_pending_chunks, mark_chunks_failed, mark_chunks_indexed,
    prune_orphaned_queue_rows, remove_document_queue_rows, retry_failed, stats,
};
use crate::models::ChunkArtifact;
use crate::schema::ensure_doc_schema;
use crate::store;

fn test_db() -> DbInstance {
    let db = DbInstance::new("mem", "", Default::default()).unwrap();
    ensure_doc_schema(&db).unwrap();
    db
}

fn chunk(id: &str) -> ChunkArtifact {
    ChunkArtifact {
        chunk_id: id.into(),
        document_id: "doc-a".into(),
        artifact_id: "artifact-a".into(),
        chunk_index: 1,
        page_start: 1,
        page_end: 1,
        content: "hello indexed world".into(),
        content_hash: format!("hash-{id}"),
        embedding_status: "pending".into(),
    }
}

#[test]
fn insert_chunk_enqueues_pending_work() {
    let db = test_db();
    store::insert_chunk(&db, &chunk("chunk-a")).unwrap();
    assert_eq!(count_pending(&db, None).unwrap(), 1);
}

#[test]
fn lease_and_complete_queue_rows() {
    let db = test_db();
    let chunk = chunk("chunk-a");
    store::insert_chunk(&db, &chunk).unwrap();
    let leased = lease_pending_chunks(&db, "worker-a", 10, 60, None).unwrap();
    assert_eq!(leased.len(), 1);
    assert_eq!(stats(&db).unwrap().leased, 1);
    mark_chunks_indexed(&db, &[&chunk]).unwrap();
    let after = stats(&db).unwrap();
    assert_eq!(after.leased, 0);
    assert_eq!(after.indexed, 1);
}

#[test]
fn failed_rows_can_be_retried() {
    let db = test_db();
    let chunk = chunk("chunk-a");
    store::insert_chunk(&db, &chunk).unwrap();
    mark_chunks_failed(&db, &[&chunk], "boom").unwrap();
    assert_eq!(stats(&db).unwrap().failed, 1);
    assert_eq!(retry_failed(&db, None).unwrap(), 1);
    assert_eq!(stats(&db).unwrap().pending, 1);
}

#[test]
fn failed_rows_preserve_attempt_counts() {
    let db = test_db();
    let chunk = chunk("chunk-a");
    store::insert_chunk(&db, &chunk).unwrap();
    let leased = lease_pending_chunks(&db, "job-a", 10, 60, None).unwrap();
    mark_chunks_failed(&db, &[&leased[0]], "first").unwrap();
    retry_failed(&db, None).unwrap();
    let leased_again = lease_pending_chunks(&db, "job-a", 10, 60, None).unwrap();
    mark_chunks_failed(&db, &[&leased_again[0]], "second").unwrap();
    let failures = failed_rows(&db, 10).unwrap();
    assert_eq!(failures[0].attempt_count, 2);
    assert_eq!(failures[0].last_error, "second");
}

#[test]
fn document_queue_rows_can_be_removed() {
    let db = test_db();
    store::insert_chunk(&db, &chunk("chunk-a")).unwrap();
    remove_document_queue_rows(&db, "doc-a").unwrap();
    assert_eq!(stats(&db).unwrap().pending, 0);
}

#[test]
fn orphaned_queue_rows_are_pruned_before_counting_or_leasing() {
    let db = test_db();
    store::insert_chunk(&db, &chunk("chunk-a")).unwrap();
    db.run_script(
        "?[chunk_id] <- [[\"chunk-a\"]]
         :rm doc_chunks { chunk_id }",
        Default::default(),
        cozo::ScriptMutability::Mutable,
    )
    .unwrap();

    assert_eq!(prune_orphaned_queue_rows(&db).unwrap(), 1);
    assert_eq!(count_pending(&db, None).unwrap(), 0);
    assert!(
        lease_pending_chunks(&db, "worker-a", 10, 60, None)
            .unwrap()
            .is_empty()
    );
}
