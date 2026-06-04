//! Document and processing status queries.

use anyhow::Result;
use cozo::DbInstance;

use crate::models::{DocumentStatus, ProcessingJob};
use crate::store;

/// Summary status for all documents in the system.
#[derive(Clone, Debug, Default)]
pub struct DocStatusSummary {
    pub total_sources: usize,
    pub discovered: usize,
    pub ingesting: usize,
    pub ingested: usize,
    pub processing: usize,
    pub processed: usize,
    pub failed: usize,
    pub total_chunks: usize,
    pub total_pages: usize,
    pub pdf_embedded_images_extracted: usize,
    pub pdf_embedded_images_skipped_filter: usize,
    pub pdf_image_ocr_runs: usize,
    pub pdf_image_ocr_failures: usize,
    pub pdf_image_vlm_descriptions: usize,
    pub pdf_image_vlm_failures: usize,
    pub pdf_pages_rendered: usize,
}

/// Get a summary of all document state.
pub fn get_status_summary(db: &DbInstance) -> Result<DocStatusSummary> {
    let sources = store::list_doc_sources(db)?;
    let mut summary = DocStatusSummary {
        total_sources: sources.len(),
        ..Default::default()
    };

    for doc in &sources {
        match doc.status {
            DocumentStatus::Discovered => summary.discovered += 1,
            DocumentStatus::Ingesting => summary.ingesting += 1,
            DocumentStatus::Ingested => summary.ingested += 1,
            DocumentStatus::Processing => summary.processing += 1,
            DocumentStatus::Processed => summary.processed += 1,
            DocumentStatus::Failed => summary.failed += 1,
        }
    }

    summary.total_chunks = store::count_chunks(db)?;
    summary.total_pages = count_pages(db)?;
    for metrics in store::list_pdf_metrics(db)? {
        summary.pdf_embedded_images_extracted += metrics.embedded_images_extracted as usize;
        summary.pdf_embedded_images_skipped_filter +=
            metrics.embedded_images_skipped_filter as usize;
        summary.pdf_image_ocr_runs += metrics.image_ocr_runs as usize;
        summary.pdf_image_ocr_failures += metrics.image_ocr_failures as usize;
        summary.pdf_image_vlm_descriptions += metrics.image_vlm_descriptions as usize;
        summary.pdf_image_vlm_failures += metrics.image_vlm_failures as usize;
        summary.pdf_pages_rendered += metrics.pages_rendered as usize;
    }
    Ok(summary)
}

fn count_pages(db: &DbInstance) -> Result<usize> {
    let result = db.run_script(
        "?[count(page_id)] := *doc_pages{page_id}",
        Default::default(),
        cozo::ScriptMutability::Immutable,
    );
    match result {
        Ok(result) => Ok(result
            .rows
            .first()
            .and_then(|row| row.first())
            .and_then(|value| value.get_int())
            .unwrap_or(0) as usize),
        Err(e)
            if e.to_string()
                .contains(crate::errors::COZO_RELATION_NOT_FOUND) =>
        {
            Ok(0)
        }
        Err(e) => Err(anyhow::anyhow!("count pages failed: {e}")),
    }
}

/// Get all processing jobs.
pub fn list_processing_jobs(db: &DbInstance) -> Result<Vec<ProcessingJob>> {
    let result = db
        .run_script(
            "?[job_id, document_id, job_type, status, started_at, completed_at, error_message] \
             := *doc_processing_jobs{job_id, document_id, job_type, status, started_at, completed_at, error_message}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list processing jobs failed: {e}"))?;

    Ok(result
        .rows
        .iter()
        .map(|row| ProcessingJob {
            job_id: row[0].get_str().unwrap_or("").to_string(),
            document_id: row[1].get_str().unwrap_or("").to_string(),
            job_type: row[2].get_str().unwrap_or("").to_string(),
            status: row[3].get_str().unwrap_or("").to_string(),
            started_at: row[4].get_str().unwrap_or("").to_string(),
            completed_at: {
                let s = row[5].get_str().unwrap_or("");
                if s.is_empty() {
                    None
                } else {
                    Some(s.to_string())
                }
            },
            error_message: {
                let s = row[6].get_str().unwrap_or("");
                if s.is_empty() {
                    None
                } else {
                    Some(s.to_string())
                }
            },
        })
        .collect())
}
