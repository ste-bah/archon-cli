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

/// Permanently delete a document and all its derived data (chunks, pages, vectors,
/// sentences, spatial rows, etc.). The content-hash dedup entry is released so the
/// same file can be re-ingested at a new path. Irreversible.
pub fn delete_document(db: &DbInstance, document_id: &str) -> Result<ClearedEvidence, DocsError> {
    crate::schema::ensure_doc_schema(db).map_err(storage)?;
    if crate::store::get_doc_source(db, document_id)
        .map_err(storage)?
        .is_none()
    {
        return Err(DocsError::Validation {
            message: format!("document not found: {document_id}"),
        });
    }
    let cleared = clear_generated_evidence(db, document_id)?;
    // Remove KB memberships and the source row itself.
    let p = params(document_id);
    run_rm_optional(
        db,
        "?[kb_id, document_id] := *doc_kb_memberships{kb_id, document_id}, document_id = $did
         :rm doc_kb_memberships { kb_id, document_id }",
        p.clone(),
        "doc_kb_memberships",
    )?;
    remove_doc_rows(db, "doc_sources", "document_id", p)?;
    Ok(cleared)
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
        Ok(outcome) => outcome,
        Err(err) => {
            pipeline_failed = true;
            store::update_doc_status(db, document_id, &DocumentStatus::Failed).map_err(storage)?;
            update_reprocess_job(db, &job_id, document_id, "failed", Some(&err.to_string()))?;
            let mut outcome = crate::ingest::PipelineOutcome::default();
            outcome.warnings.push(format!("reprocess failed: {err}"));
            outcome
        }
    };
    // GATE PARITY WITH FRESH INGEST (order matters, mirrors ingest.rs): the pipeline
    // succeeding is not the same as the document being sound. Sentence layer is rebuilt
    // BEFORE any status flip (S2 invariant: Ingested ⇒ sentence layer matches the text;
    // clear_generated_evidence dropped the old rows), then the same admissibility gate
    // that guards fresh ingest decides Ingested vs Failed — a zero-chunk or otherwise
    // degenerate reprocess must land Failed, never a warn-and-Ingested (the Goharinejad
    // class: "Ingested" with no chunks, invisible to coverage probes).
    if !pipeline_failed {
        let s = crate::sentence_index::rebuild_document(db, document_id)?;
        outcome.warnings.push(format!(
            "sentence layer: {} sentences ({} bbox, {} page) across {} chunks",
            s.sentences, s.with_bbox, s.with_page, s.chunks
        ));
        let adm = crate::admissibility::check_document(
            db,
            document_id,
            !crate::ingest::is_image_media_type(&doc.media_type),
            outcome.pdf_marker_fallback,
        )?;
        outcome.warnings.extend(adm.warnings.iter().cloned());
        if adm.failures.is_empty() {
            store::update_doc_status(db, document_id, &DocumentStatus::Ingested)
                .map_err(storage)?;
            update_reprocess_job(db, &job_id, document_id, "completed", None)?;
        } else {
            pipeline_failed = true;
            for failure in &adm.failures {
                outcome.warnings.push(format!("ADMISSIBILITY: {failure}"));
            }
            store::update_doc_status(db, document_id, &DocumentStatus::Failed).map_err(storage)?;
            update_reprocess_job(
                db,
                &job_id,
                document_id,
                "failed",
                Some(&adm.failures.join("; ")),
            )?;
        }
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
            pdf_coord: outcome.pdf_coord,
        },
    })
}

/// Remove every row the ingest pipeline generated for `document_id`, leaving the registration
/// rows (`doc_sources`, KB membership) intact so the document can be re-run in place.
///
/// Shared with [`crate::delete`], which clears the same evidence and then drops the registration
/// rows this deliberately preserves.
pub(crate) fn clear_generated_evidence(
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
    // CLIP image/figure embeddings are page-keyed (page-{doc}-N and per-figure {page_id}-imgM),
    // all sharing the "page-{document_id}-" prefix. Remove them so reprocess doesn't leak/orphan
    // image vectors (which `search-images` would otherwise still return as stale results).
    {
        let mut image_params = params.clone();
        image_params.insert(
            "img_prefix".into(),
            DataValue::from(format!("page-{document_id}-").as_str()),
        );
        run_rm_optional(
            db,
            "?[page_id] := *vec_page_images{page_id}, starts_with(page_id, $img_prefix)
             :rm vec_page_images { page_id }",
            image_params,
            "vec_page_images",
        )?;
    }
    // Chunk-keyed satellites have no document_id column → join through doc_chunks and
    // remove BEFORE the doc_chunks rows are deleted below (else the join finds nothing).
    run_rm_optional(
        db,
        "?[chunk_id] := *doc_chunks{chunk_id, document_id}, document_id = $did
         :rm doc_chunk_spatial { chunk_id }",
        params.clone(),
        "doc_chunk_spatial",
    )?;
    run_rm_optional(
        db,
        "?[chunk_id] := *doc_chunks{chunk_id, document_id}, document_id = $did
         :rm doc_chunk_hashes { chunk_id }",
        params.clone(),
        "doc_chunk_hashes",
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
        ("doc_locators", "locator_id"),
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
#[path = "reprocess_tests.rs"]
mod tests;
