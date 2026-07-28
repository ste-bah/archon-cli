//! CozoDB schema definitions and Rust types for the knowledge base.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, ScriptMutability, Vector};
use ndarray::Array1;
use serde::{Deserialize, Serialize};

#[path = "schema_state.rs"]
mod state;
pub(super) use state::{
    assert_embedding_space, lock_embedding_state, recover_interrupted_migration,
    rollback_embedding_activation,
};

// --- Rust types mirroring schema ---

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KbNodeType {
    Raw,
    Compiled,
    Concept,
    Answer,
    Index,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum KbEdgeType {
    Provenance,
    Backlink,
    CrossReference,
    ConceptOf,
    DerivedFrom,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KbNode {
    pub node_id: String,
    pub node_type: KbNodeType,
    pub source: String,
    pub domain_tag: String,
    pub title: String,
    pub content: String,
    pub content_hash: String,
    pub chunk_index: i64,
    pub created_at: f64,
    pub updated_at: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KbEdge {
    pub edge_id: String,
    pub source_node_id: String,
    pub target_node_id: String,
    pub edge_type: KbEdgeType,
    pub created_at: f64,
}

// --- CozoScript schema strings ---

pub const KB_NODES_SCHEMA: &str = "
:create kb_nodes {
    node_id: String
    =>
    node_type: String,
    source: String,
    domain_tag: String,
    title: String,
    content: String,
    content_hash: String,
    chunk_index: Int,
    created_at: Float,
    updated_at: Float
}
";

pub const KB_CONTENT_HASHES_SCHEMA: &str = "
:create kb_content_hashes {
    content_hash: String
    =>
    node_id: String
}
";
pub const KB_EDGES_SCHEMA: &str = "
:create kb_edges {
    edge_id: String
    =>
    source_node_id: String,
    target_node_id: String,
    edge_type: String,
    created_at: Float
}
";

pub const KB_EMBEDDINGS_SCHEMA: &str = "kb_embeddings";
const KB_EMBEDDING_CONFIG_SCHEMA: &str = "
:create kb_embedding_config {
    config_key: String
    =>
    provider: String,
    dimension: Int
}
";

const KB_EMBEDDING_MIGRATION_SCHEMA: &str = "
:create kb_embedding_migration {
    config_key: String
    =>
    provider: String,
    dimension: Int
}
";

pub(super) fn kb_embedding_storage_exists(db: &cozo::DbInstance) -> Result<bool> {
    relation_exists(db, KB_EMBEDDINGS_SCHEMA)
}

/// Create the derived embedding relation and HNSW index for the active provider.
#[cfg(test)]
pub(super) fn ensure_kb_embedding_schema(
    db: &cozo::DbInstance,
    provider: &str,
    dim: usize,
    embeddings: Option<&[(String, Vec<f32>)]>,
) -> Result<()> {
    let _guard = lock_embedding_state()?;
    ensure_kb_embedding_schema_locked(db, provider, dim, embeddings)
}

pub(super) fn ensure_kb_embedding_schema_locked(
    db: &cozo::DbInstance,
    provider: &str,
    dim: usize,
    embeddings: Option<&[(String, Vec<f32>)]>,
) -> Result<()> {
    run_create(db, KB_EMBEDDING_CONFIG_SCHEMA)?;
    run_create(db, KB_EMBEDDING_MIGRATION_SCHEMA)?;
    recover_interrupted_migration(db)?;
    let config = embedding_config(db)?;
    let config_matches = config
        .as_ref()
        .is_some_and(|(active_provider, active_dim)| {
            active_provider == provider && *active_dim == dim
        });
    if config_matches {
        return create_embedding_storage(db, dim);
    }
    rebuild_embedding_storage(
        db,
        provider,
        dim,
        embeddings.unwrap_or_default(),
        config.as_ref().map(|(_, dim)| *dim),
    )
}

pub(super) fn embedding_config(db: &cozo::DbInstance) -> Result<Option<(String, usize)>> {
    let result = db
        .run_script(
            "?[provider, dimension] := *kb_embedding_config{config_key, provider, dimension}, \
             config_key = 'active'",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|error| anyhow::anyhow!("read KB embedding config failed: {error}"))?;
    Ok(result.rows.first().and_then(|row| {
        Some((
            row[0].get_str()?.to_string(),
            usize::try_from(row[1].get_int()?).ok()?,
        ))
    }))
}

pub(super) fn restore_embedding_backup(
    db: &cozo::DbInstance,
    previous_dim: Option<usize>,
) -> Result<()> {
    drop_embedding_indices(db, "kb_embeddings")?;
    db.run_script(
        "::rename kb_embeddings -> kb_embeddings_staging, \
         kb_embeddings_backup -> kb_embeddings",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .map_err(|error| anyhow::anyhow!("restore KB embedding storage failed: {error}"))?;
    remove_relation_if_exists(db, "kb_embeddings_staging")?;
    if let Some(dim) = previous_dim {
        run_create(db, &hnsw_index_script("kb_embeddings", dim))?;
    }
    Ok(())
}

fn rebuild_embedding_storage(
    db: &cozo::DbInstance,
    provider: &str,
    dim: usize,
    embeddings: &[(String, Vec<f32>)],
    previous_dim: Option<usize>,
) -> Result<()> {
    const STAGING: &str = "kb_embeddings_staging";
    const BACKUP: &str = "kb_embeddings_backup";
    remove_relation_if_exists(db, STAGING)?;
    remove_relation_if_exists(db, BACKUP)?;
    run_create(db, &embedding_relation_script(STAGING, dim))?;
    store_embedding_rows(db, STAGING, embeddings)?;
    run_create(db, &hnsw_index_script(STAGING, dim))?;
    drop_embedding_indices(db, STAGING)?;
    store_embedding_migration(db, provider, dim)?;

    let had_active = kb_embedding_storage_exists(db)?;
    if had_active {
        drop_embedding_indices(db, "kb_embeddings")?;
        db.run_script(
            &format!("::rename kb_embeddings -> {BACKUP}, {STAGING} -> kb_embeddings"),
            Default::default(),
            ScriptMutability::Mutable,
        )
        .map_err(|error| anyhow::anyhow!("activate KB embedding storage failed: {error}"))?;
    } else {
        db.run_script(
            &format!("::rename {STAGING} -> kb_embeddings"),
            Default::default(),
            ScriptMutability::Mutable,
        )
        .map_err(|error| anyhow::anyhow!("activate KB embedding storage failed: {error}"))?;
    }
    if let Err(error) = run_create(db, &hnsw_index_script("kb_embeddings", dim)) {
        rollback_embedding_activation(db, had_active, previous_dim)?;
        return Err(error);
    }
    if let Err(error) = store_embedding_config(db, provider, dim) {
        rollback_embedding_activation(db, had_active, previous_dim)?;
        return Err(error);
    }
    if had_active {
        remove_relation_if_exists(db, BACKUP)?;
    }
    clear_embedding_migration(db)?;
    Ok(())
}

fn store_embedding_rows(
    db: &cozo::DbInstance,
    relation: &str,
    embeddings: &[(String, Vec<f32>)],
) -> Result<()> {
    if embeddings.is_empty() {
        return Ok(());
    }
    let rows = embeddings
        .iter()
        .map(|(node_id, embedding)| {
            DataValue::List(vec![
                DataValue::from(node_id.as_str()),
                DataValue::Vec(Vector::F32(Array1::from_vec(embedding.clone()))),
            ])
        })
        .collect();
    let mut params = BTreeMap::new();
    params.insert("rows".to_string(), DataValue::List(rows));
    db.run_script(
        &format!(
            "?[node_id, embedding] <- $rows\n         :put {relation} {{ node_id => embedding }}"
        ),
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|error| anyhow::anyhow!("prefill KB embedding storage failed: {error}"))?;
    Ok(())
}

pub(super) fn drop_embedding_indices(db: &cozo::DbInstance, relation: &str) -> Result<()> {
    let result = db
        .run_script(
            &format!("::indices {relation}"),
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|error| anyhow::anyhow!("list KB semantic indices failed: {error}"))?;
    let name_column = result
        .headers
        .iter()
        .position(|header| header == "name")
        .ok_or_else(|| anyhow::anyhow!("KB index listing omitted the name column"))?;
    for row in result.rows {
        let name = row[name_column]
            .get_str()
            .ok_or_else(|| anyhow::anyhow!("KB index listing contained a non-string name"))?;
        db.run_script(
            &format!("::index drop {relation}:{name}"),
            Default::default(),
            ScriptMutability::Mutable,
        )
        .map_err(|error| anyhow::anyhow!("drop KB semantic index failed: {error}"))?;
    }
    Ok(())
}

pub(super) fn embedding_migration(db: &cozo::DbInstance) -> Result<Option<(String, usize)>> {
    let result = db
        .run_script(
            "?[provider, dimension] := *kb_embedding_migration{config_key, provider, dimension}, \
             config_key = 'active'",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|error| anyhow::anyhow!("read KB embedding migration failed: {error}"))?;
    Ok(result.rows.first().and_then(|row| {
        Some((
            row[0].get_str()?.to_string(),
            usize::try_from(row[1].get_int()?).ok()?,
        ))
    }))
}

fn store_embedding_migration(db: &cozo::DbInstance, provider: &str, dim: usize) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("provider".to_string(), DataValue::from(provider));
    params.insert("dimension".to_string(), DataValue::from(dim as i64));
    db.run_script(
        "?[config_key, provider, dimension] <- [['active', $provider, $dimension]]\n         :put kb_embedding_migration { config_key => provider, dimension }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|error| anyhow::anyhow!("store KB embedding migration failed: {error}"))?;
    Ok(())
}

pub(super) fn clear_embedding_migration(db: &cozo::DbInstance) -> Result<()> {
    db.run_script(
        "?[config_key, provider, dimension] := *kb_embedding_migration{config_key, provider, dimension}, config_key = 'active'\n         :rm kb_embedding_migration { config_key => provider, dimension }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .map_err(|error| anyhow::anyhow!("clear KB embedding migration failed: {error}"))?;
    Ok(())
}

fn store_embedding_config(db: &cozo::DbInstance, provider: &str, dim: usize) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("provider".to_string(), DataValue::from(provider));
    params.insert("dimension".to_string(), DataValue::from(dim as i64));
    db.run_script(
        "?[config_key, provider, dimension] <- [['active', $provider, $dimension]]\n         :put kb_embedding_config { config_key => provider, dimension }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|error| anyhow::anyhow!("store KB embedding config failed: {error}"))?;
    Ok(())
}

pub(super) fn remove_relation_if_exists(db: &cozo::DbInstance, relation: &str) -> Result<()> {
    if relation_exists(db, relation)? {
        db.run_script(
            &format!("{{::remove {relation}}}"),
            Default::default(),
            ScriptMutability::Mutable,
        )
        .map_err(|error| anyhow::anyhow!("remove staged KB embedding storage failed: {error}"))?;
    }
    Ok(())
}

pub(super) fn relation_exists(db: &cozo::DbInstance, relation: &str) -> Result<bool> {
    let result = db
        .run_script(
            "::relations",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|error| anyhow::anyhow!("list KB relations failed: {error}"))?;
    let name_column = result
        .headers
        .iter()
        .position(|header| header == "name")
        .ok_or_else(|| anyhow::anyhow!("KB relation listing omitted the name column"))?;
    Ok(result
        .rows
        .iter()
        .any(|row| row[name_column].get_str() == Some(relation)))
}

fn create_embedding_storage(db: &cozo::DbInstance, dim: usize) -> Result<()> {
    run_create(db, &embedding_relation_script("kb_embeddings", dim))?;
    run_create(db, &hnsw_index_script("kb_embeddings", dim))
}

fn embedding_relation_script(relation: &str, dim: usize) -> String {
    format!(
        ":create {relation} {{
            node_id: String
            =>
            embedding: <F32; {dim}>
        }}"
    )
}

fn hnsw_index_script(relation: &str, dim: usize) -> String {
    format!(
        "::hnsw create {relation}:semantic_idx {{ \
            dim: {dim}, \
            dtype: F32, \
            fields: [embedding], \
            distance: Cosine, \
            ef_construction: 150, \
            m: 50 \
        }}"
    )
}

fn run_create(db: &cozo::DbInstance, script: &str) -> Result<()> {
    match db.run_script(script, Default::default(), ScriptMutability::Mutable) {
        Ok(_) => Ok(()),
        Err(error) => {
            let message = error.to_string();
            if message.contains("already exists")
                || message.contains("conflicts")
                || message.contains("index with the same name")
            {
                Ok(())
            } else {
                Err(anyhow::anyhow!("KB schema creation failed: {message}"))
            }
        }
    }
}

/// Create all active KB relations in the database. Idempotent — silently ignores
/// "already exists" errors so calling twice is safe.
pub fn ensure_kb_schema(db: &cozo::DbInstance) -> Result<()> {
    for script in [KB_NODES_SCHEMA, KB_CONTENT_HASHES_SCHEMA, KB_EDGES_SCHEMA] {
        match db.run_script(script, Default::default(), ScriptMutability::Mutable) {
            Ok(_) => {}
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("already exists") || msg.contains("conflicts") {
                    // Relation already exists — idempotent, skip
                } else {
                    return Err(anyhow::anyhow!("KB schema creation failed: {}", msg));
                }
            }
        }
    }
    backfill_content_hashes(db)
}

/// Populate the keyed hash relation for legacy databases. For a legacy hash
/// shared by multiple nodes, the lexicographically smallest node ID is kept.
fn backfill_content_hashes(db: &cozo::DbInstance) -> Result<()> {
    let nodes = db
        .run_script(
            "?[content_hash, node_id] := *kb_nodes{node_id, content_hash}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|error| anyhow::anyhow!("read legacy KB content hashes failed: {error}"))?;
    let mut chosen = std::collections::BTreeMap::new();
    for row in nodes.rows {
        let (Some(content_hash), Some(node_id)) = (row[0].get_str(), row[1].get_str()) else {
            continue;
        };
        if content_hash.is_empty() {
            continue;
        }
        chosen
            .entry(content_hash.to_owned())
            .and_modify(|current: &mut String| {
                if node_id < current.as_str() {
                    *current = node_id.to_owned();
                }
            })
            .or_insert_with(|| node_id.to_owned());
    }
    if chosen.is_empty() {
        return Ok(());
    }
    let rows = chosen
        .into_iter()
        .map(|(content_hash, node_id)| DataValue::List(vec![content_hash.into(), node_id.into()]))
        .collect();
    let mut params = BTreeMap::new();
    params.insert("rows".to_string(), DataValue::List(rows));
    db.run_script(
        "?[content_hash, node_id] <- $rows\n         :put kb_content_hashes { content_hash => node_id }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|error| anyhow::anyhow!("KB content hash backfill failed: {error}"))?;
    Ok(())
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
