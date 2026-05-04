//! Document and processing status queries.

use anyhow::Result;
use cozo::DbInstance;

use crate::models::{DocumentStatus, ProcessingJob};
use crate::store;

/// Summary status for all documents in the system.
#[derive(Clone, Debug)]
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
        let chunks = store::list_chunks_for_doc(db, &doc.document_id)?;
        summary.total_chunks += chunks.len();
        let pages = store::list_pages_for_doc(db, &doc.document_id)?;
        summary.total_pages += pages.len();
    }

    Ok(summary)
}

impl Default for DocStatusSummary {
    fn default() -> Self {
        Self {
            total_sources: 0,
            discovered: 0,
            ingesting: 0,
            ingested: 0,
            processing: 0,
            processed: 0,
            failed: 0,
            total_chunks: 0,
            total_pages: 0,
        }
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
