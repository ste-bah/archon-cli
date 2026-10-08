//! Shared steps of the ingest paths for a hash reservation's outcomes.
use cozo::DbInstance;
use tracing::info;

use crate::errors::DocsError;
use crate::ingest::IngestFileResult;
use crate::models::SourceDocument;

/// The result for content another document already owns.
pub(crate) fn duplicate_result(document_id: String) -> IngestFileResult {
    IngestFileResult {
        document_id,
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
    }
}

/// Prepare an interrupted registration to be finished in place: the rows its
/// earlier ingest wrote before it stopped are removed, the registration stays.
pub(crate) fn resume_registration(
    db: &DbInstance,
    existing: &SourceDocument,
    source: &str,
) -> Result<(), DocsError> {
    info!(
        document_id = %existing.document_id,
        source,
        status = ?existing.status,
        "Resuming a registration whose earlier ingest stopped before its final status"
    );
    crate::reprocess::clear_generated_evidence(db, &existing.document_id)?;
    Ok(())
}
