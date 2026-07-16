//! CozoDB schema definitions and Rust types for the knowledge base.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, ScriptMutability};
use serde::{Deserialize, Serialize};

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

#[deprecated(note = "KB schema initialization no longer creates unused embedding storage")]
pub const KB_EMBEDDINGS_SCHEMA: &str = "
:create kb_embeddings {
    node_id: String
    =>
    embedding: [Float]
}
";

/// Generate the legacy CozoScript HNSW index definition.
#[deprecated(note = "KB queries use text matching; this helper is retained for compatibility")]
pub fn hnsw_index_script(dim: usize) -> String {
    format!(
        "::hnsw create kb_embeddings:semantic_idx {{ \
            dim: {dim}, \
            dtype: F32, \
            fields: [embedding], \
            distance: Cosine, \
            ef_construction: 150, \
            m: 50 \
        }}"
    )
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
