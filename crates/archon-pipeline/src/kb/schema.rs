//! CozoDB schema definitions and Rust types for the knowledge base.

use anyhow::Result;
use cozo::ScriptMutability;
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

pub const KB_EMBEDDINGS_SCHEMA: &str = "
:create kb_embeddings {
    node_id: String
    =>
    embedding: [Float]
}
";

/// Generate the CozoScript to create an HNSW index on embeddings.
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

/// Create all KB relations in the database. Idempotent — silently ignores
/// "already exists" errors so calling twice is safe.
pub fn ensure_kb_schema(db: &cozo::DbInstance) -> Result<()> {
    for script in [KB_NODES_SCHEMA, KB_EDGES_SCHEMA, KB_EMBEDDINGS_SCHEMA] {
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
    Ok(())
}
