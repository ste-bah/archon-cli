//! Byte-source ingestion for fetched URLs and other non-local sources.

use std::io::Write;

use cozo::DbInstance;
use tempfile::Builder;
use tracing::info;

use crate::errors::DocsError;
use crate::hash::sha256_hex;
use crate::ingest::{
    IngestFileResult, PipelineOutcome, is_ocr_runnable, is_supported_media_type,
    run_ingest_pipeline_with_bytes,
};
use crate::models::{DocumentStatus, ProcessingJob, SourceDocument};
use crate::schema::ensure_doc_schema;
use crate::store;

pub async fn ingest_bytes_source_with_policy(
    db: &DbInstance,
    source_path: &str,
    media_type: &str,
    content_bytes: &[u8],
    policy: &archon_policy::EffectivePolicy,
) -> Result<IngestFileResult, DocsError> {
    ensure_doc_schema(db).map_err(storage)?;

    if !is_supported_media_type(media_type) {
        return Err(DocsError::UnsupportedMediaType {
            media_type: media_type.to_string(),
        });
    }

    let content_hash = sha256_hex(content_bytes);
    let document_id = format!("doc-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().to_rfc3339();

    let reservation = store::reserve_doc_source_by_hash(
        db,
        &SourceDocument {
            document_id: document_id.clone(),
            source_path: source_path.to_string(),
            media_type: media_type.to_string(),
            content_hash: content_hash.clone(),
            discovered_at: now.clone(),
            status: DocumentStatus::Discovered,
        },
    )
    .map_err(storage)?;

    if let store::HashReservation::Duplicate(existing) = reservation {
        info!(
            source = %source_path,
            hash = %content_hash,
            existing = %existing.document_id,
            "Skipping duplicate byte source"
        );
        return Ok(IngestFileResult {
            document_id: existing.document_id,
            was_new: false,
            ocr_skipped: false,
            pipeline_failed: false,
            warnings: Vec::new(),
            image_embeddings_stored: 0,
            vlm_descriptions: 0,
            pdf_embedded_images_extracted: 0,
            pdf_embedded_images_skipped_filter: 0,
            pdf_image_ocr_runs: 0,
            pdf_image_vlm_failures: 0,
            pdf_image_ocr_failures: 0,
            pdf_pages_rendered: 0,
            pdf_coord: None,
        });
    }

    store::insert_processing_job(
        db,
        &ProcessingJob {
            job_id: format!("job-{}", uuid::Uuid::new_v4()),
            document_id: document_id.clone(),
            job_type: "ingest".to_string(),
            status: "pending".to_string(),
            started_at: now,
            completed_at: None,
            error_message: None,
        },
    )
    .map_err(storage)?;

    let mut ocr_skipped = false;
    let mut pipeline_failed = false;
    let mut outcome = PipelineOutcome::default();
    if is_ocr_runnable(media_type) {
        store::update_doc_status(db, &document_id, &DocumentStatus::Ingesting).map_err(storage)?;
        let mut tmp = Builder::new()
            .prefix("archon-url-ingest-")
            .suffix(media_type_suffix(media_type))
            .tempfile()?;
        tmp.write_all(content_bytes)?;
        tmp.flush()?;
        let tmp_path = tmp.path().to_string_lossy().to_string();

        match run_ingest_pipeline_with_bytes(
            db,
            &document_id,
            &tmp_path,
            media_type,
            content_bytes,
            policy,
        )
        .await
        {
            Ok(pipeline_outcome) => {
                outcome = pipeline_outcome;
                store::update_doc_status(db, &document_id, &DocumentStatus::Ingested)
                    .map_err(storage)?;
            }
            Err(e) => {
                pipeline_failed = true;
                outcome.warnings.push(format!("OCR pipeline failed: {e}"));
                store::update_doc_status(db, &document_id, &DocumentStatus::Failed)
                    .map_err(storage)?;
                info!(
                    document_id = %document_id,
                    source = %source_path,
                    error = %e,
                    "byte-source OCR pipeline failed; document set to Failed"
                );
            }
        }
    } else {
        ocr_skipped = true;
        store::update_doc_status(db, &document_id, &DocumentStatus::Ingested).map_err(storage)?;
    }

    Ok(IngestFileResult {
        document_id,
        was_new: true,
        ocr_skipped,
        pipeline_failed,
        warnings: outcome.warnings,
        image_embeddings_stored: outcome.image_embeddings_stored,
        vlm_descriptions: outcome.vlm_descriptions,
        pdf_embedded_images_extracted: outcome.pdf_embedded_images_extracted,
        pdf_embedded_images_skipped_filter: outcome.pdf_embedded_images_skipped_filter,
        pdf_image_ocr_runs: outcome.pdf_image_ocr_runs,
        pdf_image_vlm_failures: outcome.pdf_image_vlm_failures,
        pdf_image_ocr_failures: outcome.pdf_image_ocr_failures,
        pdf_pages_rendered: outcome.pdf_pages_rendered,
        pdf_coord: outcome.pdf_coord,
    })
}

fn media_type_suffix(media_type: &str) -> &'static str {
    match media_type {
        "text/markdown" => ".md",
        "text/html" | "application/xhtml+xml" => ".html",
        "application/json" | "application/ld+json" => ".json",
        "application/x-ndjson" => ".jsonl",
        "application/xml" | "application/rss+xml" | "application/atom+xml" => ".xml",
        "application/yaml" | "application/x-yaml" => ".yaml",
        "application/toml" => ".toml",
        "application/pdf" => ".pdf",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => ".xlsx",
        "application/vnd.ms-excel" => ".xls",
        "text/csv" => ".csv",
        "text/tab-separated-values" => ".tsv",
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/tiff" => ".tiff",
        _ => ".txt",
    }
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

    #[tokio::test]
    async fn byte_source_keeps_original_source_path() {
        let db = test_db();
        let result = ingest_bytes_source_with_policy(
            &db,
            "https://example.test/policy.txt",
            "text/plain",
            b"Archon keeps URL provenance.",
            &archon_policy::EffectivePolicy::default(),
        )
        .await
        .unwrap();
        let doc = store::get_doc_source(&db, &result.document_id)
            .unwrap()
            .unwrap();
        let chunks = store::list_chunks_for_doc(&db, &result.document_id).unwrap();

        assert!(result.was_new);
        assert_eq!(doc.source_path, "https://example.test/policy.txt");
        assert_eq!(doc.media_type, "text/plain");
        assert_eq!(doc.status, DocumentStatus::Ingested);
        assert_eq!(chunks.len(), 1);
    }

    #[tokio::test]
    async fn byte_source_deduplicates_by_content_hash() {
        let db = test_db();
        let first = ingest_bytes_source_with_policy(
            &db,
            "https://example.test/a.txt",
            "text/plain",
            b"same bytes",
            &archon_policy::EffectivePolicy::default(),
        )
        .await
        .unwrap();
        let second = ingest_bytes_source_with_policy(
            &db,
            "https://example.test/b.txt",
            "text/plain",
            b"same bytes",
            &archon_policy::EffectivePolicy::default(),
        )
        .await
        .unwrap();

        assert_eq!(first.document_id, second.document_id);
        assert!(!second.was_new);
    }

    #[tokio::test]
    async fn byte_source_accepts_csv_media() {
        let db = test_db();
        let result = ingest_bytes_source_with_policy(
            &db,
            "https://example.test/cases.csv",
            "text/csv",
            b"scenario,expected\nS-1,match\n",
            &archon_policy::EffectivePolicy::default(),
        )
        .await
        .unwrap();
        let doc = store::get_doc_source(&db, &result.document_id)
            .unwrap()
            .unwrap();
        let chunks = store::list_chunks_for_doc(&db, &result.document_id).unwrap();

        assert!(result.was_new);
        assert_eq!(doc.media_type, "text/csv");
        assert_eq!(doc.status, DocumentStatus::Ingested);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("| scenario | expected |"));
        assert!(chunks[0].content.contains("| S-1 | match |"));
    }

    #[tokio::test]
    async fn byte_source_accepts_structured_text_media() {
        let db = test_db();
        let result = ingest_bytes_source_with_policy(
            &db,
            "https://example.test/data.json",
            "application/json",
            br#"{"answer":42}"#,
            &archon_policy::EffectivePolicy::default(),
        )
        .await
        .unwrap();
        let doc = store::get_doc_source(&db, &result.document_id)
            .unwrap()
            .unwrap();
        let chunks = store::list_chunks_for_doc(&db, &result.document_id).unwrap();

        assert!(result.was_new);
        assert_eq!(doc.source_path, "https://example.test/data.json");
        assert_eq!(doc.media_type, "application/json");
        assert_eq!(doc.status, DocumentStatus::Ingested);
        assert_eq!(chunks.len(), 1);
    }
}
