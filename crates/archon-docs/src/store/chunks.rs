use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::models::{ChunkArtifact, ChunkBlock, ChunkHashes, ChunkSpatial, PageBreak};

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

/// Tier-1b completion audit: chunks belonging to INGESTED documents whose
/// embedding_status is not "indexed". After a full (non-scoped) index pass this
/// must be zero — a non-zero count means retrieval silently misses content.
pub fn count_unindexed_ingested_chunks(db: &DbInstance) -> Result<usize> {
    let result = db
        .run_script(
            "?[chunk_id] := *doc_chunks{chunk_id, document_id, embedding_status}, \
             *doc_sources{document_id, status}, \
             status = 'ingested', embedding_status != 'indexed'",
            BTreeMap::new(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("unindexed-chunk audit failed: {e}"))?;
    Ok(result.rows.len())
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

/// All `doc_chunk_blocks` rows for a document (join through `doc_chunks`), grouped by
/// `chunk_id`. Reads the full block table in one query — avoids N per-chunk round trips.
pub fn list_chunk_blocks_for_doc(
    db: &DbInstance,
    document_id: &str,
) -> Result<BTreeMap<String, Vec<ChunkBlock>>> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));
    let result = crate::cozo_retry::run_script_guarded(
        db,
        "?[chunk_id, block_idx, char_start, char_end, page, x0, y0, x1, y1, block_type, text_hash] := \
         *doc_chunks{chunk_id, document_id}, document_id = $did, \
         *doc_chunk_blocks{chunk_id, block_idx, char_start, char_end, page, x0, y0, x1, y1, block_type, text_hash}",
        params,
        ScriptMutability::Immutable,
        "list doc_chunk_blocks (doc)",
    )
    .map_err(|e| anyhow::anyhow!("list doc_chunk_blocks (doc) failed: {e}"))?;
    let mut by_chunk: BTreeMap<String, Vec<ChunkBlock>> = BTreeMap::new();
    for row in &result.rows {
        let block = ChunkBlock {
            chunk_id: row[0].get_str().unwrap_or("").to_string(),
            block_idx: row[1].get_int().unwrap_or(0) as u32,
            char_start: row[2].get_int().unwrap_or(0).max(0) as usize,
            char_end: row[3].get_int().unwrap_or(0).max(0) as usize,
            page: row[4].get_int().unwrap_or(0) as u32,
            x0: row[5].get_float().unwrap_or(0.0) as f32,
            y0: row[6].get_float().unwrap_or(0.0) as f32,
            x1: row[7].get_float().unwrap_or(0.0) as f32,
            y1: row[8].get_float().unwrap_or(0.0) as f32,
            block_type: row[9].get_str().unwrap_or("").to_string(),
            text_hash: row[10].get_str().unwrap_or("").to_string(),
        };
        by_chunk
            .entry(block.chunk_id.clone())
            .or_default()
            .push(block);
    }
    Ok(by_chunk)
}

/// All `doc_chunk_page_breaks` rows for a document (join through `doc_chunks`), grouped by
/// `chunk_id` and ordered by `offset_in_chunk`. One query for the whole document.
pub fn list_page_breaks_for_doc(
    db: &DbInstance,
    document_id: &str,
) -> Result<BTreeMap<String, Vec<PageBreak>>> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));
    let result = crate::cozo_retry::run_script_guarded(
        db,
        "?[chunk_id, offset_in_chunk, page] := \
         *doc_chunks{chunk_id, document_id}, document_id = $did, \
         *doc_chunk_page_breaks{chunk_id, offset_in_chunk, page}",
        params,
        ScriptMutability::Immutable,
        "list doc_chunk_page_breaks (doc)",
    )
    .map_err(|e| anyhow::anyhow!("list doc_chunk_page_breaks (doc) failed: {e}"))?;
    let mut by_chunk: BTreeMap<String, Vec<PageBreak>> = BTreeMap::new();
    for row in &result.rows {
        let brk = PageBreak {
            chunk_id: row[0].get_str().unwrap_or("").to_string(),
            offset_in_chunk: row[1].get_int().unwrap_or(0).max(0) as usize,
            page: row[2].get_int().unwrap_or(0) as u32,
        };
        by_chunk.entry(brk.chunk_id.clone()).or_default().push(brk);
    }
    for breaks in by_chunk.values_mut() {
        breaks.sort_by_key(|b| b.offset_in_chunk);
    }
    Ok(by_chunk)
}

