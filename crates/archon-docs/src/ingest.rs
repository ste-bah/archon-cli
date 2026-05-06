//! Document ingestion engine.
//!
//! File-type detection, direct text/Markdown ingest, directory walking,
//! duplicate detection by content hash, and the full OCR → chunk →
//! page → provenance pipeline (Phase 1).
//!
//! Implements REQ-DOCS-001, REQ-DOCS-002, REQ-DOCS-004.

use std::fs;
use std::path::Path;

use cozo::DbInstance;
use tracing::info;

use crate::chunking::{build_chunk_artifacts, chunk_with_page_anchors};
use crate::errors::DocsError;
use crate::hash::{sha256_hex, sha256_str};
use crate::models::{
    ArtifactRecord, DocumentStatus, OcrRun, OcrStatus, PageArtifact, ProcessingJob, SourceDocument,
};
use crate::ocr::local::LocalOcrProvider;
use crate::ocr::provider::{self as ocr_provider, OcrProvider, OcrRequest};
use crate::provenance::build_doc_lineage_edges;
use crate::schema::ensure_doc_schema;
use crate::store::{self, hash_exists_in_sources};

use crate::embed;
pub use crate::ingest_directory::{IngestResult, ingest_directory, ingest_directory_with_policy};
use crate::ingest_multimodal::{apply_vlm_description, store_image_embedding_if_supported};
use crate::retrieval;

