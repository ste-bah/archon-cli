use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::models::{ChunkArtifact, ChunkHashes, ChunkSpatial};

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

/// Insert (upsert) a chunk's spatial-provenance row into `doc_chunk_spatial`.
pub fn insert_chunk_spatial(db: &DbInstance, s: &ChunkSpatial) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("cid".into(), DataValue::from(s.chunk_id.as_str()));
    params.insert("pn".into(), DataValue::from(s.page_num as i64));
    params.insert("sb".into(), DataValue::from(s.super_box.as_str()));
    params.insert("bl".into(), DataValue::from(s.blocks.as_str()));
    params.insert("cs".into(), DataValue::from(s.coord_space.as_str()));
    params.insert("sh".into(), DataValue::from(s.spatial_hash.as_str()));
    crate::cozo_retry::run_script_guarded(
        db,
        "?[chunk_id, page_num, super_box, blocks, coord_space, spatial_hash] \
         <- [[$cid, $pn, $sb, $bl, $cs, $sh]] \
         :put doc_chunk_spatial { chunk_id => page_num, super_box, blocks, coord_space, spatial_hash }",
        params,
        ScriptMutability::Mutable,
        "insert doc_chunk_spatial",
    )
    .map_err(|e| anyhow::anyhow!("insert doc_chunk_spatial failed: {e}"))?;
    Ok(())
}

/// Read a single chunk's spatial row, if present.
pub fn get_chunk_spatial(db: &DbInstance, chunk_id: &str) -> Result<Option<ChunkSpatial>> {
    let mut params = BTreeMap::new();
    params.insert("cid".into(), DataValue::from(chunk_id));
    let result = crate::cozo_retry::run_script_guarded(
        db,
        "?[chunk_id, page_num, super_box, blocks, coord_space, spatial_hash] \
         := *doc_chunk_spatial{chunk_id, page_num, super_box, blocks, coord_space, spatial_hash}, \
         chunk_id = $cid",
        params,
        ScriptMutability::Immutable,
        "get doc_chunk_spatial",
    )
    .map_err(|e| anyhow::anyhow!("get doc_chunk_spatial failed: {e}"))?;
    if result.rows.is_empty() {
        return Ok(None);
    }
    let row = &result.rows[0];
    Ok(Some(ChunkSpatial {
        chunk_id: row[0].get_str().unwrap_or("").to_string(),
        page_num: row[1].get_int().unwrap_or(0) as u32,
        super_box: row[2].get_str().unwrap_or("").to_string(),
        blocks: row[3].get_str().unwrap_or("").to_string(),
        coord_space: row[4].get_str().unwrap_or("").to_string(),
        spatial_hash: row[5].get_str().unwrap_or("").to_string(),
    }))
}

/// Insert (upsert) a chunk's integrity-hash row into `doc_chunk_hashes`.
pub fn insert_chunk_hashes(db: &DbInstance, h: &ChunkHashes) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("cid".into(), DataValue::from(h.chunk_id.as_str()));
    params.insert("raw".into(), DataValue::from(h.raw_sha256.as_str()));
    params.insert("cv".into(), DataValue::from(h.cleaning_version.as_str()));
    params.insert("commit".into(), DataValue::from(h.commit_hash.as_str()));
    crate::cozo_retry::run_script_guarded(
        db,
        "?[chunk_id, raw_sha256, cleaning_version, commit_hash] \
         <- [[$cid, $raw, $cv, $commit]] \
         :put doc_chunk_hashes { chunk_id => raw_sha256, cleaning_version, commit_hash }",
        params,
        ScriptMutability::Mutable,
        "insert doc_chunk_hashes",
    )
    .map_err(|e| anyhow::anyhow!("insert doc_chunk_hashes failed: {e}"))?;
    Ok(())
}

/// Read a single chunk's integrity-hash row, if present.
pub fn get_chunk_hashes(db: &DbInstance, chunk_id: &str) -> Result<Option<ChunkHashes>> {
    let mut params = BTreeMap::new();
    params.insert("cid".into(), DataValue::from(chunk_id));
    let result = crate::cozo_retry::run_script_guarded(
        db,
        "?[chunk_id, raw_sha256, cleaning_version, commit_hash] \
         := *doc_chunk_hashes{chunk_id, raw_sha256, cleaning_version, commit_hash}, chunk_id = $cid",
        params,
        ScriptMutability::Immutable,
        "get doc_chunk_hashes",
    )
    .map_err(|e| anyhow::anyhow!("get doc_chunk_hashes failed: {e}"))?;
    if result.rows.is_empty() {
        return Ok(None);
    }
    let row = &result.rows[0];
    Ok(Some(ChunkHashes {
        chunk_id: row[0].get_str().unwrap_or("").to_string(),
        raw_sha256: row[1].get_str().unwrap_or("").to_string(),
        cleaning_version: row[2].get_str().unwrap_or("").to_string(),
        commit_hash: row[3].get_str().unwrap_or("").to_string(),
    }))
}

/// All `commit_hash`es for a document's chunks (join `doc_chunks` × `doc_chunk_hashes`
/// on `chunk_id`). Order is unspecified — the caller sorts before computing `chunks_root`.
pub fn get_doc_commit_hashes(db: &DbInstance, document_id: &str) -> Result<Vec<String>> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));
    let result = crate::cozo_retry::run_script_guarded(
        db,
        "?[commit_hash] := *doc_chunks{chunk_id, document_id}, document_id = $did, \
         *doc_chunk_hashes{chunk_id, commit_hash}",
        params,
        ScriptMutability::Immutable,
        "get doc commit hashes",
    )
    .map_err(|e| anyhow::anyhow!("get doc commit hashes failed: {e}"))?;
    Ok(result
        .rows
        .iter()
        .map(|row| row[0].get_str().unwrap_or("").to_string())
        .collect())
}
