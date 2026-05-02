//! CozoDB CRUD operations for document artefacts.
//!
//! Each function accepts a `&DbInstance` and the relevant model struct,
//! and performs insert or query operations against the canonical Cozo relations.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::models::{
    ChunkArtifact, DocumentStatus, OcrRun, OcrStatus, PageArtifact, ProcessingJob,
    ProvenanceEdge, SourceDocument,
};

// ---------------------------------------------------------------------------
// SourceDocument
// ---------------------------------------------------------------------------

pub fn insert_doc_source(db: &DbInstance, doc: &SourceDocument) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(doc.document_id.as_str()));
    params.insert("path".into(), DataValue::from(doc.source_path.as_str()));
    params.insert("mtype".into(), DataValue::from(doc.media_type.as_str()));
    params.insert("hash".into(), DataValue::from(doc.content_hash.as_str()));
    params.insert("dat".into(), DataValue::from(doc.discovered_at.as_str()));
    params.insert("status".into(), DataValue::from(status_str(&doc.status)));

    db.run_script(
        "?[document_id, source_path, media_type, content_hash, discovered_at, status] \
         <- [[$did, $path, $mtype, $hash, $dat, $status]] \
         :put doc_sources { document_id => source_path, media_type, content_hash, discovered_at, status }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert doc_sources failed: {e}"))?;
    Ok(())
}

pub fn get_doc_source(db: &DbInstance, document_id: &str) -> Result<Option<SourceDocument>> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));

    let result = db
        .run_script(
            "?[document_id, source_path, media_type, content_hash, discovered_at, status] \
             := *doc_sources{document_id, source_path, media_type, content_hash, discovered_at, status}, \
             document_id = $did",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get doc_sources failed: {e}"))?;

    if result.rows.is_empty() {
        return Ok(None);
    }
    let row = &result.rows[0];
    Ok(Some(SourceDocument {
        document_id: row[0].get_str().unwrap_or("").to_string(),
        source_path: row[1].get_str().unwrap_or("").to_string(),
        media_type: row[2].get_str().unwrap_or("").to_string(),
        content_hash: row[3].get_str().unwrap_or("").to_string(),
        discovered_at: row[4].get_str().unwrap_or("").to_string(),
        status: parse_status(row[5].get_str().unwrap_or("")),
    }))
}

pub fn list_doc_sources(db: &DbInstance) -> Result<Vec<SourceDocument>> {
    let result = db
        .run_script(
            "?[document_id, source_path, media_type, content_hash, discovered_at, status] \
             := *doc_sources{document_id, source_path, media_type, content_hash, discovered_at, status}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list doc_sources failed: {e}"))?;

    Ok(result
        .rows
        .iter()
        .map(|row| SourceDocument {
            document_id: row[0].get_str().unwrap_or("").to_string(),
            source_path: row[1].get_str().unwrap_or("").to_string(),
            media_type: row[2].get_str().unwrap_or("").to_string(),
            content_hash: row[3].get_str().unwrap_or("").to_string(),
            discovered_at: row[4].get_str().unwrap_or("").to_string(),
            status: parse_status(row[5].get_str().unwrap_or("")),
        })
        .collect())
}

pub fn hash_exists_in_sources(db: &DbInstance, content_hash: &str) -> Result<bool> {
    let mut params = BTreeMap::new();
    params.insert("ch".into(), DataValue::from(content_hash));
    let result = db
        .run_script(
            "?[document_id] := *doc_sources{document_id, content_hash}, content_hash = $ch",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("hash check failed: {e}"))?;
    Ok(!result.rows.is_empty())
}

