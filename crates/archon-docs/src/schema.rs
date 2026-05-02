//! CozoDB relation definitions for document artefacts.
//!
//! Each `:create` call is idempotent: if the relation already exists
//! (from a prior call in the same process), the error is silently ignored.
//! All other DDL errors (typos, type mismatches, syntax errors) propagate.

use anyhow::Result;
use cozo::{DbInstance, ScriptMutability};

/// Ensure all document-core relations exist. Idempotent.
pub fn ensure_doc_schema(db: &DbInstance) -> Result<()> {
    ensure_doc_sources(db)?;
    ensure_doc_ocr_runs(db)?;
    ensure_doc_artifacts(db)?;
    ensure_doc_pages(db)?;
    ensure_doc_chunks(db)?;
    ensure_doc_provenance_edges(db)?;
    ensure_doc_processing_jobs(db)?;
    Ok(())
}

/// Run a `:create` script, ignoring "already exists" errors only.
fn run_create(db: &DbInstance, script: &str) -> Result<()> {
    match db.run_script(script, Default::default(), ScriptMutability::Mutable) {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("conflicts with an existing") || msg.contains("already exists") {
                Ok(())
            } else {
                Err(anyhow::anyhow!("schema creation failed: {msg}"))
            }
        }
    }
}

fn ensure_doc_sources(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_sources {
            document_id: String =>
            source_path: String,
            media_type: String,
            content_hash: String,
            discovered_at: String,
            status: String,
        }"#,
    )
}

fn ensure_doc_ocr_runs(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_ocr_runs {
            ocr_run_id: String =>
            document_id: String,
            provider: String,
            mode: String,
            status: String,
            started_at: String,
            completed_at: String default "",
            duration_ms: Int default 0,
        }"#,
    )
}

fn ensure_doc_artifacts(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_artifacts {
            artifact_id: String =>
            document_id: String,
            artifact_type: String,
            content_hash: String,
            created_at: String,
            provenance_record_id: String default "",
        }"#,
    )
}

fn ensure_doc_pages(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_pages {
            page_id: String =>
            document_id: String,
            page_number: Int,
            text_hash: String default "",
            image_hash: String default "",
            width: Float default 0.0,
            height: Float default 0.0,
            provenance_record_id: String,
        }"#,
    )
}

fn ensure_doc_chunks(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_chunks {
            chunk_id: String =>
            document_id: String,
            artifact_id: String,
            chunk_index: Int,
            page_start: Int,
            page_end: Int,
            content: String,
            content_hash: String,
            embedding_status: String default "pending",
        }"#,
    )
}

fn ensure_doc_provenance_edges(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_provenance_edges {
            edge_id: String =>
            from_artifact_id: String,
            to_artifact_id: String,
            edge_type: String,
            created_at: String,
        }"#,
    )
}

fn ensure_doc_processing_jobs(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_processing_jobs {
            job_id: String =>
            document_id: String,
            job_type: String,
            status: String,
            started_at: String,
            completed_at: String default "",
            error_message: String default "",
        }"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        DbInstance::new("mem", "", Default::default()).unwrap()
    }

    #[test]
    fn test_ensure_schema_idempotent() {
        let db = test_db();
        ensure_doc_schema(&db).unwrap();
        ensure_doc_schema(&db).unwrap(); // second call must not panic
    }

    #[test]
    fn test_run_create_propagates_real_errors() {
        let db = test_db();
        // Syntax error should propagate, not be silently swallowed
        let result = run_create(&db, ":create bad_syntax {");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(!msg.contains("conflicts with an existing"));
        assert!(!msg.contains("already exists"));
    }

    #[test]
    fn test_run_create_ignores_both_already_exists_phrasings() {
        let db = test_db();
        // Create once — succeeds
        run_create(&db, ":create phrasing_test { id: String => val: String }").unwrap();
        // Create again — fails with an "already exists" error, must be ignored
        run_create(&db, ":create phrasing_test { id: String => val: String }").unwrap();
        // If we got here, the error was correctly suppressed
    }
}
