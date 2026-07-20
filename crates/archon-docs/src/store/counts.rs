use anyhow::Result;
use cozo::{DbInstance, ScriptMutability};

pub fn count_image_descriptions(db: &DbInstance) -> Result<usize> {
    let result = db.run_script(
        "?[count(artifact_id)] := *doc_image_descriptions{artifact_id}",
        Default::default(),
        ScriptMutability::Immutable,
    );
    match result {
        Ok(result) => {
            if result.rows.is_empty() {
                return Ok(0);
            }
            Ok(result.rows[0][0].get_int().unwrap_or(0) as usize)
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains(crate::errors::COZO_RELATION_NOT_FOUND) {
                Ok(0)
            } else {
                Err(anyhow::anyhow!("count image descriptions failed: {msg}"))
            }
        }
    }
}
pub fn count_failed_chunks(db: &DbInstance) -> Result<usize> {
    let result = db
        .run_script(
            "?[count(chunk_id)] := *doc_chunks{chunk_id, embedding_status}, embedding_status = \"failed\"",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("count failed chunks failed: {e}"))?;
    if result.rows.is_empty() {
        return Ok(0);
    }
    Ok(result.rows[0][0].get_int().unwrap_or(0) as usize)
}

/// Count chunks with embedding_status = "pending".
pub fn count_pending_chunks(db: &DbInstance) -> Result<usize> {
    let result = db
        .run_script(
            "?[count(chunk_id)] := *doc_chunks{chunk_id, embedding_status}, embedding_status = \"pending\"",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("count pending chunks failed: {e}"))?;
    if result.rows.is_empty() {
        return Ok(0);
    }
    Ok(result.rows[0][0].get_int().unwrap_or(0) as usize)
}

/// Count chunks with embedding_status = "indexed".
pub fn count_indexed_chunks(db: &DbInstance) -> Result<usize> {
    let result = db
        .run_script(
            "?[count(chunk_id)] := *doc_chunks{chunk_id, embedding_status}, embedding_status = \"indexed\"",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("count indexed chunks failed: {e}"))?;
    if result.rows.is_empty() {
        return Ok(0);
    }
    Ok(result.rows[0][0].get_int().unwrap_or(0) as usize)
}

/// Count chunks currently stored.
pub fn count_chunks(db: &DbInstance) -> Result<usize> {
    let result = db.run_script(
        "?[count(chunk_id)] := *doc_chunks{chunk_id}",
        Default::default(),
        ScriptMutability::Immutable,
    );
    match result {
        Ok(result) => {
            if result.rows.is_empty() {
                return Ok(0);
            }
            Ok(result.rows[0][0].get_int().unwrap_or(0) as usize)
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains(crate::errors::COZO_RELATION_NOT_FOUND) {
                Ok(0)
            } else {
                Err(anyhow::anyhow!("count chunks failed: {msg}"))
            }
        }
    }
}

/// Count embeddings currently stored.
pub fn count_embeddings(db: &DbInstance) -> Result<usize> {
    let result = db.run_script(
        "?[count(chunk_id)] := *vec_text_chunks{chunk_id}",
        Default::default(),
        ScriptMutability::Immutable,
    );
    match result {
        Ok(result) => {
            if result.rows.is_empty() {
                return Ok(0);
            }
            Ok(result.rows[0][0].get_int().unwrap_or(0) as usize)
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains(crate::errors::COZO_RELATION_NOT_FOUND) {
                Ok(0)
            } else {
                Err(anyhow::anyhow!("count embeddings failed: {msg}"))
            }
        }
    }
}

pub fn count_page_image_embeddings(db: &DbInstance) -> Result<usize> {
    let result = db.run_script(
        "?[count(page_id)] := *vec_page_images{page_id}",
        Default::default(),
        ScriptMutability::Immutable,
    );
    match result {
        Ok(result) => {
            if result.rows.is_empty() {
                return Ok(0);
            }
            Ok(result.rows[0][0].get_int().unwrap_or(0) as usize)
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains(crate::errors::COZO_RELATION_NOT_FOUND) {
                Ok(0)
            } else {
                Err(anyhow::anyhow!("count page image embeddings failed: {msg}"))
            }
        }
    }
}
