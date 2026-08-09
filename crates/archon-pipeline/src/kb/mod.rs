//! Knowledge base — ingest, organize, and query external documents.
//!
//! # Two stores, mid-convergence
//!
//! [`compile`] and [`query`] operate on the document store `archon kb ingest`
//! actually writes (`doc_sources` / `doc_chunks`, owned by `archon-docs`), which
//! is what `PRD-ARCHON-DOCS-001` asked for under "Extend Carefully": the KB
//! schema evolving into document artifact lineage rather than becoming a second
//! unrelated store.
//!
//! [`ingest`] and the `kb_nodes` / `kb_edges` / `kb_content_hashes` /
//! `kb_embeddings` relations it writes are the other half of that convergence
//! and have no CLI caller. [`KnowledgeBase`] is their facade. Removing them is
//! a separate decision; until it is taken they are dead weight, not a second
//! supported path.

pub mod compile;
pub mod export;
pub mod ingest;
mod ingest_storage;
#[cfg(test)]
mod ingest_storage_test_hooks;
#[cfg(test)]
mod ingest_storage_tests;
pub mod lint;
pub mod query;
#[cfg(test)]
mod runtime_evidence_tests;
pub mod schema;
mod types;

pub use schema::{KbEdge, KbEdgeType, KbNode, KbNodeType};
pub use types::{IngestResult, IngestSource, KbStats, LintResult};

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use archon_docs::embed::LocalEmbeddingProvider;

/// Facade over the `kb_nodes` graph.
///
/// Every method except [`KnowledgeBase::export`] reads or writes `kb_nodes` and
/// has no CLI caller. `export` was repointed at the document store because a
/// `kb_nodes` export would dump an empty tree on any database an operator
/// actually built.
pub struct KnowledgeBase {
    db: cozo::DbInstance,
    ingester: ingest::Ingester,
}

impl KnowledgeBase {
    /// Create a new knowledge base, ensuring the schema exists.
    ///
    /// For a persisted database prefer [`KnowledgeBase::for_db_path`]: ingest
    /// can only serialise content-hash reservations against other handles on
    /// the same file when it knows where that file is.
    pub fn new(db: cozo::DbInstance) -> Result<Self> {
        Self::open(db, None)
    }

    /// Knowledge base over a persisted database at a known path.
    pub fn for_db_path(db: cozo::DbInstance, db_path: impl AsRef<std::path::Path>) -> Result<Self> {
        Self::open(db, Some(db_path.as_ref()))
    }

    fn open(db: cozo::DbInstance, db_path: Option<&std::path::Path>) -> Result<Self> {
        if let Some(embedder) = archon_docs::embed::get_provider() {
            return Self::from_shared_embedder(db, embedder, db_path);
        }
        schema::ensure_kb_schema(&db)?;
        let ingester = match db_path {
            Some(db_path) => ingest::Ingester::for_db_path(db.clone(), db_path)?,
            None => ingest::Ingester::new(db.clone())?,
        };
        Ok(Self { db, ingester })
    }

    pub fn with_embedder(
        db: cozo::DbInstance,
        embedder: Box<dyn LocalEmbeddingProvider>,
    ) -> Result<Self> {
        Self::from_shared_embedder(db, Arc::from(embedder), None)
    }

    /// [`KnowledgeBase::with_embedder`] for a persisted database at a known path.
    pub fn with_embedder_for_db_path(
        db: cozo::DbInstance,
        embedder: Box<dyn LocalEmbeddingProvider>,
        db_path: impl AsRef<std::path::Path>,
    ) -> Result<Self> {
        Self::from_shared_embedder(db, Arc::from(embedder), Some(db_path.as_ref()))
    }

    fn from_shared_embedder(
        db: cozo::DbInstance,
        embedder: Arc<dyn LocalEmbeddingProvider>,
        db_path: Option<&std::path::Path>,
    ) -> Result<Self> {
        schema::ensure_kb_schema(&db)?;
        let ingester = match db_path {
            Some(db_path) => ingest::Ingester::with_embedder_for_db_path(
                db.clone(),
                Arc::clone(&embedder),
                db_path,
            )?,
            None => ingest::Ingester::with_embedder(db.clone(), Arc::clone(&embedder))?,
        };
        Ok(Self { db, ingester })
    }

    /// Ingest content from the given source into the knowledge base.
    pub async fn ingest(&self, source: &IngestSource) -> Result<IngestResult> {
        self.ingester.ingest(source, None).await
    }

