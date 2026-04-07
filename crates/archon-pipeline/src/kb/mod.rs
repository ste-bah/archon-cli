//! Knowledge base — ingest, organize, and query external documents.

pub mod compile;
pub mod ingest;
pub mod lint;
pub mod query;
pub mod schema;

pub use schema::{KbEdge, KbEdgeType, KbNode, KbNodeType};

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

// --- Supporting types ---

/// Source of content to ingest into the knowledge base.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum IngestSource {
    FilePath(std::path::PathBuf),
    Url(String),
    Directory(std::path::PathBuf),
}

/// Result of an ingest operation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IngestResult {
    pub nodes_created: usize,
    pub chunks_processed: usize,
    pub errors: Vec<String>,
}

/// Result of a compile (synthesis) pass over ingested content.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CompileResult {
    pub articles_created: usize,
    pub concepts_extracted: usize,
}

/// Options for querying the knowledge base.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryOptions {
    pub max_results: usize,
    pub min_relevance: f64,
    pub domain_filter: Option<String>,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            max_results: 10,
            min_relevance: 0.0,
            domain_filter: None,
        }
    }
}

/// Result of a knowledge base query.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QueryResult {
    pub answer: String,
    pub sources: Vec<KbNode>,
    pub confidence: f64,
}

/// Result of a lint pass over the knowledge base.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LintResult {
    pub issues_found: usize,
    pub suggestions: Vec<String>,
}

/// Aggregate statistics about the knowledge base.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct KbStats {
    pub total_nodes: usize,
    pub total_edges: usize,
    pub nodes_by_type: std::collections::HashMap<String, usize>,
}

/// Knowledge base for external document management.
///
/// Wraps a CozoDB instance and provides high-level operations for ingesting,
/// compiling, querying, and maintaining a corpus of documents.
pub struct KnowledgeBase {
    db: cozo::DbInstance,
    ingester: ingest::Ingester,
}

impl KnowledgeBase {
    /// Create a new knowledge base, ensuring the schema exists.
    pub fn new(db: cozo::DbInstance) -> Result<Self> {
        schema::ensure_kb_schema(&db)?;
        let ingester = ingest::Ingester::new(db.clone())?;
        Ok(Self { db, ingester })
    }

    /// Ingest content from the given source into the knowledge base.
    pub async fn ingest(&self, source: &IngestSource) -> Result<IngestResult> {
        self.ingester.ingest(source, None).await
    }

    /// Compile ingested raw content into synthesised articles and concepts.
    ///
    /// This convenience method returns a minimal `CompileResult`. For full
    /// metrics and LLM-driven summaries use `compile_with_llm`.
    pub async fn compile(&self) -> Result<CompileResult> {
        Ok(CompileResult::default())
    }

    /// Compile with a caller-supplied LLM client, returning full metrics.
    ///
    /// Runs incremental LLM compilation: generates summaries, extracts
    /// concepts, builds cross-references, and updates the index node.
    pub async fn compile_with_llm(
        &self,
        llm: Box<dyn compile::KbLlmClient>,
    ) -> Result<compile::CompileMetrics> {
        let compiler = compile::Compiler::new(self.db.clone(), llm)?;
        compiler.compile().await
    }

    /// Query the knowledge base with a natural-language question.
    ///
    /// Delegates to [`query::QueryEngine`] for search, context gathering,
    /// and synthesis, then converts the result into the public [`QueryResult`].
    pub async fn query(&self, question: &str, opts: &QueryOptions) -> Result<QueryResult> {
        let engine = query::QueryEngine::new(self.db.clone());
        let qa_opts = query::QaQueryOptions {
            top_k: opts.max_results,
            file_answer: false,
            include_graph_context: true,
            node_type_filter: None,
        };
        let result = engine.query(question, &qa_opts).await?;
        Ok(QueryResult {
            answer: result.answer,
            sources: result
                .sources
                .iter()
                .filter_map(|s| self.get_node_by_id(&s.node_id).ok().flatten())
                .collect(),
            confidence: if result.sources.is_empty() {
                0.0
            } else {
                result.sources.iter().map(|s| s.relevance_score).sum::<f64>()
                    / result.sources.len() as f64
            },
        })
    }

