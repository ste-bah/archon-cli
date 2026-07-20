use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::models::ChunkArtifact;

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

    crate::cozo_retry::run_script_guarded(
        db,
        "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] \
         <- [[$cid, $did, $aid, $idx, $ps, $pe, $content, $hash, $estatus]] \
         :put doc_chunks { chunk_id => document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status }",
        params,
        ScriptMutability::Mutable,
        "insert doc_chunks",
    )
    .map_err(|e| anyhow::anyhow!("insert doc_chunks failed: {e}"))?;
    crate::index_queue::enqueue_pending_chunk(db, chunk, 0)?;
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

/// Look up a single chunk by its chunk_id across all documents.
pub fn get_chunk_by_id(db: &DbInstance, chunk_id: &str) -> Result<Option<ChunkArtifact>> {
    let mut params = BTreeMap::new();
    params.insert("cid".into(), DataValue::from(chunk_id));

    let result = crate::cozo_retry::run_script_guarded(
            db,
            "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] \
             := *doc_chunks{chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status}, \
             chunk_id = $cid",
            params,
            ScriptMutability::Immutable,
            "get chunk by id",
        )
        .map_err(|e| anyhow::anyhow!("get chunk by id failed: {e}"))?;

    if result.rows.is_empty() {
        return Ok(None);
    }
    let row = &result.rows[0];
    Ok(Some(ChunkArtifact {
        chunk_id: row[0].get_str().unwrap_or("").to_string(),
        document_id: row[1].get_str().unwrap_or("").to_string(),
        artifact_id: row[2].get_str().unwrap_or("").to_string(),
        chunk_index: row[3].get_int().unwrap_or(0) as u32,
        page_start: row[4].get_int().unwrap_or(0) as u32,
        page_end: row[5].get_int().unwrap_or(0) as u32,
        content: row[6].get_str().unwrap_or("").to_string(),
        content_hash: row[7].get_str().unwrap_or("").to_string(),
        embedding_status: row[8].get_str().unwrap_or("pending").to_string(),
    }))
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
