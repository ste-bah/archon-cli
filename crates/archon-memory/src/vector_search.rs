//! CozoDB HNSW vector index operations for memory embeddings.
//!
//! The `memory_embeddings` stored relation holds `(memory_id, embedding, provider)`
//! and an HNSW index is maintained for nearest-neighbour search.

use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability, Vector};

use crate::types::MemoryError;

/// Convert a CozoDB error to [`MemoryError`].
fn db_err(e: impl std::fmt::Display) -> MemoryError {
    MemoryError::Database(e.to_string())
}

fn empty_rows() -> NamedRows {
    NamedRows::new(vec![], vec![])
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

/// Create the `memory_embeddings` stored relation and its HNSW index.
///
/// The dimension is baked into the DDL as a literal integer because CozoDB
/// does not support parameterised DDL for vector dimensions.
///
/// This function is idempotent: calling it multiple times is safe.
pub fn init_embedding_schema(db: &DbInstance, dim: usize) -> Result<(), MemoryError> {
    // Create the stored relation
    let create_rel = format!(
        ":create memory_embeddings {{
            memory_id: String
            =>
            embedding: <F32; {dim}>,
            provider: String
        }}"
    );
    db.run_script(&create_rel, Default::default(), ScriptMutability::Mutable)
        .or_else(|e| {
            let msg = e.to_string();
            if msg.contains("already exists") || msg.contains("conflicts") {
                Ok(empty_rows())
            } else {
                Err(db_err(e))
            }
        })?;

    // Create the HNSW index
    let create_idx = format!(
        "::hnsw create memory_embeddings:embedding_idx {{
            dim: {dim},
            m: 50,
            dtype: F32,
            fields: [embedding],
            distance: Cosine,
            ef_construction: 200
        }}"
    );
    db.run_script(&create_idx, Default::default(), ScriptMutability::Mutable)
        .or_else(|e| {
            let msg = e.to_string();
            if msg.contains("already exists")
                || msg.contains("conflicts")
                || msg.contains("index with the same name")
            {
                Ok(empty_rows())
            } else {
                Err(db_err(e))
            }
        })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// CRUD
// ---------------------------------------------------------------------------

/// Store (or upsert) an embedding for a memory.
pub fn store_embedding(
    db: &DbInstance,
    memory_id: &str,
    embedding: &[f32],
    provider: &str,
    _dim: usize,
) -> Result<(), MemoryError> {
    let arr = ndarray::Array1::from_vec(embedding.to_vec());

    let mut params = BTreeMap::new();
    params.insert("id".to_string(), DataValue::from(memory_id));
    params.insert("embedding".to_string(), DataValue::Vec(Vector::F32(arr)));
    params.insert("provider".to_string(), DataValue::from(provider));

    db.run_script(
        "?[memory_id, embedding, provider] <- [[$id, $embedding, $provider]]
         :put memory_embeddings { memory_id => embedding, provider }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(db_err)?;

    Ok(())
}

/// Delete the embedding for a memory (no-op if it doesn't exist).
pub fn delete_embedding(db: &DbInstance, memory_id: &str) -> Result<(), MemoryError> {
    let mut params = BTreeMap::new();
    params.insert("id".to_string(), DataValue::from(memory_id));

    // First check if the row exists
    let result = db
        .run_script(
            "?[memory_id, embedding, provider] := *memory_embeddings{memory_id, embedding, provider}, memory_id = $id",
            params.clone(),
            ScriptMutability::Immutable,
        )
        .map_err(db_err)?;

    if result.rows.is_empty() {
        return Ok(());
    }

    db.run_script(
        "?[memory_id, embedding, provider] := *memory_embeddings{memory_id, embedding, provider}, memory_id = $id
         :rm memory_embeddings { memory_id => embedding, provider }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(db_err)?;

    Ok(())
}

/// Count the number of stored embeddings.
pub fn embedding_count(db: &DbInstance) -> Result<usize, MemoryError> {
    let result = db
        .run_script(
            "?[count(memory_id)] := *memory_embeddings{memory_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(db_err)?;

    let count = result
        .rows
        .first()
        .and_then(|row| row[0].get_int())
        .unwrap_or(0);

    Ok(count as usize)
}

/// Search for the `top_k` most similar embeddings to `query_vec`.
///
/// Returns `(memory_id, cosine_distance)` pairs sorted by distance ascending
/// (closest first).
pub fn search_similar(
    db: &DbInstance,
    query_vec: &[f32],
    top_k: usize,
) -> Result<Vec<(String, f64)>, MemoryError> {
    let arr = ndarray::Array1::from_vec(query_vec.to_vec());

    let mut params = BTreeMap::new();
    params.insert("query".to_string(), DataValue::Vec(Vector::F32(arr)));
    params.insert("k".to_string(), DataValue::from(top_k as i64));

    let query = "?[memory_id, distance] := ~memory_embeddings:embedding_idx{
            memory_id,
            |
            query: $query,
            k: $k,
            ef: 50,
            bind_distance: distance
        }";

    let result = db
        .run_script(query, params, ScriptMutability::Immutable)
        .map_err(db_err)?;

    let mut results = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        let id = row[0].get_str().unwrap_or("").to_string();
        let distance = row[1].get_float().unwrap_or(1.0);
        results.push((id, distance));
    }

    // Sort by distance ascending (closest first)
    results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(results)
}

/// Drop the embeddings relation and its HNSW index.
///
/// Useful when switching providers (different dimension).
pub fn drop_embeddings(db: &DbInstance) -> Result<(), MemoryError> {
    // Drop index first
    db.run_script(
        "::hnsw drop memory_embeddings:embedding_idx",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .or_else(|e| {
        let msg = e.to_string();
        if msg.contains("not found") || msg.contains("does not exist") {
            Ok(empty_rows())
        } else {
            Err(db_err(e))
        }
    })?;

    // Drop relation
    db.run_script(
        "::remove memory_embeddings",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .or_else(|e| {
        let msg = e.to_string();
        if msg.contains("not found") || msg.contains("does not exist") {
            Ok(empty_rows())
        } else {
            Err(db_err(e))
        }
    })?;

    Ok(())
}