/// Look up an existing document by content hash (for duplicate reporting).
pub fn get_doc_by_hash(db: &DbInstance, content_hash: &str) -> Result<Option<SourceDocument>> {
    let mut params = BTreeMap::new();
    params.insert("ch".into(), DataValue::from(content_hash));
    let result = db
        .run_script(
            "?[document_id, source_path, media_type, content_hash, discovered_at, status] \
             := *doc_sources{document_id, source_path, media_type, content_hash, discovered_at, status}, \
             content_hash = $ch",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get doc by hash failed: {e}"))?;
    if result.rows.is_empty() {
        return Ok(None);
    }
    let row = &result.rows[0];
    Ok(Some(SourceDocument {
        document_id: row[0].get_str().unwrap_or("").to_string(),
        source_path: row[1].get_str().unwrap_or("").to_string(),
        media_type: row[2].get_str().unwrap_or("").to_string(),
        content_hash: row[3].get_str().unwrap_or("").to_string(),
        discovered_at: row[4].get_str().unwrap_or("").to_string(),
        status: parse_status(row[5].get_str().unwrap_or("")),
    }))
}

/// Update the status field on an existing document.
pub fn update_doc_status(
    db: &DbInstance,
    document_id: &str,
    status: &DocumentStatus,
) -> Result<()> {
    let mut doc = get_doc_source(db, document_id)?
        .ok_or_else(|| anyhow::anyhow!("document not found: {document_id}"))?;
    doc.status = status.clone();
    insert_doc_source(db, &doc)
}

// ---------------------------------------------------------------------------
// OcrRun
// ---------------------------------------------------------------------------

pub fn insert_ocr_run(db: &DbInstance, run: &OcrRun) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("oid".into(), DataValue::from(run.ocr_run_id.as_str()));
    params.insert("did".into(), DataValue::from(run.document_id.as_str()));
    params.insert("prov".into(), DataValue::from(run.provider.as_str()));
    params.insert("mode".into(), DataValue::from(run.mode.as_str()));
    params.insert("status".into(), DataValue::from(ocr_status_str(&run.status)));
    params.insert("sat".into(), DataValue::from(run.started_at.as_str()));
    params.insert(
        "cat".into(),
        DataValue::from(run.completed_at.as_deref().unwrap_or("")),
    );
    params.insert("dur".into(), DataValue::from(run.duration_ms.unwrap_or(0) as i64));

    db.run_script(
        "?[ocr_run_id, document_id, provider, mode, status, started_at, completed_at, duration_ms] \
         <- [[$oid, $did, $prov, $mode, $status, $sat, $cat, $dur]] \
         :put doc_ocr_runs { ocr_run_id => document_id, provider, mode, status, started_at, completed_at, duration_ms }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert doc_ocr_runs failed: {e}"))?;
    Ok(())
}

/// Update an existing OCR run with completion data.
pub fn update_ocr_run_completion(
    db: &DbInstance,
    ocr_run_id: &str,
    status: &OcrStatus,
    completed_at: &str,
    duration_ms: u64,
) -> Result<()> {
    // Read back existing run, update fields, re-:put by key
    let runs = list_ocr_runs_for_ocr_id(db, ocr_run_id)?;
    let mut run = runs
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("OCR run not found: {ocr_run_id}"))?;
    run.status = status.clone();
    run.completed_at = Some(completed_at.to_string());
    run.duration_ms = if duration_ms == 0 { None } else { Some(duration_ms) };
    insert_ocr_run(db, &run)
}

