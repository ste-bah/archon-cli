use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::errors::{COZO_RELATION_NOT_FOUND, DocsError};
use crate::hash::sha256_hex;
use crate::ingest::{IngestFileResult, run_ingest_pipeline_with_bytes};
use crate::models::{DocumentStatus, ProcessingJob};
use crate::schema::ensure_doc_schema;
use crate::store;

#[derive(Clone, Debug, Default)]
pub struct ClearedEvidence {
    pub chunks: usize,
    pub pages: usize,
    pub artifacts: usize,
    pub image_descriptions: usize,
}

#[derive(Clone, Debug)]
pub struct ReprocessDocumentResult {
    pub ingest: IngestFileResult,
    pub source_path: String,
    pub cleared: ClearedEvidence,
}

pub async fn reprocess_document_with_policy(
    db: &DbInstance,
    document_id: &str,
    policy: &archon_policy::EffectivePolicy,
) -> Result<ReprocessDocumentResult, DocsError> {
    ensure_doc_schema(db).map_err(storage)?;
    let doc = store::get_doc_source(db, document_id)
        .map_err(storage)?
        .ok_or_else(|| DocsError::Validation {
            message: format!("document not found: {document_id}"),
        })?;
    let path = Path::new(&doc.source_path);
    if !path.exists() {
        return Err(DocsError::OcrFile {
            path: doc.source_path.clone(),
            message: "source file no longer exists; cannot reprocess in place".into(),
        });
    }
    let content_bytes = fs::read(path).map_err(|e| DocsError::OcrFile {
        path: doc.source_path.clone(),
        message: e.to_string(),
    })?;
    let current_hash = sha256_hex(&content_bytes);
    if current_hash != doc.content_hash {
        return Err(DocsError::Validation {
            message: format!(
                "source file content changed for {}; ingest it as a new document instead",
                doc.source_path
            ),
        });
    }

    let cleared = clear_generated_evidence(db, document_id)?;
    store::update_doc_status(db, document_id, &DocumentStatus::Ingesting).map_err(storage)?;
    let job_id = insert_reprocess_job(db, document_id, "running", None)?;

    let mut pipeline_failed = false;
    let mut outcome = match run_ingest_pipeline_with_bytes(
        db,
        document_id,
        &doc.source_path,
        &doc.media_type,
        &content_bytes,
        policy,
    )
    .await
    {
        Ok(outcome) => {
            store::update_doc_status(db, document_id, &DocumentStatus::Ingested)
                .map_err(storage)?;
            update_reprocess_job(db, &job_id, document_id, "completed", None)?;
            outcome
        }
        Err(err) => {
            pipeline_failed = true;
            store::update_doc_status(db, document_id, &DocumentStatus::Failed).map_err(storage)?;
            update_reprocess_job(db, &job_id, document_id, "failed", Some(&err.to_string()))?;
            let mut outcome = crate::ingest::PipelineOutcome::default();
            outcome.warnings.push(format!("reprocess failed: {err}"));
            outcome
        }
    };
    let chunks_registered = store::list_chunks_for_doc(db, document_id)
        .map_err(storage)?
        .len();
    if chunks_registered == 0 && !pipeline_failed {
        outcome
            .warnings
            .push("reprocess completed but produced no chunks".into());
    }

    Ok(ReprocessDocumentResult {
        source_path: doc.source_path,
        cleared,
        ingest: IngestFileResult {
            document_id: document_id.to_string(),
            was_new: false,
            ocr_skipped: false,
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
        },
    })
}

fn clear_generated_evidence(
    db: &DbInstance,
    document_id: &str,
) -> Result<ClearedEvidence, DocsError> {
    let cleared = ClearedEvidence {
        chunks: store::list_chunks_for_doc(db, document_id)
            .map_err(storage)?
            .len(),
        pages: store::list_pages_for_doc(db, document_id)
            .map_err(storage)?
            .len(),
        artifacts: store::list_artifacts_for_doc(db, document_id)
            .map_err(storage)?
            .len(),
        image_descriptions: store::list_image_descriptions_for_doc(db, document_id)
            .map_err(storage)?
            .len(),
    };
    remove_generated_rows(db, document_id)?;
    Ok(cleared)
}

fn remove_generated_rows(db: &DbInstance, document_id: &str) -> Result<(), DocsError> {
    crate::index_queue::remove_document_queue_rows(db, document_id).map_err(storage)?;
    let params = params(document_id);
    run_rm_optional(
        db,
        "?[chunk_id] := *doc_chunks{chunk_id, document_id}, document_id = $did
         :rm vec_text_chunks { chunk_id }",
        params.clone(),
        "vec_text_chunks",
    )?;
    for (relation, key) in [
        ("doc_artifacts", "artifact_id"),
        ("doc_pages", "page_id"),
        ("doc_chunks", "chunk_id"),
        ("doc_image_descriptions", "artifact_id"),
    ] {
        remove_provenance_edges_for_targets(db, relation, key, &params)?;
    }
    for (relation, key) in [
        ("doc_image_descriptions", "artifact_id"),
        ("doc_pdf_metrics", "document_id"),
        ("doc_processing_jobs", "job_id"),
        ("doc_ocr_runs", "ocr_run_id"),
        ("doc_pages", "page_id"),
        ("doc_artifacts", "artifact_id"),
        ("doc_chunks", "chunk_id"),
    ] {
        remove_doc_rows(db, relation, key, params.clone())?;
    }
    remove_direct_document_edges(db, params)?;
    Ok(())
}