/// Result of a single-file ingest.
#[derive(Clone, Debug)]
pub struct IngestFileResult {
    pub document_id: String,
    pub was_new: bool,
    pub ocr_skipped: bool,
    pub pipeline_failed: bool,
    pub warnings: Vec<String>,
    pub image_embeddings_stored: usize,
    pub vlm_descriptions: usize,
    pub pdf_embedded_images_extracted: usize,
    pub pdf_embedded_images_skipped_filter: usize,
    pub pdf_image_ocr_runs: usize,
    pub pdf_image_vlm_failures: usize,
    pub pdf_image_ocr_failures: usize,
    pub pdf_pages_rendered: usize,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PipelineOutcome {
    pub(crate) warnings: Vec<String>,
    pub(crate) image_embeddings_stored: usize,
    pub(crate) vlm_descriptions: usize,
    pub(crate) pdf_embedded_images_extracted: usize,
    pub(crate) pdf_embedded_images_skipped_filter: usize,
    pub(crate) pdf_image_ocr_runs: usize,
    pub(crate) pdf_image_vlm_failures: usize,
    pub(crate) pdf_image_ocr_failures: usize,
    pub(crate) pdf_pages_rendered: usize,
}

/// Detect media type from file extension.
pub fn detect_media_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "tiff" | "tif" => "image/tiff",
        "md" | "markdown" => "text/markdown",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Determine whether a media type is supported for Phase 1 ingest.
pub fn is_supported_media_type(media_type: &str) -> bool {
    matches!(
        media_type,
        "text/plain"
            | "text/markdown"
            | "application/pdf"
            | "image/png"
            | "image/jpeg"
            | "image/tiff"
    )
}

/// Determine whether the media type can go through the OCR → chunk pipeline.
fn is_ocr_runnable(media_type: &str) -> bool {
    matches!(
        media_type,
        "text/plain"
            | "text/markdown"
            | "application/pdf"
            | "image/png"
            | "image/jpeg"
            | "image/tiff"
    )
}

pub(crate) fn is_image_media_type(media_type: &str) -> bool {
    matches!(media_type, "image/png" | "image/jpeg" | "image/tiff")
}

/// Ingest a single file: register document source, run OCR + chunk +
/// page + provenance pipeline, update status to Ingested.
pub async fn ingest_file(db: &DbInstance, path: &Path) -> Result<IngestFileResult, DocsError> {
    ingest_file_with_policy(db, path, &archon_policy::EffectivePolicy::default()).await
}

/// Ingest a single file with an explicit policy. The policy gates optional
/// multimodal enrichment such as VLM descriptions.
pub async fn ingest_file_with_policy(
    db: &DbInstance,
    path: &Path,
    policy: &archon_policy::EffectivePolicy,
) -> Result<IngestFileResult, DocsError> {
    ensure_doc_schema(db).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    let source_path = path.to_string_lossy().to_string();
    let media_type = detect_media_type(path);

    if !is_supported_media_type(media_type) {
        return Err(DocsError::UnsupportedMediaType {
            media_type: media_type.to_string(),
        });
    }

    // Read and hash file content
    let content_bytes = fs::read(path).map_err(|e| DocsError::OcrFile {
        path: source_path.clone(),
        message: e.to_string(),
    })?;

    let content_hash = sha256_hex(&content_bytes);

    // Check for duplicate by content hash
    if hash_exists_in_sources(db, &content_hash).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })? {
        // Return existing document_id so callers can link to it
        let existing_id = store::get_doc_by_hash(db, &content_hash)
            .map_err(|e| DocsError::Storage {
                message: e.to_string(),
            })?
            .map(|d| d.document_id)
            .unwrap_or_default();
        info!(
            path = %source_path,
            hash = %content_hash,
            existing = %existing_id,
            "Skipping duplicate document"
        );
        return Ok(IngestFileResult {
            document_id: existing_id,
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
        });
    }

    // Register the source document
    let document_id = format!("doc-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().to_rfc3339();

    let doc = SourceDocument {
        document_id: document_id.clone(),
        source_path: source_path.clone(),
        media_type: media_type.to_string(),
        content_hash: content_hash.clone(),
        discovered_at: now.clone(),
        status: DocumentStatus::Discovered,
    };

    store::insert_doc_source(db, &doc).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    // Create processing job
    let job = ProcessingJob {
        job_id: format!("job-{}", uuid::Uuid::new_v4()),
        document_id: document_id.clone(),
        job_type: "ingest".to_string(),
        status: "pending".to_string(),
        started_at: now,
        completed_at: None,
        error_message: None,
    };
    store::insert_processing_job(db, &job).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    // ── OCR → chunk → page → provenance pipeline ──────────────────
    let mut ocr_skipped = false;
    let mut pipeline_failed = false;
    let mut outcome = PipelineOutcome::default();
    if is_ocr_runnable(media_type) {
        // Transition to Ingesting before starting the pipeline
        store::update_doc_status(db, &document_id, &DocumentStatus::Ingesting).map_err(|e| {
            DocsError::Storage {
                message: e.to_string(),
            }
        })?;

        match run_ingest_pipeline_with_bytes(
            db,
            &document_id,
            &source_path,
            media_type,
            &content_bytes,
            policy,
        )
        .await
        {
            Ok(pipeline_outcome) => {
                outcome = pipeline_outcome;
                store::update_doc_status(db, &document_id, &DocumentStatus::Ingested).map_err(
                    |e| DocsError::Storage {
                        message: e.to_string(),
                    },
                )?;
            }
            Err(e) => {
                pipeline_failed = true;
                outcome.warnings.push(format!("OCR pipeline failed: {e}"));
                store::update_doc_status(db, &document_id, &DocumentStatus::Failed).map_err(
                    |e| DocsError::Storage {
                        message: e.to_string(),
                    },
                )?;
                info!(
                    document_id = %document_id,
                    error = %e,
                    "OCR pipeline failed; document set to Failed"
                );
                // Don't fail the ingest — document is registered, pipeline can be retried
            }
        }
    } else {
        ocr_skipped = true;
        tracing::warn!(
            document_id = %document_id,
            path = %source_path,
            media_type = %media_type,
            "OCR pipeline skipped for supported non-OCR media"
        );
        store::update_doc_status(db, &document_id, &DocumentStatus::Ingested).map_err(|e| {
            DocsError::Storage {
                message: e.to_string(),
            }
        })?;
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
    })
}