/// Look up OCR runs by ocr_run_id (not just document_id).
fn list_ocr_runs_for_ocr_id(db: &DbInstance, ocr_run_id: &str) -> Result<Vec<OcrRun>> {
    let mut params = BTreeMap::new();
    params.insert("oid".into(), DataValue::from(ocr_run_id));
    let result = db
        .run_script(
            "?[ocr_run_id, document_id, provider, mode, status, started_at, completed_at, duration_ms] \
             := *doc_ocr_runs{ocr_run_id, document_id, provider, mode, status, started_at, completed_at, duration_ms}, \
             ocr_run_id = $oid",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list ocr_runs by id failed: {e}"))?;
    Ok(result
        .rows
        .iter()
        .map(|row| OcrRun {
            ocr_run_id: row[0].get_str().unwrap_or("").to_string(),
            document_id: row[1].get_str().unwrap_or("").to_string(),
            provider: row[2].get_str().unwrap_or("").to_string(),
            mode: row[3].get_str().unwrap_or("").to_string(),
            status: parse_ocr_status(row[4].get_str().unwrap_or("")),
            started_at: row[5].get_str().unwrap_or("").to_string(),
            completed_at: {
                let s = row[6].get_str().unwrap_or("");
                if s.is_empty() { None } else { Some(s.to_string()) }
            },
            duration_ms: {
                let d = row[7].get_int().unwrap_or(0);
                if d == 0 { None } else { Some(d as u64) }
            },
        })
        .collect())
}

pub fn list_ocr_runs_for_doc(db: &DbInstance, document_id: &str) -> Result<Vec<OcrRun>> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));

    let result = db
        .run_script(
            "?[ocr_run_id, document_id, provider, mode, status, started_at, completed_at, duration_ms] \
             := *doc_ocr_runs{ocr_run_id, document_id, provider, mode, status, started_at, completed_at, duration_ms}, \
             document_id = $did",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list ocr_runs failed: {e}"))?;

    Ok(result
        .rows
        .iter()
        .map(|row| OcrRun {
            ocr_run_id: row[0].get_str().unwrap_or("").to_string(),
            document_id: row[1].get_str().unwrap_or("").to_string(),
            provider: row[2].get_str().unwrap_or("").to_string(),
            mode: row[3].get_str().unwrap_or("").to_string(),
            status: parse_ocr_status(row[4].get_str().unwrap_or("")),
            started_at: row[5].get_str().unwrap_or("").to_string(),
            completed_at: {
                let s = row[6].get_str().unwrap_or("");
                if s.is_empty() { None } else { Some(s.to_string()) }
            },
            duration_ms: {
                let d = row[7].get_int().unwrap_or(0);
                if d == 0 { None } else { Some(d as u64) }
            },
        })
        .collect())
}

// ---------------------------------------------------------------------------
// PageArtifact
// ---------------------------------------------------------------------------

pub fn insert_page(db: &DbInstance, page: &PageArtifact) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("pid".into(), DataValue::from(page.page_id.as_str()));
    params.insert("did".into(), DataValue::from(page.document_id.as_str()));
    params.insert("pnum".into(), DataValue::from(page.page_number as i64));
    params.insert(
        "thash".into(),
        DataValue::from(page.text_hash.as_deref().unwrap_or("")),
    );
    params.insert(
        "ihash".into(),
        DataValue::from(page.image_hash.as_deref().unwrap_or("")),
    );
    params.insert("w".into(), DataValue::from(page.width.unwrap_or(0.0) as f64));
    params.insert("h".into(), DataValue::from(page.height.unwrap_or(0.0) as f64));
    params.insert(
        "prov".into(),
        DataValue::from(page.provenance_record_id.as_str()),
    );

    db.run_script(
        "?[page_id, document_id, page_number, text_hash, image_hash, width, height, provenance_record_id] \
         <- [[$pid, $did, $pnum, $thash, $ihash, $w, $h, $prov]] \
         :put doc_pages { page_id => document_id, page_number, text_hash, image_hash, width, height, provenance_record_id }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert doc_pages failed: {e}"))?;
    Ok(())
}