    /// Node IDs whose embedding vector is stored, in `kb_embeddings` order.
    ///
    /// The only remaining reader of the semantic index this facade's ingest
    /// path writes. `compile` and `query` moved to the document store, so
    /// without this the embedding-space migration machinery in
    /// [`schema`] would have no observable behaviour at all.
    pub async fn embedded_node_ids(&self) -> Result<Vec<String>> {
        if !schema::kb_embedding_storage_exists(&self.db)? {
            return Ok(Vec::new());
        }
        let result = self
            .db
            .run_script(
                "?[node_id] := *kb_embeddings{node_id} :order node_id",
                Default::default(),
                cozo::ScriptMutability::Immutable,
            )
            .map_err(|e| anyhow::anyhow!("read kb_embeddings failed: {}", e))?;
        Ok(result
            .rows
            .iter()
            .filter_map(|row| row[0].get_str().map(ToString::to_string))
            .collect())
    }

    /// Run lint checks over the knowledge base contents.
    pub async fn lint(&self) -> Result<LintResult> {
        Ok(LintResult::default())
    }

    /// List all nodes in the knowledge base, sorted by created_at descending.
    pub async fn list(&self) -> Result<Vec<KbNode>> {
        let result = self.db.run_script(
            "?[node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at] := \
             *kb_nodes{node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at} \
             :order -created_at",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        ).map_err(|e| anyhow::anyhow!("list query failed: {}", e))?;

        let nodes = result.rows.iter().map(|row| row_to_kb_node(row)).collect();
        Ok(nodes)
    }

    /// Return aggregate statistics about the knowledge base.
    pub async fn stats(&self) -> Result<KbStats> {
        // Count nodes by type
        let node_result = self
            .db
            .run_script(
                "?[node_type, count(node_id)] := *kb_nodes{node_id, node_type}",
                Default::default(),
                cozo::ScriptMutability::Immutable,
            )
            .map_err(|e| anyhow::anyhow!("stats node query failed: {}", e))?;

        let mut nodes_by_type = std::collections::HashMap::new();
        let mut total_nodes = 0usize;
        for row in &node_result.rows {
            let ntype = row[0].get_str().unwrap_or("unknown").to_string();
            let count = row[1].get_int().unwrap_or(0) as usize;
            nodes_by_type.insert(ntype, count);
            total_nodes += count;
        }

        // Count edges
        let edge_result = self
            .db
            .run_script(
                "?[count(edge_id)] := *kb_edges{edge_id}",
                Default::default(),
                cozo::ScriptMutability::Immutable,
            )
            .map_err(|e| anyhow::anyhow!("stats edge query failed: {}", e))?;

        let total_edges = edge_result
            .rows
            .first()
            .and_then(|r| r[0].get_int())
            .unwrap_or(0) as usize;

        Ok(KbStats {
            total_nodes,
            total_edges,
            nodes_by_type,
        })
    }

    /// Search for nodes matching the given query string (simple text search).
    ///
    /// This is the non-LLM search: filters nodes by title/content containing
    /// the query substring. Use `query()` for text retrieval with graph context
    /// and optional answer synthesis.
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<KbNode>> {
        let mut params = std::collections::BTreeMap::new();
        params.insert("q".to_string(), cozo::DataValue::from(query));
        params.insert("lim".to_string(), cozo::DataValue::from(limit as i64));

        let result = self.db.run_script(
            "?[node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at] := \
             *kb_nodes{node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at}, \
             (str_includes(title, $q) or str_includes(content, $q)) \
             :limit $lim",
            params,
            cozo::ScriptMutability::Immutable,
        ).map_err(|e| anyhow::anyhow!("search query failed: {}", e))?;

        let nodes = result.rows.iter().map(|row| row_to_kb_node(row)).collect();
        Ok(nodes)
    }

