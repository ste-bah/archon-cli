//! Permanent removal of an ingested document.
//!
//! [`reprocess`](crate::reprocess) already knows how to clear everything the ingest pipeline
//! generated for a document; delete reuses that and additionally drops the *registration* rows
//! reprocess deliberately preserves — `doc_sources` above all.
//!
//! Dropping `doc_sources` is the load-bearing part. Content-hash dedupe is not a side registry:
//! [`store::hash_exists_in_sources`] queries `doc_sources.content_hash` directly, so as long as
//! that row survives, re-ingesting the same bytes reports `Skipped: 1 duplicates` with
//! `was_new == false`. An ingest killed partway through leaves exactly that row behind (it is
//! inserted before the pipeline runs), which is the bug this command exists to clear.

use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::errors::DocsError;
use crate::schema::ensure_doc_schema;
use crate::store;

/// What a single [`delete_document`] call removed.
#[derive(Clone, Debug, Default)]
pub struct DeletedDocument {
    pub document_id: String,
    pub source_path: String,
    pub content_hash: String,
    pub chunks: usize,
    pub pages: usize,
    pub artifacts: usize,
    /// Raw vectors removed from the RocksDB vector store.
    pub vectors: usize,
}

/// Delete `document_id` and everything derived from it.
///
/// Errors with [`DocsError::Validation`] if no such document exists.
pub fn delete_document(db: &DbInstance, document_id: &str) -> Result<DeletedDocument, DocsError> {
    ensure_doc_schema(db).map_err(storage)?;
    let doc = store::get_doc_source(db, document_id)
        .map_err(storage)?
        .ok_or_else(|| DocsError::Validation {
            message: format!("document not found: {document_id}"),
        })?;

    // Collect chunk ids before the rows go away — the RocksDB vector store is keyed by chunk id
    // and has no back-reference to the document.
    let chunk_ids = store::list_chunks_for_doc(db, document_id)
        .map_err(storage)?
        .into_iter()
        .map(|chunk| chunk.chunk_id)
        .collect::<Vec<_>>();

    let cleared = crate::reprocess::clear_generated_evidence(db, document_id)?;
    remove_registration_rows(db, document_id)?;
    let vectors = remove_raw_vectors(&chunk_ids);

    Ok(DeletedDocument {
        document_id: document_id.to_string(),
        source_path: doc.source_path,
        content_hash: doc.content_hash,
        chunks: cleared.chunks,
        pages: cleared.pages,
        artifacts: cleared.artifacts,
        vectors,
    })
}

/// Drop the rows that survive a reprocess: KB membership, index-job markers, and the
/// `doc_sources` row that content-hash dedupe consults.
fn remove_registration_rows(db: &DbInstance, document_id: &str) -> Result<(), DocsError> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));

    // Composite key — both columns are needed to identify the row.
    run_rm(
        db,
        "?[kb_id, document_id] := *doc_kb_memberships{kb_id, document_id}, document_id = $did
         :rm doc_kb_memberships { kb_id, document_id }",
        params.clone(),
        "doc_kb_memberships",
    )?;
    run_rm(
        db,
        "?[job_id] := *doc_index_jobs{job_id, document_id}, document_id = $did
         :rm doc_index_jobs { job_id }",
        params.clone(),
        "doc_index_jobs",
    )?;
    run_rm(
        db,
        "?[document_id] <- [[$did]]
         :rm doc_sources { document_id }",
        params,
        "doc_sources",
    )
}

/// Best-effort removal of raw vectors. The vector store lives outside the Cozo DB and may be
/// locked by a running indexer; failing the whole delete over it would leave the caller with a
/// half-deleted document, whereas a leftover vector is inert — semantic search resolves every
/// hit through `doc_chunks` and drops ids that no longer exist.
fn remove_raw_vectors(chunk_ids: &[String]) -> usize {
    if chunk_ids.is_empty() {
        return 0;
    }
    match crate::vector_store::DocVectorStore::acquire_default()
        .and_then(|store| store.delete_chunks(chunk_ids))
    {
        Ok(removed) => removed,
        Err(error) => {
            tracing::warn!(%error, "vector store cleanup failed; orphaned vectors left behind");
            0
        }
    }
}

