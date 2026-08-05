//! CozoDB HNSW vector index operations for memory embeddings.
//!
//! The `memory_embeddings` stored relation holds `(memory_id, embedding, provider)`
//! and an HNSW index is maintained for nearest-neighbour search.

use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability, Vector};

use crate::graph::helpers::run_mutable;
use crate::types::MemoryError;

/// Convert a CozoDB error to [`MemoryError`].
fn db_err(e: impl std::fmt::Display) -> MemoryError {
    MemoryError::Database(e.to_string())
}

fn empty_rows() -> NamedRows {
    NamedRows::new(vec![], vec![])
}

const SCHEMA_PROBE_ID: &str = "__archon_embedding_schema_probe__";

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
    create_embedding_relation(db, dim)?;
    create_embedding_index(db, dim)?;

    if let Err(error) = probe_embedding_schema(db, dim) {
        tracing::warn!(
            dim,
            error = %error,
            "memory_embeddings schema incompatible with provider; rebuilding derived embedding index"
        );
        rebuild_embedding_schema(db, dim)?;
        probe_embedding_schema(db, dim).map_err(|probe_error| {
            MemoryError::Database(format!(
                "memory_embeddings rebuild completed but {dim}-dim schema probe still failed: {probe_error}"
            ))
        })?;
    }

    Ok(())
}

fn create_embedding_relation(db: &DbInstance, dim: usize) -> Result<(), MemoryError> {
    let create_rel = format!(
        ":create memory_embeddings {{
            memory_id: String
            =>
            embedding: <F32; {dim}>,
            provider: String
        }}"
    );
    run_mutable(
        db,
        &create_rel,
        Default::default(),
        "memory vector search: create memory_embeddings relation",
    )
    .or_else(|e| {
        let msg = e.to_string();
        if msg.contains("already exists") || msg.contains("conflicts") {
            Ok(empty_rows())
        } else {
            Err(e)
        }
    })?;

    Ok(())
}

fn create_embedding_index(db: &DbInstance, dim: usize) -> Result<(), MemoryError> {
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
    run_mutable(
        db,
        &create_idx,
        Default::default(),
        "memory vector search: create memory_embeddings HNSW index",
    )
    .or_else(|e| {
        let msg = e.to_string();
        if msg.contains("already exists")
            || msg.contains("conflicts")
            || msg.contains("index with the same name")
        {
            Ok(empty_rows())
        } else {
            Err(e)
        }
    })?;

    Ok(())
}

fn rebuild_embedding_schema(db: &DbInstance, dim: usize) -> Result<(), MemoryError> {
    // Embeddings are derived data. If the provider dimension changes, drop only
    // the vector relation/index and let `archon memory reindex --all` rebuild
    // from the authoritative `memories` relation.
    let _ = run_mutable(
        db,
        "::index drop memory_embeddings:embedding_idx",
        Default::default(),
        "memory vector search: drop stale memory_embeddings index",
    );
    let _ = run_mutable(
        db,
        "{::remove memory_embeddings}",
        Default::default(),
        "memory vector search: remove stale memory_embeddings relation",
    );
    create_embedding_relation(db, dim)?;
    create_embedding_index(db, dim)
}

fn probe_embedding_schema(db: &DbInstance, dim: usize) -> Result<(), MemoryError> {
    let probe = vec![0.0_f32; dim];
    store_embedding(db, SCHEMA_PROBE_ID, &probe, "schema-probe", dim).map_err(|error| {
        MemoryError::Database(format!(
            "failed to store {dim}-dim schema probe in memory_embeddings: {error}"
        ))
    })?;
    if let Err(error) = delete_embedding(db, SCHEMA_PROBE_ID) {
        tracing::warn!(
            error = %error,
            "failed to remove memory_embeddings schema probe row"
        );
    }
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
    dim: usize,
) -> Result<(), MemoryError> {
    if embedding.len() != dim {
        return Err(MemoryError::Embedding(format!(
            "embedding dimension mismatch for memory {memory_id}: got {}, expected {dim}",
            embedding.len()
        )));
    }

    let arr = ndarray::Array1::from_vec(embedding.to_vec());

    let mut params = BTreeMap::new();
    params.insert("id".to_string(), DataValue::from(memory_id));
    params.insert("embedding".to_string(), DataValue::Vec(Vector::F32(arr)));
    params.insert("provider".to_string(), DataValue::from(provider));

    run_mutable(
        db,
        "?[memory_id, embedding, provider] <- [[$id, $embedding, $provider]]
         :put memory_embeddings { memory_id => embedding, provider }",
        params,
        "memory vector search: store memory embedding",
    )
    .map_err(|e| {
        MemoryError::Database(format!(
            "failed to store {}-dim embedding for memory {memory_id}; \
             memory_embeddings may have been created for a different provider dimension: {e}",
            embedding.len()
        ))
    })?;

    Ok(())
}

/// Read back the stored embedding for `memory_id`, if one exists.
///
/// Deduplication needs a query vector for a memory that is already stored, and
/// re-embedding its content to get one would mean an provider call per
/// comparison. The vector is already on disk; this reads it.
pub fn fetch_embedding(db: &DbInstance, memory_id: &str) -> Result<Option<Vec<f32>>, MemoryError> {
    let mut params = BTreeMap::new();
    params.insert("id".to_string(), DataValue::from(memory_id));

    let result = db
        .run_script(
            "?[embedding] := *memory_embeddings{memory_id: $id, embedding}",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(db_err)?;

    let Some(row) = result.rows.first() else {
        return Ok(None);
    };
    match row.first() {
        Some(DataValue::Vec(Vector::F32(arr))) => Ok(Some(arr.to_vec())),
        // A different vector width means the relation was built for another
        // provider. Reporting "absent" rather than erroring keeps the caller on
        // its non-vector fallback instead of failing the whole pass.
        _ => Ok(None),
    }
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

    run_mutable(
        db,
        "?[memory_id, embedding, provider] := *memory_embeddings{memory_id, embedding, provider}, memory_id = $id
         :rm memory_embeddings { memory_id => embedding, provider }",
        params,
        "memory vector search: delete memory embedding",
    )?;

    Ok(())
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