pub fn list_pages_for_doc(db: &DbInstance, document_id: &str) -> Result<Vec<PageArtifact>> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));

    let result = db
        .run_script(
            "?[page_id, document_id, page_number, text_hash, image_hash, width, height, provenance_record_id] \
             := *doc_pages{page_id, document_id, page_number, text_hash, image_hash, width, height, provenance_record_id}, \
             document_id = $did",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list pages failed: {e}"))?;

    Ok(result
        .rows
        .iter()
        .map(|row| PageArtifact {
            page_id: row[0].get_str().unwrap_or("").to_string(),
            document_id: row[1].get_str().unwrap_or("").to_string(),
            page_number: row[2].get_int().unwrap_or(0) as u32,
            text_hash: optional_str(row[3].get_str().unwrap_or("")),
            image_hash: optional_str(row[4].get_str().unwrap_or("")),
            width: {
                let v = row[5].get_float().unwrap_or(0.0);
                if v == 0.0 { None } else { Some(v as f32) }
            },
            height: {
                let v = row[6].get_float().unwrap_or(0.0);
                if v == 0.0 { None } else { Some(v as f32) }
            },
            provenance_record_id: row[7].get_str().unwrap_or("").to_string(),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// ChunkArtifact
// ---------------------------------------------------------------------------

pub fn insert_chunk(db: &DbInstance, chunk: &ChunkArtifact) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("cid".into(), DataValue::from(chunk.chunk_id.as_str()));
    params.insert("did".into(), DataValue::from(chunk.document_id.as_str()));
    params.insert("aid".into(), DataValue::from(chunk.artifact_id.as_str()));
    params.insert("idx".into(), DataValue::from(chunk.chunk_index as i64));
    params.insert("ps".into(), DataValue::from(chunk.page_start as i64));
    params.insert("pe".into(), DataValue::from(chunk.page_end as i64));
    params.insert("content".into(), DataValue::from(chunk.content.as_str()));
    params.insert("hash".into(), DataValue::from(chunk.content_hash.as_str()));
    params.insert(
        "estatus".into(),
        DataValue::from(chunk.embedding_status.as_str()),
    );

    db.run_script(
        "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] \
         <- [[$cid, $did, $aid, $idx, $ps, $pe, $content, $hash, $estatus]] \
         :put doc_chunks { chunk_id => document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert doc_chunks failed: {e}"))?;
    Ok(())
}

pub fn list_chunks_for_doc(db: &DbInstance, document_id: &str) -> Result<Vec<ChunkArtifact>> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));

    let result = db
        .run_script(
            "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] \
             := *doc_chunks{chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status}, \
             document_id = $did",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list chunks failed: {e}"))?;

    Ok(result
        .rows
        .iter()
        .map(|row| ChunkArtifact {
            chunk_id: row[0].get_str().unwrap_or("").to_string(),
            document_id: row[1].get_str().unwrap_or("").to_string(),
            artifact_id: row[2].get_str().unwrap_or("").to_string(),
            chunk_index: row[3].get_int().unwrap_or(0) as u32,
            page_start: row[4].get_int().unwrap_or(0) as u32,
            page_end: row[5].get_int().unwrap_or(0) as u32,
            content: row[6].get_str().unwrap_or("").to_string(),
            content_hash: row[7].get_str().unwrap_or("").to_string(),
            embedding_status: row[8].get_str().unwrap_or("pending").to_string(),
        })
        .collect())
}

pub fn chunk_hash_exists(db: &DbInstance, content_hash: &str) -> Result<bool> {
    let mut params = BTreeMap::new();
    params.insert("ch".into(), DataValue::from(content_hash));
    let result = db
        .run_script(
            "?[chunk_id] := *doc_chunks{chunk_id, content_hash}, content_hash = $ch",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("chunk hash check failed: {e}"))?;
    Ok(!result.rows.is_empty())
}

// ---------------------------------------------------------------------------
// ArtifactRecord
// ---------------------------------------------------------------------------

use crate::models::ArtifactRecord;

pub fn insert_artifact(db: &DbInstance, art: &ArtifactRecord) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("aid".into(), DataValue::from(art.artifact_id.as_str()));
    params.insert("did".into(), DataValue::from(art.document_id.as_str()));
    params.insert("atype".into(), DataValue::from(art.artifact_type.as_str()));
    params.insert("hash".into(), DataValue::from(art.content_hash.as_str()));
    params.insert("cat".into(), DataValue::from(art.created_at.as_str()));
    params.insert("prov".into(), DataValue::from(art.provenance_record_id.as_str()));

    db.run_script(
        "?[artifact_id, document_id, artifact_type, content_hash, created_at, provenance_record_id] \
         <- [[$aid, $did, $atype, $hash, $cat, $prov]] \
         :put doc_artifacts { artifact_id => document_id, artifact_type, content_hash, created_at, provenance_record_id }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert doc_artifacts failed: {e}"))?;
    Ok(())
}

