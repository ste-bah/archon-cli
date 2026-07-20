use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability, Vector};

use crate::models::ChunkArtifact;

use super::chunks::{get_chunk_by_id, insert_chunk};

pub fn insert_chunk_embedding(
    db: &DbInstance,
    chunk_id: &str,
    embedding: &[f32],
    provider: &str,
) -> Result<()> {
    let arr = ndarray::Array1::from_vec(embedding.to_vec());

    let mut params = BTreeMap::new();
    params.insert("cid".into(), DataValue::from(chunk_id));
    params.insert("emb".into(), DataValue::Vec(Vector::F32(arr)));
    params.insert("prov".into(), DataValue::from(provider));

    crate::cozo_retry::run_script_guarded(
        db,
        "?[chunk_id, embedding, provider] <- [[$cid, $emb, $prov]]
         :put vec_text_chunks { chunk_id => embedding, provider }",
        params,
        ScriptMutability::Mutable,
        "insert chunk embedding",
    )
    .map_err(|e| anyhow::anyhow!("insert chunk embedding failed: {e}"))?;
    Ok(())
}

pub struct ChunkEmbeddingInput<'a> {
    pub chunk_id: &'a str,
    pub embedding: &'a [f32],
}

/// Store embeddings for multiple chunks in one Cozo mutation.
pub fn insert_chunk_embeddings(
    db: &DbInstance,
    rows: &[ChunkEmbeddingInput<'_>],
    provider: &str,
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }

    let mut params = BTreeMap::new();
    params.insert("prov".into(), DataValue::from(provider));
    let mut tuples = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let cid_key = format!("cid{index}");
        let emb_key = format!("emb{index}");
        let arr = ndarray::Array1::from_vec(row.embedding.to_vec());
        params.insert(cid_key.clone(), DataValue::from(row.chunk_id));
        params.insert(emb_key.clone(), DataValue::Vec(Vector::F32(arr)));
        tuples.push(format!("[${cid_key}, ${emb_key}, $prov]"));
    }

    let script = format!(
        "?[chunk_id, embedding, provider] <- [{}]\n\
         :put vec_text_chunks {{ chunk_id => embedding, provider }}",
        tuples.join(", ")
    );
    crate::cozo_retry::run_script_guarded(
        db,
        &script,
        params,
        ScriptMutability::Mutable,
        "bulk insert chunk embeddings",
    )
    .map_err(|e| anyhow::anyhow!("bulk insert chunk embeddings failed: {e}"))?;
    Ok(())
}

/// Read back a stored embedding for a chunk. Returns None if not found.
pub fn get_chunk_embedding(db: &DbInstance, chunk_id: &str) -> Result<Option<Vec<f32>>> {
    let mut params = BTreeMap::new();
    params.insert("cid".into(), DataValue::from(chunk_id));

    let result = crate::cozo_retry::run_script_guarded(
        db,
        "?[embedding] := *vec_text_chunks{chunk_id, embedding}, chunk_id = $cid",
        params,
        ScriptMutability::Immutable,
        "get chunk embedding",
    )
    .map_err(|e| anyhow::anyhow!("get chunk embedding failed: {e}"))?;

    if result.rows.is_empty() {
        return Ok(None);
    }
    // Extract the embedding from the first row
    let emb = &result.rows[0][0];
    match emb {
        DataValue::Vec(Vector::F32(arr)) => Ok(Some(arr.to_vec())),
        DataValue::Vec(_) => Err(anyhow::anyhow!(
            "unexpected vector dtype in vec_text_chunks"
        )),
        _ => Err(anyhow::anyhow!(
            "expected vector data in vec_text_chunks for chunk {chunk_id}"
        )),
    }
}

/// Update the embedding_status field for a chunk.
///
/// Reads the full chunk row, modifies only the status, and re-inserts.
/// Returns Ok(()) even if the chunk doesn't exist (no-op).
pub fn update_chunk_embedding_status(db: &DbInstance, chunk_id: &str, status: &str) -> Result<()> {
    // Read the full chunk first — :put requires all non-default columns.
    let chunk = get_chunk_by_id(db, chunk_id)?;
    if let Some(mut chunk) = chunk {
        chunk.embedding_status = status.to_string();
        insert_chunk(db, &chunk)?;
    }
    Ok(())
}

/// Update embedding_status for multiple already-loaded chunks in one mutation.
pub fn update_chunk_embedding_statuses(
    db: &DbInstance,
    chunks: &[&ChunkArtifact],
    status: &str,
) -> Result<()> {
    if chunks.is_empty() {
        return Ok(());
    }

    let mut params = BTreeMap::new();
    params.insert("status".into(), DataValue::from(status));
    let mut tuples = Vec::with_capacity(chunks.len());
    for (index, chunk) in chunks.iter().enumerate() {
        let cid = format!("cid{index}");
        let did = format!("did{index}");
        let aid = format!("aid{index}");
        let cix = format!("cix{index}");
        let ps = format!("ps{index}");
        let pe = format!("pe{index}");
        let txt = format!("txt{index}");
        let hash = format!("hash{index}");
        params.insert(cid.clone(), DataValue::from(chunk.chunk_id.as_str()));
        params.insert(did.clone(), DataValue::from(chunk.document_id.as_str()));
        params.insert(aid.clone(), DataValue::from(chunk.artifact_id.as_str()));
        params.insert(cix.clone(), DataValue::from(chunk.chunk_index as i64));
        params.insert(ps.clone(), DataValue::from(chunk.page_start as i64));
        params.insert(pe.clone(), DataValue::from(chunk.page_end as i64));
        params.insert(txt.clone(), DataValue::from(chunk.content.as_str()));
        params.insert(hash.clone(), DataValue::from(chunk.content_hash.as_str()));
        tuples.push(format!(
            "[${cid}, ${did}, ${aid}, ${cix}, ${ps}, ${pe}, ${txt}, ${hash}, $status]"
        ));
    }

    let script = format!(
        "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] <- [{}]\n\
         :put doc_chunks {{ chunk_id => document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status }}",
        tuples.join(", ")
    );
    crate::cozo_retry::run_script_guarded(
        db,
        &script,
        params,
        ScriptMutability::Mutable,
        "bulk update chunk embedding status",
    )
    .map_err(|e| anyhow::anyhow!("bulk update chunk embedding status failed: {e}"))?;
    Ok(())
}

pub fn insert_page_image_embedding(
    db: &DbInstance,
    page_id: &str,
    embedding: &[f32],
    provider: &str,
) -> Result<()> {
    let arr = ndarray::Array1::from_vec(embedding.to_vec());

    let mut params = BTreeMap::new();
    params.insert("pid".into(), DataValue::from(page_id));
    params.insert("emb".into(), DataValue::Vec(Vector::F32(arr)));
    params.insert("prov".into(), DataValue::from(provider));

    crate::cozo_retry::run_script_guarded(
        db,
        "?[page_id, embedding, provider] <- [[$pid, $emb, $prov]]
         :put vec_page_images { page_id => embedding, provider }",
        params,
        ScriptMutability::Mutable,
        "insert page image embedding",
    )
    .map_err(|e| anyhow::anyhow!("insert page image embedding failed: {e}"))?;
    Ok(())
}