fn run_rm(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    label: &str,
) -> Result<(), DocsError> {
    crate::cozo_retry::run_script_guarded(
        db,
        script,
        params,
        ScriptMutability::Mutable,
        &format!("delete {label} rows"),
    )
    .map_err(|e| DocsError::Storage {
        message: format!("delete {label} rows failed: {e}"),
    })?;
    Ok(())
}

fn storage(error: impl std::fmt::Display) -> DocsError {
    DocsError::Storage {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use cozo::DbInstance;

    use super::*;
    use crate::ingest::ingest_file_with_policy;

    fn test_db() -> DbInstance {
        let db = DbInstance::new("mem", "", "").unwrap();
        ensure_doc_schema(&db).unwrap();
        db
    }

    // Serial with the docs_global_state group for the same reason the reprocess tests are:
    // ingest touches the process-global OCR/VLM/embedding registries.
    #[tokio::test]
    #[serial_test::serial(docs_global_state)]
    async fn delete_releases_the_content_hash_so_identical_bytes_reingest() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("elliott-notes.md");
        let content = "Wave one starts the impulse.\nWave two retraces.\n";
        fs::write(&path, content).unwrap();
        let policy = archon_policy::EffectivePolicy::default();

        let original = ingest_file_with_policy(&db, &path, &policy).await.unwrap();
        assert!(original.was_new);
        let chunks = store::list_chunks_for_doc(&db, &original.document_id).unwrap();
        assert!(!chunks.is_empty());

        // Re-ingesting before the delete is refused as a duplicate — this is the state a killed
        // ingest leaves behind.
        let duplicate = ingest_file_with_policy(&db, &path, &policy).await.unwrap();
        assert!(!duplicate.was_new);

        let deleted = delete_document(&db, &original.document_id).unwrap();
        assert_eq!(deleted.document_id, original.document_id);
        assert_eq!(deleted.chunks, chunks.len());

        assert!(
            store::get_doc_source(&db, &original.document_id)
                .unwrap()
                .is_none()
        );
        assert!(
            store::list_chunks_for_doc(&db, &original.document_id)
                .unwrap()
                .is_empty()
        );
        assert!(
            !store::hash_exists_in_sources(&db, &deleted.content_hash).unwrap(),
            "content hash still registered; re-ingest would be skipped as a duplicate"
        );

        let reingested = ingest_file_with_policy(&db, &path, &policy).await.unwrap();
        assert!(
            reingested.was_new,
            "identical content must ingest as new after delete"
        );
        assert_ne!(reingested.document_id, original.document_id);
        assert!(
            !store::list_chunks_for_doc(&db, &reingested.document_id)
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    #[serial_test::serial(docs_global_state)]
    async fn delete_clears_kb_membership_and_queue_rows() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.md");
        fs::write(&path, "Impulse waves subdivide into five.\n").unwrap();
        let policy = archon_policy::EffectivePolicy::default();

        let doc = ingest_file_with_policy(&db, &path, &policy).await.unwrap();
        store::assign_document_to_kb(&db, "trading-elliott-wave", &doc.document_id).unwrap();
        let chunks = store::list_chunks_for_doc(&db, &doc.document_id).unwrap();
        for chunk in &chunks {
            crate::index_queue::enqueue_pending_chunk(&db, chunk, 0).unwrap();
        }
        assert!(crate::index_queue::stats(&db).unwrap().pending > 0);

        delete_document(&db, &doc.document_id).unwrap();

        assert!(
            store::list_kb_document_ids(&db, "trading-elliott-wave")
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            crate::index_queue::stats(&db).unwrap().pending,
            0,
            "pending index-queue jobs outlived the document"
        );
    }

    #[test]
    fn delete_of_unknown_document_errors() {
        let db = test_db();
        let err = delete_document(&db, "doc-does-not-exist").unwrap_err();
        assert!(err.to_string().contains("document not found"));
    }
}