pub fn list_artifacts_for_doc(db: &DbInstance, document_id: &str) -> Result<Vec<ArtifactRecord>> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));
    let result = db
        .run_script(
            "?[artifact_id, document_id, artifact_type, content_hash, created_at, provenance_record_id] \
             := *doc_artifacts{artifact_id, document_id, artifact_type, content_hash, created_at, provenance_record_id}, \
             document_id = $did",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list artifacts failed: {e}"))?;
    Ok(result
        .rows
        .iter()
        .map(|row| ArtifactRecord {
            artifact_id: row[0].get_str().unwrap_or("").to_string(),
            document_id: row[1].get_str().unwrap_or("").to_string(),
            artifact_type: row[2].get_str().unwrap_or("").to_string(),
            content_hash: row[3].get_str().unwrap_or("").to_string(),
            created_at: row[4].get_str().unwrap_or("").to_string(),
            provenance_record_id: row[5].get_str().unwrap_or("").to_string(),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// ProvenanceEdge
// ---------------------------------------------------------------------------

pub fn insert_provenance_edge(db: &DbInstance, edge: &ProvenanceEdge) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(edge.edge_id.as_str()));
    params.insert("from".into(), DataValue::from(edge.from_artifact_id.as_str()));
    params.insert("to".into(), DataValue::from(edge.to_artifact_id.as_str()));
    params.insert("etype".into(), DataValue::from(edge_type_str(&edge.edge_type)));
    params.insert("cat".into(), DataValue::from(edge.created_at.as_str()));

    db.run_script(
        "?[edge_id, from_artifact_id, to_artifact_id, edge_type, created_at] \
         <- [[$eid, $from, $to, $etype, $cat]] \
         :put doc_provenance_edges { edge_id => from_artifact_id, to_artifact_id, edge_type, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert provenance edge failed: {e}"))?;
    Ok(())
}

pub fn list_provenance_from(
    db: &DbInstance,
    from_artifact_id: &str,
) -> Result<Vec<ProvenanceEdge>> {
    let mut params = BTreeMap::new();
    params.insert("faid".into(), DataValue::from(from_artifact_id));

    let result = db
        .run_script(
            "?[edge_id, from_artifact_id, to_artifact_id, edge_type, created_at] \
             := *doc_provenance_edges{edge_id, from_artifact_id, to_artifact_id, edge_type, created_at}, \
             from_artifact_id = $faid",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list provenance edges failed: {e}"))?;

    Ok(result
        .rows
        .iter()
        .map(|row| ProvenanceEdge {
            edge_id: row[0].get_str().unwrap_or("").to_string(),
            from_artifact_id: row[1].get_str().unwrap_or("").to_string(),
            to_artifact_id: row[2].get_str().unwrap_or("").to_string(),
            edge_type: parse_edge_type(row[3].get_str().unwrap_or("")),
            created_at: row[4].get_str().unwrap_or("").to_string(),
        })
        .collect())
}

pub fn list_provenance_to(
    db: &DbInstance,
    to_artifact_id: &str,
) -> Result<Vec<ProvenanceEdge>> {
    let mut params = BTreeMap::new();
    params.insert("taid".into(), DataValue::from(to_artifact_id));

    let result = db
        .run_script(
            "?[edge_id, from_artifact_id, to_artifact_id, edge_type, created_at] \
             := *doc_provenance_edges{edge_id, from_artifact_id, to_artifact_id, edge_type, created_at}, \
             to_artifact_id = $taid",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list provenance to failed: {e}"))?;

    Ok(result
        .rows
        .iter()
        .map(|row| ProvenanceEdge {
            edge_id: row[0].get_str().unwrap_or("").to_string(),
            from_artifact_id: row[1].get_str().unwrap_or("").to_string(),
            to_artifact_id: row[2].get_str().unwrap_or("").to_string(),
            edge_type: parse_edge_type(row[3].get_str().unwrap_or("")),
            created_at: row[4].get_str().unwrap_or("").to_string(),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// ProcessingJob
// ---------------------------------------------------------------------------

pub fn insert_processing_job(db: &DbInstance, job: &ProcessingJob) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("jid".into(), DataValue::from(job.job_id.as_str()));
    params.insert("did".into(), DataValue::from(job.document_id.as_str()));
    params.insert("jtype".into(), DataValue::from(job.job_type.as_str()));
    params.insert("status".into(), DataValue::from(job.status.as_str()));
    params.insert("sat".into(), DataValue::from(job.started_at.as_str()));
    params.insert(
        "cat".into(),
        DataValue::from(job.completed_at.as_deref().unwrap_or("")),
    );
    params.insert(
        "err".into(),
        DataValue::from(job.error_message.as_deref().unwrap_or("")),
    );

    db.run_script(
        "?[job_id, document_id, job_type, status, started_at, completed_at, error_message] \
         <- [[$jid, $did, $jtype, $status, $sat, $cat, $err]] \
         :put doc_processing_jobs { job_id => document_id, job_type, status, started_at, completed_at, error_message }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert processing job failed: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn status_str(s: &DocumentStatus) -> &'static str {
    match s {
        DocumentStatus::Discovered => "discovered",
        DocumentStatus::Ingesting => "ingesting",
        DocumentStatus::Ingested => "ingested",
        DocumentStatus::Processing => "processing",
        DocumentStatus::Processed => "processed",
        DocumentStatus::Failed => "failed",
    }
}

fn parse_status(s: &str) -> DocumentStatus {
    match s {
        "discovered" => DocumentStatus::Discovered,
        "ingesting" => DocumentStatus::Ingesting,
        "ingested" => DocumentStatus::Ingested,
        "processing" => DocumentStatus::Processing,
        "processed" => DocumentStatus::Processed,
        "failed" => DocumentStatus::Failed,
        _ => DocumentStatus::Discovered,
    }
}

fn ocr_status_str(s: &OcrStatus) -> &'static str {
    match s {
        OcrStatus::Pending => "pending",
        OcrStatus::Running => "running",
        OcrStatus::Completed => "completed",
        OcrStatus::Failed => "failed",
    }
}

fn parse_ocr_status(s: &str) -> OcrStatus {
    match s {
        "pending" => OcrStatus::Pending,
        "running" => OcrStatus::Running,
        "completed" => OcrStatus::Completed,
        "failed" => OcrStatus::Failed,
        _ => OcrStatus::Pending,
    }
}

fn edge_type_str(t: &crate::models::ProvenanceEdgeType) -> &'static str {
    match t {
        crate::models::ProvenanceEdgeType::DerivedFrom => "DerivedFrom",
        crate::models::ProvenanceEdgeType::Contains => "Contains",
        crate::models::ProvenanceEdgeType::ExtractedFrom => "ExtractedFrom",
        crate::models::ProvenanceEdgeType::Describes => "Describes",
        crate::models::ProvenanceEdgeType::Cites => "Cites",
    }
}

fn parse_edge_type(s: &str) -> crate::models::ProvenanceEdgeType {
    match s {
        "DerivedFrom" => crate::models::ProvenanceEdgeType::DerivedFrom,
        "Contains" => crate::models::ProvenanceEdgeType::Contains,
        "ExtractedFrom" => crate::models::ProvenanceEdgeType::ExtractedFrom,
        "Describes" => crate::models::ProvenanceEdgeType::Describes,
        "Cites" => crate::models::ProvenanceEdgeType::Cites,
        _ => crate::models::ProvenanceEdgeType::DerivedFrom,
    }
}

fn optional_str(s: &str) -> Option<String> {
    if s.is_empty() { None } else { Some(s.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-store-{}.db", uuid::Uuid::new_v4());
        DbInstance::new("sqlite", &path, "").unwrap()
    }

    fn test_doc(id: &str) -> SourceDocument {
        SourceDocument {
            document_id: id.to_string(),
            source_path: "/tmp/test.txt".to_string(),
            media_type: "text/plain".to_string(),
            content_hash: "abc123".to_string(),
            discovered_at: "2026-01-01T00:00:00Z".to_string(),
            status: DocumentStatus::Discovered,
        }
    }

    #[test]
    fn test_insert_and_readback_doc_source() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        let doc = test_doc("insert-readback");
        insert_doc_source(&db, &doc).unwrap();
        let got = get_doc_source(&db, "insert-readback").unwrap().unwrap();
        assert_eq!(got.document_id, doc.document_id);
        assert_eq!(got.status, DocumentStatus::Discovered);
    }

    #[test]
    fn test_update_status() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        let doc = test_doc("update-status");
        insert_doc_source(&db, &doc).unwrap();

        update_doc_status(&db, "update-status", &DocumentStatus::Ingested).unwrap();

        let got = get_doc_source(&db, "update-status").unwrap().unwrap();
        assert_eq!(got.status, DocumentStatus::Ingested, ":update must change status");
    }

    #[test]
    fn test_insert_and_readback_chunk() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        let chunk = ChunkArtifact {
            chunk_id: "chunk-test-0".into(),
            document_id: "test-doc".into(),
            artifact_id: "art-1".into(),
            chunk_index: 0,
            page_start: 1,
            page_end: 1,
            content: "hello".into(),
            content_hash: "hash123".into(),
            embedding_status: "pending".into(),
        };
        insert_chunk(&db, &chunk).unwrap();
        let chunks = list_chunks_for_doc(&db, "test-doc").unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_id, "chunk-test-0");
    }

    #[test]
    fn test_insert_and_readback_page() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        let page = PageArtifact {
            page_id: "page-test-1".into(),
            document_id: "test-doc".into(),
            page_number: 1,
            text_hash: Some("txthash".into()),
            image_hash: None,
            width: None,
            height: None,
            provenance_record_id: String::new(),
        };
        insert_page(&db, &page).unwrap();
        let pages = list_pages_for_doc(&db, "test-doc").unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].page_number, 1);
    }

    #[test]
    fn test_insert_and_readback_ocr_run() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        let run = OcrRun {
            ocr_run_id: "ocr-test-1".into(),
            document_id: "test-doc".into(),
            provider: "local".into(),
            mode: "text/plain".into(),
            status: OcrStatus::Completed,
            started_at: "2026-01-01T00:00:00Z".into(),
            completed_at: Some("2026-01-01T00:00:01Z".into()),
            duration_ms: Some(100),
        };
        insert_ocr_run(&db, &run).unwrap();
        let runs = list_ocr_runs_for_doc(&db, "test-doc").unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, OcrStatus::Completed);
    }

    #[test]
    fn test_insert_and_readback_provenance_edge() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        let edge = ProvenanceEdge {
            edge_id: "edge-test-1".into(),
            from_artifact_id: "chunk-1".into(),
            to_artifact_id: "page-1".into(),
            edge_type: crate::models::ProvenanceEdgeType::ExtractedFrom,
            created_at: "2026-01-01T00:00:00Z".into(),
        };
        insert_provenance_edge(&db, &edge).unwrap();
        let edges = list_provenance_from(&db, "chunk-1").unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to_artifact_id, "page-1");
    }

    #[test]
    fn test_hash_exists_and_get_by_hash() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        let doc = test_doc("hash-doc");
        insert_doc_source(&db, &doc).unwrap();
        assert!(hash_exists_in_sources(&db, "abc123").unwrap());
        assert!(!hash_exists_in_sources(&db, "nonexistent").unwrap());

        let found = get_doc_by_hash(&db, "abc123").unwrap().unwrap();
        assert_eq!(found.document_id, "hash-doc");
    }

    #[test]
    fn test_ocr_run_single_row_after_completion() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();

        // Insert initial Running run
        let run = OcrRun {
            ocr_run_id: "ocr-single".into(),
            document_id: "test-doc".into(),
            provider: "local".into(),
            mode: "text/plain".into(),
            status: OcrStatus::Running,
            started_at: "2026-01-01T00:00:00Z".into(),
            completed_at: None,
            duration_ms: None,
        };
        insert_ocr_run(&db, &run).unwrap();
        assert_eq!(list_ocr_runs_for_doc(&db, "test-doc").unwrap().len(), 1);

        // Update to Completed — must not create a second row
        update_ocr_run_completion(
            &db,
            "ocr-single",
            &OcrStatus::Completed,
            "2026-01-01T00:00:05Z",
            500,
        )
        .unwrap();

        let runs = list_ocr_runs_for_doc(&db, "test-doc").unwrap();
        assert_eq!(runs.len(), 1, "update must not create a second row");
        assert_eq!(runs[0].status, OcrStatus::Completed);
        assert_eq!(runs[0].duration_ms, Some(500));
        assert_eq!(
            runs[0].completed_at.as_deref(),
            Some("2026-01-01T00:00:05Z")
        );
    }
}