fn remove_provenance_edges_for_targets(
    db: &DbInstance,
    relation: &str,
    key: &str,
    params: &BTreeMap<String, DataValue>,
) -> Result<(), DocsError> {
    for edge_column in ["from_artifact_id", "to_artifact_id"] {
        run_rm(
            db,
            &format!(
                "?[edge_id] := *{relation}{{{key}: target_id, document_id}}, document_id = $did,
                 *doc_provenance_edges{{edge_id, {edge_column}: target_id}}
                 :rm doc_provenance_edges {{ edge_id }}"
            ),
            params.clone(),
            "doc_provenance_edges",
        )?;
    }
    Ok(())
}

fn remove_doc_rows(
    db: &DbInstance,
    relation: &str,
    key: &str,
    params: BTreeMap<String, DataValue>,
) -> Result<(), DocsError> {
    let script = if key == "document_id" {
        format!(
            "?[document_id] <- [[$did]]
             :rm {relation} {{ document_id }}"
        )
    } else {
        format!(
            "?[{key}] := *{relation}{{{key}, document_id}}, document_id = $did
             :rm {relation} {{ {key} }}"
        )
    };
    run_rm(db, &script, params, relation)
}

fn remove_direct_document_edges(
    db: &DbInstance,
    params: BTreeMap<String, DataValue>,
) -> Result<(), DocsError> {
    for edge_column in ["from_artifact_id", "to_artifact_id"] {
        run_rm(
            db,
            &format!(
                "?[edge_id] := *doc_provenance_edges{{edge_id, {edge_column}}}, {edge_column} = $did
                 :rm doc_provenance_edges {{ edge_id }}"
            ),
            params.clone(),
            "doc_provenance_edges",
        )?;
    }
    Ok(())
}

fn insert_reprocess_job(
    db: &DbInstance,
    document_id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<String, DocsError> {
    let job_id = format!("job-{}", uuid::Uuid::new_v4());
    let job = ProcessingJob {
        job_id: job_id.clone(),
        document_id: document_id.to_string(),
        job_type: "reprocess".into(),
        status: status.into(),
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        error_message: error.map(ToString::to_string),
    };
    store::insert_processing_job(db, &job).map_err(storage)?;
    Ok(job_id)
}

fn update_reprocess_job(
    db: &DbInstance,
    job_id: &str,
    document_id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<(), DocsError> {
    let job = ProcessingJob {
        job_id: job_id.to_string(),
        document_id: document_id.to_string(),
        job_type: "reprocess".into(),
        status: status.into(),
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: Some(chrono::Utc::now().to_rfc3339()),
        error_message: error.map(ToString::to_string),
    };
    store::insert_processing_job(db, &job).map_err(storage)
}

fn params(document_id: &str) -> BTreeMap<String, DataValue> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));
    params
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

fn run_rm_optional(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    label: &str,
) -> Result<(), DocsError> {
    match crate::cozo_retry::run_script_guarded(
        db,
        script,
        params,
        ScriptMutability::Mutable,
        &format!("delete {label} rows"),
    ) {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().contains(COZO_RELATION_NOT_FOUND) => Ok(()),
        Err(e) => Err(DocsError::Storage {
            message: format!("delete {label} rows failed: {e}"),
        }),
    }
}

fn storage(error: impl std::fmt::Display) -> DocsError {
    DocsError::Storage {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use cozo::DbInstance;

    use super::*;
    use crate::ingest::ingest_file_with_policy;

    fn test_db() -> DbInstance {
        let db = DbInstance::new("mem", "", "").unwrap();
        ensure_doc_schema(&db).unwrap();
        db
    }

    #[tokio::test]
    async fn reprocess_preserves_source_and_kb_membership() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("elliott-notes.md");
        fs::write(&path, "Wave one starts the impulse.\nWave two retraces.\n").unwrap();
        let policy = archon_policy::EffectivePolicy::default();

        let original = ingest_file_with_policy(&db, &path, &policy).await.unwrap();
        store::assign_document_to_kb(&db, "trading-elliott-wave", &original.document_id).unwrap();
        let old_chunks = store::list_chunks_for_doc(&db, &original.document_id).unwrap();
        assert!(!old_chunks.is_empty());

        let result = reprocess_document_with_policy(&db, &original.document_id, &policy)
            .await
            .unwrap();

        assert_eq!(result.ingest.document_id, original.document_id);
        assert!(!result.ingest.was_new);
        assert_eq!(result.cleared.chunks, old_chunks.len());
        let doc = store::get_doc_source(&db, &original.document_id)
            .unwrap()
            .unwrap();
        assert_eq!(doc.status, DocumentStatus::Ingested);
        let kb_docs = store::list_kb_document_ids(&db, "trading-elliott-wave").unwrap();
        assert_eq!(kb_docs, vec![original.document_id.clone()]);
        let new_chunks = store::list_chunks_for_doc(&db, &original.document_id).unwrap();
        assert!(!new_chunks.is_empty());
        assert_eq!(old_chunks[0].content, new_chunks[0].content);
    }

    #[tokio::test]
    async fn reprocess_rejects_changed_source_content() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.md");
        fs::write(&path, "original").unwrap();
        let policy = archon_policy::EffectivePolicy::default();
        let original = ingest_file_with_policy(&db, &path, &policy).await.unwrap();

        fs::write(&path, "changed").unwrap();
        let err = reprocess_document_with_policy(&db, &original.document_id, &policy)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("source file content changed"));
        let chunks = store::list_chunks_for_doc(&db, &original.document_id).unwrap();
        assert_eq!(chunks.len(), 1);
    }
}
