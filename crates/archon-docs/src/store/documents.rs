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
    .map_err(|e| e.context("insert doc_sources failed"))?;
    Ok(())
}

pub fn get_doc_source(db: &DbInstance, document_id: &str) -> Result<Option<SourceDocument>> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));

    let result = crate::cozo_retry::run_script_guarded(
        db,
        "?[document_id, source_path, media_type, content_hash, discovered_at, status] \
         := *doc_sources{document_id, source_path, media_type, content_hash, discovered_at, status}, \
         document_id = $did",
        params,
        ScriptMutability::Immutable,
        "get doc source",
    )
    .map_err(|e| e.context("get doc_sources failed"))?;

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
    let result = crate::cozo_retry::run_script_guarded(
        db,
        "?[document_id, source_path, media_type, content_hash, discovered_at, status] \
         := *doc_sources{document_id, source_path, media_type, content_hash, discovered_at, status}",
        Default::default(),
        ScriptMutability::Immutable,
        "list doc sources",
    )
    .map_err(|e| e.context("list doc_sources failed"))?;

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

/// Outcome of [`reserve_doc_source_by_hash`].
#[derive(Debug)]
pub enum HashReservation<'a> {
    /// Nothing owned the hash, so the caller's document was registered and
    /// the caller owns its ingest.
    Registered(super::IngestClaim<'a>),
    /// An earlier ingest of this hash registered the document and stopped
    /// before its final status (a pause or a crash). Nothing owns it, so the
    /// caller now owns it and must finish it in place.
    Resume(Box<SourceDocument>, super::IngestClaim<'a>),
    /// Another document already owned the hash, finished or still in a live
    /// ingest; nothing was written.
    Duplicate(Box<SourceDocument>),
}

/// Claim `doc`'s content hash, registering `doc` only if no document owns it yet.
///
/// `doc_sources` is keyed on `document_id`, and every ingest mints a fresh UUID,
/// so the relation itself enforces nothing about `content_hash`. Dedup is
/// therefore a read followed by a write, and it only holds if nothing can write
/// between the two. Callers must not open-code that pair — this is the one place
/// the window exists, so it is the one place that closes it.
///
/// The ingest claim in `slot` is taken in the same window: a registration
/// whose claim is free belongs to an interrupted ingest and is resumed, and
/// one whose claim is held belongs to a live ingest and is a duplicate.
pub fn reserve_doc_source_by_hash<'a>(
    db: &DbInstance,
    doc: &SourceDocument,
    slot: &'a mut super::ClaimSlot,
) -> Result<HashReservation<'a>> {
    with_reservation_lock_held(db, move || match get_doc_by_hash(db, &doc.content_hash)? {
        Some(existing) if !super::ingest_claim::is_resumable(&existing.status) => {
            Ok(HashReservation::Duplicate(Box::new(existing)))
        }
        Some(existing) => Ok(match slot.try_claim()? {
            Some(claim) => HashReservation::Resume(Box::new(existing), claim),
            None => HashReservation::Duplicate(Box::new(existing)),
        }),
        None => {
            let claim = slot.try_claim()?.ok_or_else(|| {
                anyhow::anyhow!(
                    "content hash {} is claimed by a live ingest that has no registration",
                    doc.content_hash
                )
            })?;
            #[cfg(test)]
            super::hash_reservation_test_hooks::wait_before_reservation(&doc.content_hash);
            insert_doc_source(db, doc)?;
            Ok(HashReservation::Registered(claim))
        }
    })
}

const RESERVATION_CONTEXT: &str = "reserve document content hash";

/// Run `reserve` with exclusive access to the backing database.
///
/// The blocking write lock keeps the read and write in one exclusive window.
/// A wait continues while the holder progresses and ends as a typed pause
/// after one no-progress window. It is re-entrant, so a reservation nested
/// inside an already-guarded mutable operation on the same database runs inline
/// rather than blocking on the lock its own thread holds.
///
/// An in-memory store has no lock path and needs none — nothing outside the
/// process can reach it. A file-backed store that was opened without a guard
/// config has no path either, but that is a mistake rather than a case to
/// tolerate, and `bound_guard_config` already rejects it here for the same
/// reason every `:put` in this crate does.
pub(super) fn with_reservation_lock_held<T>(
    db: &DbInstance,
    reserve: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let config = archon_cozo::bound_guard_config(db, RESERVATION_CONTEXT)?;
    match config.write_lock_path.as_deref() {
        Some(path) => archon_cozo::with_write_lock_resuming(
            path,
            RESERVATION_CONTEXT,
            config
                .write_lock_wait
                .unwrap_or(archon_cozo::DEFAULT_WRITE_LOCK_WAIT),
            reserve,
        ),
        None => reserve(),
    }
}

/// Look up an existing document by content hash (for duplicate reporting).
pub fn get_doc_by_hash(db: &DbInstance, content_hash: &str) -> Result<Option<SourceDocument>> {
    let mut params = BTreeMap::new();
    params.insert("ch".into(), DataValue::from(content_hash));
    let result = crate::cozo_retry::run_script_guarded(
        db,
        "?[document_id, source_path, media_type, content_hash, discovered_at, status] \
         := *doc_sources{document_id, source_path, media_type, content_hash, discovered_at, status}, \
         content_hash = $ch",
        params,
        ScriptMutability::Immutable,
        "get doc by hash",
    )
    .map_err(|e| e.context("get doc by hash failed"))?;
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
    .map_err(|e| e.context("assign document to kb failed"))?;
    Ok(())
}

pub fn list_kb_document_ids(db: &DbInstance, kb_id: &str) -> Result<Vec<String>> {
    let mut params = BTreeMap::new();
    params.insert("kid".into(), DataValue::from(kb_id));
    let result = crate::cozo_retry::run_script_guarded(
        db,
        "?[document_id] := *doc_kb_memberships{kb_id, document_id}, kb_id = $kid",
        params,
        ScriptMutability::Immutable,
        "list kb document ids",
    )
    .map_err(|e| e.context("list kb documents failed"))?;
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
    .map_err(|e| e.context("insert doc_ocr_runs failed"))?;
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
    let result = crate::cozo_retry::run_script_guarded(
        db,
        "?[ocr_run_id, document_id, provider, mode, status, started_at, completed_at, duration_ms] \
         := *doc_ocr_runs{ocr_run_id, document_id, provider, mode, status, started_at, completed_at, duration_ms}, \
         ocr_run_id = $oid",
        params,
        ScriptMutability::Immutable,
        "list ocr runs for ocr id",
    )
    .map_err(|e| e.context("list ocr_runs by id failed"))?;
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

    let result = crate::cozo_retry::run_script_guarded(
        db,
        "?[ocr_run_id, document_id, provider, mode, status, started_at, completed_at, duration_ms] \
         := *doc_ocr_runs{ocr_run_id, document_id, provider, mode, status, started_at, completed_at, duration_ms}, \
         document_id = $did",
        params,
        ScriptMutability::Immutable,
        "list ocr runs for doc",
    )
    .map_err(|e| e.context("list ocr_runs failed"))?;

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
