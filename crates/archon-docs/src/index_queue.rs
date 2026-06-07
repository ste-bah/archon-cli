use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::models::ChunkArtifact;

const STATUS_PENDING: &str = "pending";
const STATUS_LEASED: &str = "leased";
const STATUS_INDEXED: &str = "indexed";
const STATUS_FAILED: &str = "failed";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexQueueStats {
    pub pending: usize,
    pub leased: usize,
    pub indexed: usize,
    pub failed: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexQueueFailure {
    pub chunk_id: String,
    pub document_id: String,
    pub attempt_count: i64,
    pub last_error: String,
    pub updated_at: String,
}

#[derive(Clone, Debug)]
struct QueueRow {
    chunk: ChunkArtifact,
    priority: i64,
    status: String,
    attempt_count: i64,
    lease_owner: String,
    lease_expires_at: String,
    last_error: String,
    created_at: String,
    updated_at: String,
}

pub fn enqueue_pending_chunk(db: &DbInstance, chunk: &ChunkArtifact, priority: i64) -> Result<()> {
    if chunk.embedding_status != STATUS_PENDING {
        return Ok(());
    }
    put_rows(db, &[new_row(chunk.clone(), priority, STATUS_PENDING)])?;
    Ok(())
}

pub fn backfill_pending_chunks(
    db: &DbInstance,
    document_id: Option<&str>,
    limit: Option<usize>,
) -> Result<usize> {
    let chunks = list_pending_doc_chunks(db, document_id, limit)?;
    let rows = chunks
        .into_iter()
        .map(|chunk| new_row(chunk, 0, STATUS_PENDING))
        .collect::<Vec<_>>();
    put_rows(db, &rows)?;
    Ok(rows.len())
}

pub fn count_pending(db: &DbInstance, document_id: Option<&str>) -> Result<usize> {
    prune_orphaned_queue_rows(db)?;
    count_status(db, STATUS_PENDING, document_id)
}

pub fn stats(db: &DbInstance) -> Result<IndexQueueStats> {
    prune_orphaned_queue_rows(db)?;
    Ok(IndexQueueStats {
        pending: count_status(db, STATUS_PENDING, None)?,
        leased: count_status(db, STATUS_LEASED, None)?,
        indexed: count_status(db, STATUS_INDEXED, None)?,
        failed: count_status(db, STATUS_FAILED, None)?,
    })
}

pub fn lease_pending_chunks(
    db: &DbInstance,
    owner: &str,
    limit: usize,
    lease_secs: u64,
    document_id: Option<&str>,
) -> Result<Vec<ChunkArtifact>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    reset_expired_leases(db)?;
    prune_orphaned_queue_rows(db)?;
    let rows = list_pending_queue_rows(db, document_id, Some(limit))?;
    let expires_at =
        (chrono::Utc::now() + chrono::Duration::seconds(lease_secs as i64)).to_rfc3339();
    let leased = rows
        .into_iter()
        .map(|mut row| {
            row.status = STATUS_LEASED.into();
            row.attempt_count += 1;
            row.lease_owner = owner.into();
            row.lease_expires_at = expires_at.clone();
            row
        })
        .collect::<Vec<_>>();
    put_rows(db, &leased)?;
    Ok(leased.into_iter().map(|row| row.chunk).collect())
}

pub fn mark_chunks_indexed(db: &DbInstance, chunks: &[&ChunkArtifact]) -> Result<()> {
    mark_chunks(db, chunks, STATUS_INDEXED, "")
}

pub fn mark_chunks_failed(db: &DbInstance, chunks: &[&ChunkArtifact], error: &str) -> Result<()> {
    mark_chunks(db, chunks, STATUS_FAILED, error)
}

pub fn retry_failed(db: &DbInstance, limit: Option<usize>) -> Result<usize> {
    let mut rows = list_queue_rows_by_status(db, STATUS_FAILED, limit)?;
    let count = rows.len();
    for row in &mut rows {
        row.status = STATUS_PENDING.into();
        row.lease_owner.clear();
        row.lease_expires_at.clear();
        row.last_error.clear();
    }
    put_rows(db, &rows)?;
    Ok(count)
}