/// Run the full OCR → page records → chunking → provenance pipeline
/// for a single document. All inserts go through the Cozo `db`.
async fn run_ingest_pipeline_with_bytes(
    db: &DbInstance,
    document_id: &str,
    file_path: &str,
    media_type: &str,
    content_bytes: &[u8],
    policy: &archon_policy::EffectivePolicy,
) -> Result<PipelineOutcome, DocsError> {
    let local_provider = LocalOcrProvider;
    let configured_provider = ocr_provider::get_provider();
    let provider: &dyn OcrProvider = configured_provider.as_deref().unwrap_or(&local_provider);
    let ocr_run_id = format!("ocr-{}", uuid::Uuid::new_v4());
    let mut outcome = PipelineOutcome::default();

    // 1. Create OCR run (Running)
    let ocr_run = OcrRun {
        ocr_run_id: ocr_run_id.clone(),
        document_id: document_id.to_string(),
        provider: provider.name().to_string(),
        mode: media_type.to_string(),
        status: OcrStatus::Running,
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        duration_ms: None,
    };
    store::insert_ocr_run(db, &ocr_run).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    if media_type == "application/pdf" {
        return crate::ingest_pdf::run_pdf_ingest_pipeline(
            db,
            document_id,
            file_path,
            &ocr_run_id,
            policy,
        )
        .await;
    }

    // 2. Run OCR extraction
    let request = OcrRequest {
        file_path: file_path.to_string(),
        document_id: document_id.to_string(),
        ocr_run_id: ocr_run_id.clone(),
        page_range: None,
        language_hint: None,
    };

    let extract_result = match provider.extract(request).await {
        Ok(result) => result,
        Err(e) => {
            // Mark OCR run as Failed before propagating
            let _ = store::update_ocr_run_completion(
                db,
                &ocr_run_id,
                &OcrStatus::Failed,
                &chrono::Utc::now().to_rfc3339(),
                0,
            );
            return Err(e);
        }
    };

    // Update OCR run to Completed
    let completed_at = chrono::Utc::now().to_rfc3339();
    store::update_ocr_run_completion(
        db,
        &ocr_run_id,
        &OcrStatus::Completed,
        &completed_at,
        extract_result.processing_duration_ms,
    )
    .map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    let full_text = extract_result.full_text;
    let page_offsets = extract_result.page_offsets;

    // 3. Create page records from page_offsets
    let mut page_ids: Vec<String> = Vec::new();
    for po in &page_offsets {
        let page_id = format!("page-{}-{}", document_id, po.page);
        let page_text = &full_text[po.char_start..po.char_end];
        let page = PageArtifact {
            page_id: page_id.clone(),
            document_id: document_id.to_string(),
            page_number: po.page,
            text_hash: Some(sha256_str(page_text)),
            image_hash: if is_image_media_type(media_type) {
                Some(sha256_hex(content_bytes))
            } else {
                None
            },
            width: None,
            height: None,
            provenance_record_id: String::new(),
        };
        store::insert_page(db, &page).map_err(|e| DocsError::Storage {
            message: e.to_string(),
        })?;
        page_ids.push(page_id);
    }

    // 4. Create OCR-result artifact for provenance chain
    let ocr_artifact_id = format!("ocr-result-{}", ocr_run_id);
    let ocr_artifact = ArtifactRecord {
        artifact_id: ocr_artifact_id.clone(),
        document_id: document_id.to_string(),
        artifact_type: "ocr_text".to_string(),
        content_hash: sha256_str(&full_text),
        created_at: chrono::Utc::now().to_rfc3339(),
        provenance_record_id: String::new(),
    };
    store::insert_artifact(db, &ocr_artifact).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    // 5. Chunk text with page anchors
    let page_chunks = chunk_with_page_anchors(&full_text, &page_offsets);

    // 6. Build chunk artifacts keyed to the OCR-result artifact
    let chunk_artifacts = build_chunk_artifacts(document_id, &ocr_artifact_id, &page_chunks);
    for chunk in &chunk_artifacts {
        store::insert_chunk(db, chunk).map_err(|e| DocsError::Storage {
            message: e.to_string(),
        })?;
    }

    // 7. Build and store provenance edges (chunk→page, artifact→document)
    let edges = build_doc_lineage_edges(document_id, &ocr_artifact_id, &chunk_artifacts, &page_ids);
    for edge in &edges {
        store::insert_provenance_edge(db, edge).map_err(|e| DocsError::Storage {
            message: e.to_string(),
        })?;
    }

    if is_image_media_type(media_type) {
        apply_vlm_description(
            db,
            document_id,
            content_bytes,
            policy,
            &page_ids,
            &mut outcome,
        )
        .await?;
    }

    // 8. Eager indexing: embed and store vectors if a provider is configured.
    if let Some(_provider) = embed::get_provider() {
        for chunk in &chunk_artifacts {
            if let Err(e) = retrieval::index_chunk(db, chunk) {
                tracing::warn!(
                    chunk_id = %chunk.chunk_id,
                    error = %e,
                    "failed to index chunk during ingest"
                );
            }
        }
    } else {
        tracing::info!("no embedding provider configured; skipping eager indexing during ingest");
    }

    if is_image_media_type(media_type) {
        store_image_embedding_if_supported(
            db,
            &page_ids,
            content_bytes,
            outcome.vlm_descriptions > 0,
            &mut outcome,
        );
    }

    Ok(outcome)
}

#[cfg(test)]
#[path = "ingest_embedding_tests.rs"]
mod embedding_tests;
#[cfg(test)]
#[path = "ingest_multimodal_tests.rs"]
mod multimodal_tests;
#[cfg(test)]
#[path = "ingest_pdf_ingest_tests.rs"]
mod pdf_ingest_tests;
#[cfg(test)]
#[path = "ingest_test_support.rs"]
mod test_support;
#[cfg(test)]
#[path = "ingest_tests.rs"]
mod tests;