    /// Delete a node by ID, cascading to related edges and derived nodes.
    ///
    /// Cascade logic:
    /// 1. Find all nodes that have a DerivedFrom edge pointing to this node
    /// 2. Delete those derived nodes (recursively)
    /// 3. Delete all edges where this node is source or target
    /// 4. Atomically remove the node and its owned content-hash reservation
    pub async fn delete(&self, node_id: &str) -> Result<()> {
        let mut params = std::collections::BTreeMap::new();
        params.insert("nid".to_string(), cozo::DataValue::from(node_id));

        // 1. Find derived nodes (DerivedFrom edges where target = this node)
        let derived = self
            .db
            .run_script(
                "?[source_node_id] := *kb_edges{source_node_id, target_node_id, edge_type}, \
             target_node_id = $nid, edge_type = 'DerivedFrom'",
                params.clone(),
                cozo::ScriptMutability::Immutable,
            )
            .map_err(|e| anyhow::anyhow!("find derived failed: {}", e))?;

        // 2. Recursively delete derived nodes
        for row in &derived.rows {
            if let Some(derived_id) = row[0].get_str() {
                // Use Box::pin for recursive async
                Box::pin(self.delete(derived_id)).await?;
            }
        }

        // 3. Delete all edges where this node is source or target
        self.db
            .run_script(
                "?[edge_id, source_node_id, target_node_id, edge_type, created_at] := \
             *kb_edges{edge_id, source_node_id, target_node_id, edge_type, created_at}, \
             (source_node_id = $nid or target_node_id = $nid) \
             :rm kb_edges { edge_id => source_node_id, target_node_id, edge_type, created_at }",
                params.clone(),
                cozo::ScriptMutability::Mutable,
            )
            .map_err(|e| anyhow::anyhow!("delete edges failed: {}", e))?;

        // 4. Remove the node and its hash mapping in one transaction. The
        // mapping is conditional so deleting a legacy duplicate never removes
        // another node's keyed ownership reservation.
        let _embedding_guard = schema::lock_embedding_state()?;
        let has_embedding_storage = schema::kb_embedding_storage_exists(&self.db)?;
        let transaction = self.db.multi_transaction(true);
        let content_hash = transaction
            .run_script(
                "?[content_hash] := *kb_nodes{node_id, content_hash}, node_id = $nid",
                params.clone(),
            )
            .map_err(|error| anyhow::anyhow!("read node before deletion failed: {error}"))?
            .rows
            .first()
            .and_then(|row| row[0].get_str())
            .map(str::to_owned);
        if has_embedding_storage
            && let Err(error) = transaction.run_script(
                "?[node_id, embedding] := *kb_embeddings{node_id, embedding}, node_id = $nid\n                 :rm kb_embeddings { node_id => embedding }",
                params.clone(),
            )
        {
            let _ = transaction.abort();
            return Err(anyhow::anyhow!("delete node embedding failed: {error}"));
        }
        if let Err(error) = transaction.run_script(
            "?[node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at] := \
             *kb_nodes{node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at}, \
             node_id = $nid \
             :rm kb_nodes { node_id => node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at }",
            params.clone(),
        ) {
            let _ = transaction.abort();
            return Err(anyhow::anyhow!("delete node failed: {error}"));
        }
        if let Some(content_hash) = content_hash
            && !content_hash.is_empty()
        {
            let mut hash_params = std::collections::BTreeMap::new();
            hash_params.insert("chash".to_string(), cozo::DataValue::from(content_hash));
            hash_params.insert("nid".to_string(), cozo::DataValue::from(node_id));
            if let Err(error) = transaction.run_script(
                "?[content_hash, node_id] := *kb_content_hashes{content_hash, node_id}, \
                 content_hash = $chash, node_id = $nid \
                 :rm kb_content_hashes { content_hash => node_id }",
                hash_params,
            ) {
                let _ = transaction.abort();
                return Err(anyhow::anyhow!(
                    "delete content-hash mapping failed: {error}"
                ));
            }
        }
        transaction
            .commit()
            .map_err(|error| anyhow::anyhow!("commit node deletion failed: {error}"))?;

        Ok(())
    }

    /// Export the knowledge base to a directory of markdown files.
    ///
    /// Delegates to [`export`], which reads the document store. Exporting
    /// `kb_nodes` would dump an empty tree on any database an operator built.
    pub async fn export(&self, path: &Path) -> Result<export::ExportSummary> {
        export::export_to_directory(&self.db, path, &export::ExportOptions::default())
    }

    /// The handle this knowledge base was opened over.
    pub fn db(&self) -> &cozo::DbInstance {
        &self.db
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Convert a CozoDB row to a KbNode.
fn row_to_kb_node(row: &[cozo::DataValue]) -> KbNode {
    KbNode {
        node_id: row[0].get_str().unwrap_or("").to_string(),
        node_type: str_to_node_type(row[1].get_str().unwrap_or("raw")),
        source: row[2].get_str().unwrap_or("").to_string(),
        domain_tag: row[3].get_str().unwrap_or("").to_string(),
        title: row[4].get_str().unwrap_or("").to_string(),
        content: row[5].get_str().unwrap_or("").to_string(),
        content_hash: row[6].get_str().unwrap_or("").to_string(),
        chunk_index: row[7].get_int().unwrap_or(0),
        created_at: row[8].get_float().unwrap_or(0.0),
        updated_at: row[9].get_float().unwrap_or(0.0),
    }
}

fn str_to_node_type(s: &str) -> KbNodeType {
    match s {
        "raw" => KbNodeType::Raw,
        "compiled" => KbNodeType::Compiled,
        "concept" => KbNodeType::Concept,
        "answer" => KbNodeType::Answer,
        "index" => KbNodeType::Index,
        _ => KbNodeType::Raw,
    }
}