pub fn release_leases_for_owner(db: &DbInstance, owner: &str) -> Result<usize> {
    let mut params = BTreeMap::new();
    params.insert("owner".into(), DataValue::from(owner));
    let mut rows = query_queue_rows(
        db,
        &queue_select("status = \"leased\", lease_owner = $owner"),
        params,
    )?;
    let count = rows.len();
    for row in &mut rows {
        row.status = STATUS_PENDING.into();
        row.lease_owner.clear();
        row.lease_expires_at.clear();
    }
    put_rows(db, &rows)?;
    Ok(count)
}

pub fn failed_rows(db: &DbInstance, limit: usize) -> Result<Vec<IndexQueueFailure>> {
    let rows = list_queue_rows_by_status(db, STATUS_FAILED, Some(limit))?;
    Ok(rows
        .into_iter()
        .map(|row| IndexQueueFailure {
            chunk_id: row.chunk.chunk_id,
            document_id: row.chunk.document_id,
            attempt_count: row.attempt_count,
            last_error: row.last_error,
            updated_at: row.updated_at,
        })
        .collect())
}

pub fn remove_document_queue_rows(db: &DbInstance, document_id: &str) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));
    crate::cozo_retry::run_script_guarded(
        db,
        "?[chunk_id] := *doc_index_queue{chunk_id, document_id}, document_id = $did
         :rm doc_index_queue { chunk_id }",
        params,
        ScriptMutability::Mutable,
        "remove document queue rows",
    )
    .map_err(|e| anyhow::anyhow!("remove document queue rows failed: {e}"))?;
    Ok(())
}

pub fn prune_orphaned_queue_rows(db: &DbInstance) -> Result<usize> {
    let result = crate::cozo_retry::run_script_guarded(
        db,
        "?[chunk_id] := *doc_index_queue{chunk_id}, not *doc_chunks{chunk_id}
         :rm doc_index_queue { chunk_id }",
        BTreeMap::new(),
        ScriptMutability::Mutable,
        "prune orphaned doc index queue rows",
    )
    .map_err(|e| anyhow::anyhow!("prune orphaned doc index queue rows failed: {e}"))?;
    Ok(result.rows.len())
}

fn mark_chunks(
    db: &DbInstance,
    chunks: &[&ChunkArtifact],
    status: &str,
    error: &str,
) -> Result<()> {
    if chunks.is_empty() {
        return Ok(());
    }
    let existing = existing_rows_for_chunks(db, chunks)?;
    let rows = chunks
        .iter()
        .map(|chunk| QueueRow {
            chunk: (*chunk).clone(),
            priority: existing
                .get(chunk.chunk_id.as_str())
                .map(|row| row.priority)
                .unwrap_or(0),
            status: status.into(),
            attempt_count: existing
                .get(chunk.chunk_id.as_str())
                .map(|row| row.attempt_count)
                .unwrap_or(1)
                .max(1),
            lease_owner: String::new(),
            lease_expires_at: String::new(),
            last_error: truncate_error(error),
            created_at: existing
                .get(chunk.chunk_id.as_str())
                .map(|row| row.created_at.clone())
                .unwrap_or_else(now),
            updated_at: String::new(),
        })
        .collect::<Vec<_>>();
    put_rows(db, &rows)
}

fn existing_rows_for_chunks(
    db: &DbInstance,
    chunks: &[&ChunkArtifact],
) -> Result<BTreeMap<String, QueueRow>> {
    let mut rows = BTreeMap::new();
    for chunk in chunks {
        if let Some(row) = existing_row_for_chunk(db, &chunk.chunk_id)? {
            rows.insert(chunk.chunk_id.clone(), row);
        }
    }
    Ok(rows)
}

fn existing_row_for_chunk(db: &DbInstance, chunk_id: &str) -> Result<Option<QueueRow>> {
    let mut params = BTreeMap::new();
    params.insert("cid".into(), DataValue::from(chunk_id));
    let mut rows = query_queue_rows(db, &queue_select("chunk_id = $cid"), params)?;
    Ok(rows.pop())
}

