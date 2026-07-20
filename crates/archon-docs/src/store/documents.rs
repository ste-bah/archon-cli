use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::models::{DocumentStatus, OcrRun, OcrStatus, SourceDocument};

use super::common::{ocr_status_str, parse_ocr_status, parse_status, status_str};

pub fn insert_doc_source(db: &DbInstance, doc: &SourceDocument) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(doc.document_id.as_str()));
    params.insert("path".into(), DataValue::from(doc.source_path.as_str()));
    params.insert("mtype".into(), DataValue::from(doc.media_type.as_str()));
    params.insert("hash".into(), DataValue::from(doc.content_hash.as_str()));
    params.insert("dat".into(), DataValue::from(doc.discovered_at.as_str()));
    params.insert("status".into(), DataValue::from(status_str(&doc.status)));

    crate::cozo_retry::run_script_guarded(
        db,
        "?[document_id, source_path, media_type, content_hash, discovered_at, status] \
         <- [[$did, $path, $mtype, $hash, $dat, $status]] \
         :put doc_sources { document_id => source_path, media_type, content_hash, discovered_at, status }",
        params,
        ScriptMutability::Mutable,
        "insert doc_sources",
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
// Knowledge-base membership
// ---------------------------------------------------------------------------

pub fn assign_document_to_kb(db: &DbInstance, kb_id: &str, document_id: &str) -> Result<()> {
    if get_doc_source(db, document_id)?.is_none() {
        anyhow::bail!("document not found: {document_id}");
    }
    let mut params = BTreeMap::new();
    params.insert("kid".into(), DataValue::from(kb_id));
    params.insert("did".into(), DataValue::from(document_id));
    let assigned_at = chrono::Utc::now().to_rfc3339();
    params.insert("ts".into(), DataValue::from(assigned_at.as_str()));
    crate::cozo_retry::run_script_guarded(
        db,
        "?[kb_id, document_id, assigned_at] <- [[$kid, $did, $ts]] \
         :put doc_kb_memberships { kb_id, document_id => assigned_at }",
        params,
        ScriptMutability::Mutable,
        "assign document to kb",
    )
    .map_err(|e| anyhow::anyhow!("assign document to kb failed: {e}"))?;
    Ok(())
}

pub fn list_kb_document_ids(db: &DbInstance, kb_id: &str) -> Result<Vec<String>> {
    let mut params = BTreeMap::new();
    params.insert("kid".into(), DataValue::from(kb_id));
    let result = db
        .run_script(
            "?[document_id] := *doc_kb_memberships{kb_id, document_id}, kb_id = $kid",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list kb documents failed: {e}"))?;
    Ok(result
        .rows
        .iter()
        .filter_map(|row| row.first()?.get_str().map(ToString::to_string))
        .collect())
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
    params.insert(
        "status".into(),
        DataValue::from(ocr_status_str(&run.status)),
    );
    params.insert("sat".into(), DataValue::from(run.started_at.as_str()));
    params.insert(
        "cat".into(),
        DataValue::from(run.completed_at.as_deref().unwrap_or("")),
    );
    params.insert(
        "dur".into(),
        DataValue::from(run.duration_ms.unwrap_or(0) as i64),
    );

    crate::cozo_retry::run_script_guarded(
        db,
        "?[ocr_run_id, document_id, provider, mode, status, started_at, completed_at, duration_ms] \
         <- [[$oid, $did, $prov, $mode, $status, $sat, $cat, $dur]] \
         :put doc_ocr_runs { ocr_run_id => document_id, provider, mode, status, started_at, completed_at, duration_ms }",
        params,
        ScriptMutability::Mutable,
        "insert doc_ocr_runs",
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
    run.duration_ms = if duration_ms == 0 {
        None
    } else {
        Some(duration_ms)
    };
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
                if s.is_empty() {
                    None
                } else {
                    Some(s.to_string())
                }
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
                if s.is_empty() {
                    None
                } else {
                    Some(s.to_string())
                }
            },
            duration_ms: {
                let d = row[7].get_int().unwrap_or(0);
                if d == 0 { None } else { Some(d as u64) }
            },
        })
        .collect())
}
