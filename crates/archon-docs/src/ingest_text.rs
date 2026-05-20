//! Text-source ingestion for non-file inputs such as fetched URLs.

use cozo::DbInstance;

use crate::chunking::{build_chunk_artifacts, chunk_with_page_anchors};
use crate::errors::DocsError;
use crate::hash::sha256_str;
use crate::models::{
    ArtifactRecord, DocumentStatus, PageArtifact, PageOffset, ProcessingJob, SourceDocument,
};
use crate::provenance::build_doc_lineage_edges;
use crate::schema::ensure_doc_schema;
use crate::store;
use crate::{embed, retrieval};

#[derive(Clone, Debug)]
pub struct IngestTextSourceResult {
    pub document_id: String,
    pub was_new: bool,
    pub chunks_registered: usize,
}

pub fn ingest_text_source(
    db: &DbInstance,
    source_path: &str,
    media_type: &str,
    content: &str,
) -> Result<IngestTextSourceResult, DocsError> {
    ensure_doc_schema(db).map_err(storage)?;

    let content_hash = sha256_str(content);
    if let Some(existing) = store::get_doc_by_hash(db, &content_hash).map_err(storage)? {
        let chunks = store::list_chunks_for_doc(db, &existing.document_id).map_err(storage)?;
        return Ok(IngestTextSourceResult {
            document_id: existing.document_id,
            was_new: false,
            chunks_registered: chunks.len(),
        });
    }

    let document_id = format!("doc-{}", uuid::Uuid::new_v4());
    let artifact_id = format!("text-source-{document_id}");
    let page_id = format!("page-{document_id}-1");
    let started_at = chrono::Utc::now().to_rfc3339();
    let job_id = format!("job-{}", uuid::Uuid::new_v4());

    store::insert_doc_source(
        db,
        &SourceDocument {
            document_id: document_id.clone(),
            source_path: source_path.to_string(),
            media_type: media_type.to_string(),
            content_hash: content_hash.clone(),
            discovered_at: started_at.clone(),
            status: DocumentStatus::Discovered,
        },
    )
    .map_err(storage)?;
    store::insert_processing_job(
        db,
        &ProcessingJob {
            job_id: job_id.clone(),
            document_id: document_id.clone(),
            job_type: "ingest-url".to_string(),
            status: "running".to_string(),
            started_at: started_at.clone(),
            completed_at: None,
            error_message: None,
        },
    )
    .map_err(storage)?;
    store::update_doc_status(db, &document_id, &DocumentStatus::Ingesting).map_err(storage)?;

    store_artifacts(
        db,
        &document_id,
        &artifact_id,
        &page_id,
        &content_hash,
        content,
    )?;
    store::update_doc_status(db, &document_id, &DocumentStatus::Ingested).map_err(storage)?;
    store::insert_processing_job(
        db,
        &ProcessingJob {
            job_id,
            document_id: document_id.clone(),
            job_type: "ingest-url".to_string(),
            status: "completed".to_string(),
            started_at,
            completed_at: Some(chrono::Utc::now().to_rfc3339()),
            error_message: None,
        },
    )
    .map_err(storage)?;

    let chunks = store::list_chunks_for_doc(db, &document_id).map_err(storage)?;
    Ok(IngestTextSourceResult {
        document_id,
        was_new: true,
        chunks_registered: chunks.len(),
    })
}

fn store_artifacts(
    db: &DbInstance,
    document_id: &str,
    artifact_id: &str,
    page_id: &str,
    content_hash: &str,
    content: &str,
) -> Result<(), DocsError> {
    store::insert_artifact(
        db,
        &ArtifactRecord {
            artifact_id: artifact_id.to_string(),
            document_id: document_id.to_string(),
            artifact_type: "source_text".to_string(),
            content_hash: content_hash.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            provenance_record_id: String::new(),
        },
    )
    .map_err(storage)?;

    store::insert_page(
        db,
        &PageArtifact {
            page_id: page_id.to_string(),
            document_id: document_id.to_string(),
            page_number: 1,
            text_hash: Some(sha256_str(content)),
            image_hash: None,
            width: None,
            height: None,
            provenance_record_id: String::new(),
        },
    )
    .map_err(storage)?;

    let offsets = vec![PageOffset {
        page: 1,
        char_start: 0,
        char_end: content.len(),
    }];
    let page_chunks = chunk_with_page_anchors(content, &offsets);
    let chunks = build_chunk_artifacts(document_id, artifact_id, &page_chunks);
    for chunk in &chunks {
        store::insert_chunk(db, chunk).map_err(storage)?;
    }
    for edge in build_doc_lineage_edges(document_id, artifact_id, &chunks, &[page_id.to_string()]) {
        store::insert_provenance_edge(db, &edge).map_err(storage)?;
    }
    if embed::get_provider().is_some() {
        for chunk in &chunks {
            if let Err(error) = retrieval::index_chunk(db, chunk) {
                tracing::warn!(chunk_id = %chunk.chunk_id, %error, "failed to index URL chunk");
            }
        }
    }

    Ok(())
}

fn storage(error: impl std::fmt::Display) -> DocsError {
    DocsError::Storage {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let db = DbInstance::new("mem", "", "").unwrap();
        ensure_doc_schema(&db).unwrap();
        db
    }

    #[test]
    fn text_source_ingest_writes_rich_doc_rows() {
        let db = test_db();
        let result = ingest_text_source(
            &db,
            "https://example.test/policy.txt",
            "text/plain",
            "Archon uses CozoDB.",
        )
        .unwrap();

        assert!(result.was_new);
        assert_eq!(result.chunks_registered, 1);
        let doc = store::get_doc_source(&db, &result.document_id)
            .unwrap()
            .unwrap();
        assert_eq!(doc.status, DocumentStatus::Ingested);
        let artifacts = store::list_artifacts_for_doc(&db, &result.document_id).unwrap();
        assert_eq!(artifacts.len(), 1);
        let chunks = store::list_chunks_for_doc(&db, &result.document_id).unwrap();
        assert_eq!(chunks.len(), 1);
        let chunk_edges = store::list_provenance_from(&db, &chunks[0].chunk_id).unwrap();
        assert_eq!(chunk_edges.len(), 1);
        let artifact_edges = store::list_provenance_from(&db, &artifacts[0].artifact_id).unwrap();
        assert_eq!(artifact_edges.len(), 1);
    }

    #[test]
    fn text_source_ingest_deduplicates_by_content_hash() {
        let db = test_db();
        let first =
            ingest_text_source(&db, "https://example.test/a.txt", "text/plain", "Same.").unwrap();
        let second =
            ingest_text_source(&db, "https://example.test/b.txt", "text/plain", "Same.").unwrap();
        assert_eq!(first.document_id, second.document_id);
        assert!(!second.was_new);
    }
}