fn list_pending_doc_chunks(
    db: &DbInstance,
    document_id: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<ChunkArtifact>> {
    let mut script = if document_id.is_some() {
        "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] \
         := *doc_chunks{chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status}, \
         document_id = $did, embedding_status = \"pending\""
            .to_string()
    } else {
        "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] \
         := *doc_chunks{chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status}, \
         embedding_status = \"pending\""
            .to_string()
    };
    script.push_str(" :order document_id, chunk_index");
    if let Some(limit) = limit {
        script.push_str(&format!(" :limit {limit}"));
    }
    let result = crate::cozo_retry::run_script_guarded(
        db,
        &script,
        doc_params(document_id),
        ScriptMutability::Immutable,
        "list pending doc chunks",
    )
    .map_err(|e| anyhow::anyhow!("list pending doc chunks failed: {e}"))?;
    Ok(result.rows.iter().map(|row| chunk_from_row(row)).collect())
}

fn list_pending_queue_rows(
    db: &DbInstance,
    document_id: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<QueueRow>> {
    let mut script = if document_id.is_some() {
        queue_select("status = \"pending\", document_id = $did")
    } else {
        queue_select("status = \"pending\"")
    };
    script.push_str(" :order document_id, chunk_index");
    if let Some(limit) = limit {
        script.push_str(&format!(" :limit {limit}"));
    }
    query_queue_rows(db, &script, doc_params(document_id))
}

fn list_queue_rows_by_status(
    db: &DbInstance,
    status: &str,
    limit: Option<usize>,
) -> Result<Vec<QueueRow>> {
    let mut params = BTreeMap::new();
    params.insert("status".into(), DataValue::from(status));
    let mut script = queue_select("status = $status");
    script.push_str(" :order document_id, chunk_index");
    if let Some(limit) = limit {
        script.push_str(&format!(" :limit {limit}"));
    }
    query_queue_rows(db, &script, params)
}

fn reset_expired_leases(db: &DbInstance) -> Result<()> {
    let now = now();
    let mut params = BTreeMap::new();
    params.insert("now".into(), DataValue::from(now.as_str()));
    let mut rows = query_queue_rows(
        db,
        &queue_select("status = \"leased\", lease_expires_at < $now"),
        params,
    )?;
    for row in &mut rows {
        row.status = STATUS_PENDING.into();
        row.lease_owner.clear();
        row.lease_expires_at.clear();
    }
    put_rows(db, &rows)
}

fn queue_select(predicate: &str) -> String {
    format!(
        "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status, priority, status, attempt_count, lease_owner, lease_expires_at, last_error, created_at, updated_at] \
         := *doc_index_queue{{chunk_id, document_id, content_hash, priority, status, attempt_count, lease_owner, lease_expires_at, last_error, created_at, updated_at}}, \
         *doc_chunks{{chunk_id, artifact_id, chunk_index, page_start, page_end, content, embedding_status}}, {predicate}"
    )
}

fn query_queue_rows(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
) -> Result<Vec<QueueRow>> {
    let result = crate::cozo_retry::run_script_guarded(
        db,
        script,
        params,
        ScriptMutability::Immutable,
        "query doc index queue",
    )
    .map_err(|e| anyhow::anyhow!("query doc index queue failed: {e}"))?;
    Ok(result
        .rows
        .iter()
        .map(|row| queue_row_from_row(row))
        .collect())
}

fn count_status(db: &DbInstance, status: &str, document_id: Option<&str>) -> Result<usize> {
    let mut params = doc_params(document_id);
    params.insert("status".into(), DataValue::from(status));
    let script = if document_id.is_some() {
        "?[count(chunk_id)] := *doc_index_queue{chunk_id, document_id, status}, status = $status, document_id = $did"
    } else {
        "?[count(chunk_id)] := *doc_index_queue{chunk_id, status}, status = $status"
    };
    let result = crate::cozo_retry::run_script_guarded(
        db,
        script,
        params,
        ScriptMutability::Immutable,
        "count doc index queue",
    )
    .map_err(|e| anyhow::anyhow!("count doc index queue failed: {e}"))?;
    Ok(result.rows[0][0].get_int().unwrap_or(0) as usize)
}

fn put_rows(db: &DbInstance, rows: &[QueueRow]) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let mut params = BTreeMap::new();
    let updated_at = now();
    let tuples = rows
        .iter()
        .enumerate()
        .map(|(index, row)| queue_tuple(&mut params, index, row, &updated_at))
        .collect::<Vec<_>>();
    let script = format!(
        "?[chunk_id, document_id, content_hash, priority, status, attempt_count, lease_owner, lease_expires_at, last_error, created_at, updated_at] <- [{}]\n\
         :put doc_index_queue {{ chunk_id => document_id, content_hash, priority, status, attempt_count, lease_owner, lease_expires_at, last_error, created_at, updated_at }}",
        tuples.join(", ")
    );
    crate::cozo_retry::run_script_guarded(
        db,
        &script,
        params,
        ScriptMutability::Mutable,
        "write doc index queue",
    )
    .map_err(|e| anyhow::anyhow!("write doc index queue failed: {e}"))?;
    Ok(())
}

fn queue_tuple(
    params: &mut BTreeMap<String, DataValue>,
    index: usize,
    row: &QueueRow,
    updated_at: &str,
) -> String {
    let keys = [
        ("cid", row.chunk.chunk_id.as_str()),
        ("did", row.chunk.document_id.as_str()),
        ("hash", row.chunk.content_hash.as_str()),
        ("status", row.status.as_str()),
        ("owner", row.lease_owner.as_str()),
        ("expires", row.lease_expires_at.as_str()),
        ("err", row.last_error.as_str()),
        ("created", row.created_at.as_str()),
        ("updated", updated_at),
    ];
    for (prefix, value) in keys {
        params.insert(format!("{prefix}{index}"), DataValue::from(value));
    }
    params.insert(format!("priority{index}"), DataValue::from(row.priority));
    params.insert(
        format!("attempts{index}"),
        DataValue::from(row.attempt_count),
    );
    format!(
        "[$cid{index}, $did{index}, $hash{index}, $priority{index}, $status{index}, $attempts{index}, $owner{index}, $expires{index}, $err{index}, $created{index}, $updated{index}]"
    )
}

fn new_row(chunk: ChunkArtifact, priority: i64, status: &str) -> QueueRow {
    QueueRow {
        chunk,
        priority,
        status: status.into(),
        attempt_count: 0,
        lease_owner: String::new(),
        lease_expires_at: String::new(),
        last_error: String::new(),
        created_at: now(),
        updated_at: String::new(),
    }
}

fn doc_params(document_id: Option<&str>) -> BTreeMap<String, DataValue> {
    let mut params = BTreeMap::new();
    if let Some(document_id) = document_id {
        params.insert("did".into(), DataValue::from(document_id));
    }
    params
}

fn queue_row_from_row(row: &[DataValue]) -> QueueRow {
    QueueRow {
        chunk: chunk_from_row(row),
        priority: row[9].get_int().unwrap_or(0),
        status: row[10].get_str().unwrap_or("").to_string(),
        attempt_count: row[11].get_int().unwrap_or(0),
        lease_owner: row[12].get_str().unwrap_or("").to_string(),
        lease_expires_at: row[13].get_str().unwrap_or("").to_string(),
        last_error: row[14].get_str().unwrap_or("").to_string(),
        created_at: row[15].get_str().unwrap_or("").to_string(),
        updated_at: row[16].get_str().unwrap_or("").to_string(),
    }
}

fn chunk_from_row(row: &[DataValue]) -> ChunkArtifact {
    ChunkArtifact {
        chunk_id: row[0].get_str().unwrap_or("").to_string(),
        document_id: row[1].get_str().unwrap_or("").to_string(),
        artifact_id: row[2].get_str().unwrap_or("").to_string(),
        chunk_index: row[3].get_int().unwrap_or(0) as u32,
        page_start: row[4].get_int().unwrap_or(0) as u32,
        page_end: row[5].get_int().unwrap_or(0) as u32,
        content: row[6].get_str().unwrap_or("").to_string(),
        content_hash: row[7].get_str().unwrap_or("").to_string(),
        embedding_status: row[8].get_str().unwrap_or("pending").to_string(),
    }
}

fn truncate_error(error: &str) -> String {
    error.chars().take(500).collect()
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