/// Insert (upsert) a batch of `doc_chunk_blocks` rows.
pub fn insert_chunk_blocks(db: &DbInstance, blocks: &[ChunkBlock]) -> Result<()> {
    if blocks.is_empty() {
        return Ok(());
    }
    let rows: Vec<cozo::DataValue> = blocks
        .iter()
        .map(|b| {
            cozo::DataValue::List(vec![
                cozo::DataValue::from(b.chunk_id.as_str()),
                cozo::DataValue::from(b.block_idx as i64),
                cozo::DataValue::from(b.char_start as i64),
                cozo::DataValue::from(b.char_end as i64),
                cozo::DataValue::from(b.page as i64),
                cozo::DataValue::from(b.x0 as f64),
                cozo::DataValue::from(b.y0 as f64),
                cozo::DataValue::from(b.x1 as f64),
                cozo::DataValue::from(b.y1 as f64),
                cozo::DataValue::from(b.block_type.as_str()),
                cozo::DataValue::from(b.text_hash.as_str()),
            ])
        })
        .collect();
    let mut params = BTreeMap::new();
    params.insert("rows".into(), cozo::DataValue::List(rows));
    crate::cozo_retry::run_script_guarded(
        db,
        "?[chunk_id, block_idx, char_start, char_end, page, x0, y0, x1, y1, block_type, text_hash] \
         <- $rows \
         :put doc_chunk_blocks { chunk_id, block_idx => char_start, char_end, page, x0, y0, x1, y1, block_type, text_hash }",
        params,
        ScriptMutability::Mutable,
        "insert doc_chunk_blocks",
    )
    .map_err(|e| anyhow::anyhow!("insert doc_chunk_blocks failed: {e}"))?;
    Ok(())
}

/// All `doc_chunk_blocks` rows for a single chunk, ordered by `block_idx`.
pub fn list_chunk_blocks_for_chunk(db: &DbInstance, chunk_id: &str) -> Result<Vec<ChunkBlock>> {
    let mut params = BTreeMap::new();
    params.insert("cid".into(), DataValue::from(chunk_id));
    let result = crate::cozo_retry::run_script_guarded(
        db,
        "?[chunk_id, block_idx, char_start, char_end, page, x0, y0, x1, y1, block_type, text_hash] := \
         *doc_chunk_blocks{chunk_id, block_idx, char_start, char_end, page, x0, y0, x1, y1, block_type, text_hash}, \
         chunk_id = $cid \
         :order block_idx",
        params,
        ScriptMutability::Immutable,
        "list doc_chunk_blocks (chunk)",
    )
    .map_err(|e| anyhow::anyhow!("list doc_chunk_blocks (chunk) failed: {e}"))?;
    let mut out = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        out.push(ChunkBlock {
            chunk_id: row[0].get_str().unwrap_or("").to_string(),
            block_idx: row[1].get_int().unwrap_or(0) as u32,
            char_start: row[2].get_int().unwrap_or(0).max(0) as usize,
            char_end: row[3].get_int().unwrap_or(0).max(0) as usize,
            page: row[4].get_int().unwrap_or(0) as u32,
            x0: row[5].get_float().unwrap_or(0.0) as f32,
            y0: row[6].get_float().unwrap_or(0.0) as f32,
            x1: row[7].get_float().unwrap_or(0.0) as f32,
            y1: row[8].get_float().unwrap_or(0.0) as f32,
            block_type: row[9].get_str().unwrap_or("").to_string(),
            text_hash: row[10].get_str().unwrap_or("").to_string(),
        });
    }
    Ok(out)
}

/// Insert (upsert) a batch of `doc_chunk_page_breaks` rows.
pub fn insert_page_breaks(db: &DbInstance, breaks: &[PageBreak]) -> Result<()> {
    if breaks.is_empty() {
        return Ok(());
    }
    let rows: Vec<cozo::DataValue> = breaks
        .iter()
        .map(|b| {
            cozo::DataValue::List(vec![
                cozo::DataValue::from(b.chunk_id.as_str()),
                cozo::DataValue::from(b.offset_in_chunk as i64),
                cozo::DataValue::from(b.page as i64),
            ])
        })
        .collect();
    let mut params = BTreeMap::new();
    params.insert("rows".into(), cozo::DataValue::List(rows));
    crate::cozo_retry::run_script_guarded(
        db,
        "?[chunk_id, offset_in_chunk, page] <- $rows \
         :put doc_chunk_page_breaks { chunk_id, offset_in_chunk => page }",
        params,
        ScriptMutability::Mutable,
        "insert doc_chunk_page_breaks",
    )
    .map_err(|e| anyhow::anyhow!("insert doc_chunk_page_breaks failed: {e}"))?;
    Ok(())
}