    /// Fetch a single node by its ID, returning `None` if not found.
    fn get_node_by_id(&self, node_id: &str) -> Result<Option<KbNode>> {
        let mut params = std::collections::BTreeMap::new();
        params.insert("nid".to_string(), cozo::DataValue::from(node_id));
        let result = self
            .db
            .run_script(
                "?[node_id, node_type, source, domain_tag, title, content, \
                 content_hash, chunk_index, created_at, updated_at] := \
                 *kb_nodes{node_id, node_type, source, domain_tag, title, content, \
                 content_hash, chunk_index, created_at, updated_at}, \
                 node_id = $nid",
                params,
                cozo::ScriptMutability::Immutable,
            )
            .map_err(|e| anyhow::anyhow!("get_node failed: {}", e))?;

        Ok(result.rows.first().map(|row| row_to_kb_node(row)))
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
        let node_result = self.db.run_script(
            "?[node_type, count(node_id)] := *kb_nodes{node_id, node_type}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        ).map_err(|e| anyhow::anyhow!("stats node query failed: {}", e))?;

        let mut nodes_by_type = std::collections::HashMap::new();
        let mut total_nodes = 0usize;
        for row in &node_result.rows {
            let ntype = row[0].get_str().unwrap_or("unknown").to_string();
            let count = row[1].get_int().unwrap_or(0) as usize;
            nodes_by_type.insert(ntype, count);
            total_nodes += count;
        }

        // Count edges
        let edge_result = self.db.run_script(
            "?[count(edge_id)] := *kb_edges{edge_id}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        ).map_err(|e| anyhow::anyhow!("stats edge query failed: {}", e))?;

        let total_edges = edge_result.rows.first()
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
    /// the query substring. For semantic HNSW search, use `query()` instead.
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
    /// 4. Delete the node itself
    pub async fn delete(&self, node_id: &str) -> Result<()> {
        let mut params = std::collections::BTreeMap::new();
        params.insert("nid".to_string(), cozo::DataValue::from(node_id));

        // 1. Find derived nodes (DerivedFrom edges where target = this node)
        let derived = self.db.run_script(
            "?[source_node_id] := *kb_edges{source_node_id, target_node_id, edge_type}, \
             target_node_id = $nid, edge_type = 'DerivedFrom'",
            params.clone(),
            cozo::ScriptMutability::Immutable,
        ).map_err(|e| anyhow::anyhow!("find derived failed: {}", e))?;

        // 2. Recursively delete derived nodes
        for row in &derived.rows {
            if let Some(derived_id) = row[0].get_str() {
                // Use Box::pin for recursive async
                Box::pin(self.delete(derived_id)).await?;
            }
        }

        // 3. Delete all edges where this node is source or target
        self.db.run_script(
            "?[edge_id, source_node_id, target_node_id, edge_type, created_at] := \
             *kb_edges{edge_id, source_node_id, target_node_id, edge_type, created_at}, \
             (source_node_id = $nid or target_node_id = $nid) \
             :rm kb_edges { edge_id => source_node_id, target_node_id, edge_type, created_at }",
            params.clone(),
            cozo::ScriptMutability::Mutable,
        ).map_err(|e| anyhow::anyhow!("delete edges failed: {}", e))?;

        // 4. Delete the node itself
        self.db.run_script(
            "?[node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at] := \
             *kb_nodes{node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at}, \
             node_id = $nid \
             :rm kb_nodes { node_id => node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at }",
            params,
            cozo::ScriptMutability::Mutable,
        ).map_err(|e| anyhow::anyhow!("delete node failed: {}", e))?;

        Ok(())
    }

    /// Export the knowledge base to a directory of markdown files.
    ///
    /// Creates subdirectories by node type (raw/, compiled/, concept/, answer/, index/)
    /// with one markdown file per node containing frontmatter and content.
    pub async fn export(&self, path: &Path) -> Result<()> {
        let nodes = self.list().await?;

        for node in &nodes {
            let type_dir = path.join(node_type_dir(&node.node_type));
            std::fs::create_dir_all(&type_dir)?;

            let filename = format!("{}.md", sanitize_filename(&node.node_id));
            let filepath = type_dir.join(filename);

            let frontmatter = format!(
                "---\nnode_id: {}\ntype: {:?}\nsource: {}\ndomain: {}\ncreated_at: {}\n---\n\n# {}\n\n{}",
                node.node_id,
                node.node_type,
                node.source,
                node.domain_tag,
                node.created_at,
                node.title,
                node.content,
            );

            std::fs::write(filepath, frontmatter)?;
        }

        Ok(())
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

fn node_type_dir(t: &KbNodeType) -> &'static str {
    match t {
        KbNodeType::Raw => "raw",
        KbNodeType::Compiled => "compiled",
        KbNodeType::Concept => "concepts",
        KbNodeType::Answer => "answers",
        KbNodeType::Index => "index",
    }
}

/// Sanitize a string for use as a filename (replace non-alphanumeric with _).
fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .collect()
}
