//! KB LLM Compilation — summaries, concepts, cross-references, index.
//!
//! Implements REQ-KB-002. NFR-PIPE-012: 20 docs in < 5 minutes.
//!
//! The `Compiler` accepts an abstract `KbLlmClient` so that `archon-pipeline`
//! does not need a hard dependency on `archon-llm`.

use std::collections::BTreeMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::{KbEdgeType, KbNode, KbNodeType};

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Abstract LLM completion interface for KB compilation.
///
/// Implementors call the actual LLM (archon-llm, mock, etc.).
#[async_trait::async_trait]
pub trait KbLlmClient: Send + Sync {
    /// Send a prompt and return the completion as text.
    async fn complete(&self, prompt: &str) -> Result<String>;
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Metrics returned after a compile pass.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompileMetrics {
    pub summaries_generated: usize,
    pub concepts_extracted: usize,
    pub edges_created: usize,
    pub index_updated: bool,
    pub duration_secs: f64,
}

/// Result of compiling a single document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentCompilation {
    pub node_id: String,
    pub summary: String,
    pub concepts: Vec<ConceptArticle>,
}

/// A concept extracted from one or more documents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConceptArticle {
    pub name: String,
    pub explanation: String,
    pub source_nodes: Vec<String>,
    /// The KB node ID assigned when the concept is stored.
    /// Not returned by the LLM — populated by `extract_concepts()`.
    #[serde(default)]
    pub node_id: String,
}

/// A cross-reference relationship between two concepts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossReference {
    pub source: String,
    pub target: String,
    pub relationship: String,
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn now_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn node_type_str(t: &KbNodeType) -> &'static str {
    match t {
        KbNodeType::Raw => "raw",
        KbNodeType::Compiled => "compiled",
        KbNodeType::Concept => "concept",
        KbNodeType::Answer => "answer",
        KbNodeType::Index => "index",
    }
}

fn edge_type_str(t: &KbEdgeType) -> &'static str {
    match t {
        KbEdgeType::Provenance => "Provenance",
        KbEdgeType::Backlink => "Backlink",
        KbEdgeType::CrossReference => "CrossReference",
        KbEdgeType::ConceptOf => "ConceptOf",
        KbEdgeType::DerivedFrom => "DerivedFrom",
    }
}

// ---------------------------------------------------------------------------
// Compiler
// ---------------------------------------------------------------------------

/// LLM-powered knowledge base compiler.
///
/// Reads raw nodes from the CozoDB knowledge base, generates summaries and
/// extracts concepts using the supplied `KbLlmClient`, then writes compiled
/// nodes, concept nodes, provenance edges, and an index node back to the DB.
pub struct Compiler {
    db: DbInstance,
    llm: Box<dyn KbLlmClient>,
}

impl Compiler {
    /// Create a new `Compiler`.
    ///
    /// Ensures the `compile_state` relation exists (idempotent).
    pub fn new(db: DbInstance, llm: Box<dyn KbLlmClient>) -> Result<Self> {
        Self::ensure_compile_schema(&db)?;
        Ok(Self { db, llm })
    }

    /// Create the `compile_state` relation used to track incremental state.
    fn ensure_compile_schema(db: &DbInstance) -> Result<()> {
        let script = ":create compile_state { key: String => value: Float }";
        match db.run_script(script, Default::default(), ScriptMutability::Mutable) {
            Ok(_) => {}
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("already exists") || msg.contains("conflicts") {
                    // Idempotent — relation already present
                } else {
                    return Err(anyhow::anyhow!("compile_state schema creation failed: {}", msg));
                }
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // State helpers
    // -----------------------------------------------------------------------

    fn get_last_compiled_at(&self) -> Result<f64> {
        let result = self.db.run_script(
            "?[value] := *compile_state{key, value}, key = 'last_compiled_at'",
            Default::default(),
            ScriptMutability::Immutable,
        ).map_err(|e| anyhow::anyhow!("read compile_state failed: {}", e))?;

        Ok(result.rows.first().and_then(|r| r[0].get_float()).unwrap_or(0.0))
    }

    fn set_last_compiled_at(&self, ts: f64) -> Result<()> {
        let mut params = BTreeMap::new();
        params.insert("ts".to_string(), DataValue::from(ts));
        self.db.run_script(
            "?[key, value] <- [['last_compiled_at', $ts]] \
             :put compile_state { key => value }",
            params,
            ScriptMutability::Mutable,
        ).map_err(|e| anyhow::anyhow!("write compile_state failed: {}", e))?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // DB helpers
    // -----------------------------------------------------------------------

    fn insert_kb_node(
        &self,
        node_id: &str,
        node_type: &KbNodeType,
        title: &str,
        content: &str,
        source: &str,
        domain_tag: &str,
    ) -> Result<()> {
        let ts = now_f64();
        let mut params = BTreeMap::new();
        params.insert("nid".to_string(), DataValue::from(node_id));
        params.insert("ntype".to_string(), DataValue::from(node_type_str(node_type)));
        params.insert("title".to_string(), DataValue::from(title));
        params.insert("content".to_string(), DataValue::from(content));
        params.insert("source".to_string(), DataValue::from(source));
        params.insert("dtag".to_string(), DataValue::from(domain_tag));
        params.insert("ts".to_string(), DataValue::from(ts));

        self.db.run_script(
            "?[node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at] \
             <- [[$nid, $ntype, $source, $dtag, $title, $content, '', 0, $ts, $ts]] \
             :put kb_nodes { node_id => node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at }",
            params,
            ScriptMutability::Mutable,
        ).map_err(|e| anyhow::anyhow!("insert kb_node failed: {}", e))?;
        Ok(())
    }

    fn insert_kb_edge(
        &self,
        source_node_id: &str,
        target_node_id: &str,
        edge_type: &KbEdgeType,
    ) -> Result<()> {
        let edge_id = uuid::Uuid::new_v4().to_string();
        let ts = now_f64();
        let mut params = BTreeMap::new();
        params.insert("eid".to_string(), DataValue::from(edge_id.as_str()));
        params.insert("src".to_string(), DataValue::from(source_node_id));
        params.insert("tgt".to_string(), DataValue::from(target_node_id));
        params.insert("etype".to_string(), DataValue::from(edge_type_str(edge_type)));
        params.insert("ts".to_string(), DataValue::from(ts));

        self.db.run_script(
            "?[edge_id, source_node_id, target_node_id, edge_type, created_at] \
             <- [[$eid, $src, $tgt, $etype, $ts]] \
             :put kb_edges { edge_id => source_node_id, target_node_id, edge_type, created_at }",
            params,
            ScriptMutability::Mutable,
        ).map_err(|e| anyhow::anyhow!("insert kb_edge failed: {}", e))?;
        Ok(())
    }

    fn query_raw_nodes_after(&self, since: f64) -> Result<Vec<KbNode>> {
        let mut params = BTreeMap::new();
        params.insert("since".to_string(), DataValue::from(since));

        let result = self.db.run_script(
            "?[node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at] :=
             *kb_nodes{node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at},
             node_type = 'raw',
             created_at > $since
             :order created_at",
            params,
            ScriptMutability::Immutable,
        ).map_err(|e| anyhow::anyhow!("query raw nodes failed: {}", e))?;

        Ok(result.rows.iter().map(|r| row_to_kb_node(r)).collect())
    }

    fn query_all_nodes(&self) -> Result<Vec<KbNode>> {
        let result = self.db.run_script(
            "?[node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at] :=
             *kb_nodes{node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at}
             :order created_at",
            Default::default(),
            ScriptMutability::Immutable,
        ).map_err(|e| anyhow::anyhow!("query all nodes failed: {}", e))?;

        Ok(result.rows.iter().map(|r| row_to_kb_node(r)).collect())
    }

    // -----------------------------------------------------------------------
    // Public compile API
    // -----------------------------------------------------------------------

    /// Main compile entry point.
    ///
    /// 1. Reads `last_compiled_at` from `compile_state`.
    /// 2. Queries raw nodes created after that timestamp.
    /// 3. For each raw node: generates a summary and creates a compiled node
    ///    with a provenance edge back to the source.
    /// 4. Extracts concepts across the batch.
    /// 5. Builds cross-references between discovered concepts.
    /// 6. Creates or updates the single index node.
    /// 7. Records the current timestamp as `last_compiled_at`.
    pub async fn compile(&self) -> Result<CompileMetrics> {
        let start = Instant::now();
        let last_compiled_at = self.get_last_compiled_at()?;

        let raw_nodes = self.query_raw_nodes_after(last_compiled_at)?;
        if raw_nodes.is_empty() {
            return Ok(CompileMetrics {
                duration_secs: start.elapsed().as_secs_f64(),
                ..Default::default()
            });
        }

        info!("Compiling {} raw node(s)", raw_nodes.len());
        let compile_ts = now_f64();

        let mut total_edges = 0usize;
        let mut summaries_generated = 0usize;
        let mut compiled_nodes: Vec<KbNode> = Vec::new();

        for raw_node in &raw_nodes {
            match self.compile_document(raw_node).await {
                Ok(doc) => {
                    summaries_generated += 1;
                    total_edges += 1; // one provenance edge per compiled node

                    // Look up the compiled node we just inserted for concept extraction
                    compiled_nodes.push(KbNode {
                        node_id: doc.node_id.clone(),
                        node_type: KbNodeType::Compiled,
                        source: raw_node.source.clone(),
                        domain_tag: raw_node.domain_tag.clone(),
                        title: raw_node.title.clone(),
                        content: doc.summary,
                        content_hash: String::new(),
                        chunk_index: 0,
                        created_at: compile_ts,
                        updated_at: compile_ts,
                    });
                }
                Err(e) => {
                    warn!("compile_document failed for node {}: {}", raw_node.node_id, e);
                }
            }
        }

        // Extract concepts from all compiled nodes
        let concepts = self.extract_concepts(&compiled_nodes).await.unwrap_or_else(|e| {
            warn!("extract_concepts failed: {}", e);
            vec![]
        });
        let concepts_extracted = concepts.len();
        total_edges += concepts_extracted; // each concept gets a ConceptOf edge (at minimum)

        // Build concept name → node_id map for cross-referencing.
        // Uses the node_ids populated by extract_concepts() — NOT fresh UUIDs.
        let concept_ids: Vec<(String, String)> = concepts
            .iter()
            .map(|c| (c.node_id.clone(), c.name.clone()))
            .collect();

        // Build cross-references (errors here are non-fatal)
        let xref_count = self.build_cross_references(&concept_ids).await.unwrap_or_else(|e| {
            warn!("build_cross_references failed: {}", e);
            0
        });
        total_edges += xref_count;

        // Update the index node
        let index_updated = self.update_index_node().await.is_ok();

        // Record last_compiled_at
        self.set_last_compiled_at(compile_ts)?;

        let duration_secs = start.elapsed().as_secs_f64();
        info!(
            "Compile complete: {} summaries, {} concepts, {} edges in {:.2}s",
            summaries_generated, concepts_extracted, total_edges, duration_secs
        );

        Ok(CompileMetrics {
            summaries_generated,
            concepts_extracted,
            edges_created: total_edges,
            index_updated,
            duration_secs,
        })
    }

    /// Compile a single document: generate a summary via the LLM.
    ///
    /// Stores the compiled node and a provenance edge in CozoDB.
    /// If the LLM response is not valid JSON the raw text is used as the
    /// summary (logged at WARN level, not fatal).
    pub async fn compile_document(&self, node: &KbNode) -> Result<DocumentCompilation> {
        let prompt = format!(
            "Summarize the following document in 100-200 words. Return JSON: {{\"summary\": \"...\"}}\n\nDocument title: {}\n\nContent:\n{}",
            node.title, node.content
        );

        let response = self.llm.complete(&prompt).await?;

        let summary = match serde_json::from_str::<serde_json::Value>(&response) {
            Ok(val) => val["summary"].as_str().unwrap_or(&response).to_string(),
            Err(_) => {
                warn!(
                    "Failed to parse LLM summary as JSON for node '{}', using raw response",
                    node.node_id
                );
                response
            }
        };

        let compiled_id = uuid::Uuid::new_v4().to_string();

        // Store compiled node
        self.insert_kb_node(
            &compiled_id,
            &KbNodeType::Compiled,
            &format!("Summary: {}", node.title),
            &summary,
            &node.source,
            &node.domain_tag,
        )?;

        // Provenance edge: compiled_id → raw node_id
        self.insert_kb_edge(&compiled_id, &node.node_id, &KbEdgeType::Provenance)?;

        Ok(DocumentCompilation {
            node_id: compiled_id,
            summary,
            concepts: vec![],
        })
    }

    /// Extract key concepts across a batch of compiled (or raw) nodes.
    ///
    /// Sends a batch prompt to the LLM and stores each concept as a
    /// `KbNodeType::Concept` node with `ConceptOf` edges to the source nodes.
    pub async fn extract_concepts(&self, nodes: &[KbNode]) -> Result<Vec<ConceptArticle>> {
        if nodes.is_empty() {
            return Ok(vec![]);
        }

        let docs: String = nodes
            .iter()
            .map(|n| {
                let preview = &n.content[..n.content.len().min(200)];
                format!("- {} (ID: {}): {}", n.title, n.node_id, preview)
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "Extract key concepts from these documents. For each concept, provide a name and 2-3 sentence explanation.\nReturn JSON array: [{{\"name\": \"...\", \"explanation\": \"...\", \"source_nodes\": [\"node_id\", ...]}}]\n\nDocuments:\n{}",
            docs
        );

        let response = self.llm.complete(&prompt).await?;

        match serde_json::from_str::<Vec<ConceptArticle>>(&response) {
            Ok(mut concepts) => {
                for concept in &mut concepts {
                    let concept_node_id = uuid::Uuid::new_v4().to_string();
                    concept.node_id = concept_node_id.clone();

                    self.insert_kb_node(
                        &concept_node_id,
                        &KbNodeType::Concept,
                        &concept.name,
                        &concept.explanation,
                        "compilation",
                        "concepts",
                    )?;

                    // ConceptOf edges to source nodes
                    for source_id in &concept.source_nodes {
                        if let Err(e) = self.insert_kb_edge(
                            &concept_node_id,
                            source_id,
                            &KbEdgeType::ConceptOf,
                        ) {
                            warn!("Failed to insert ConceptOf edge for concept '{}' → '{}': {}", concept.name, source_id, e);
                        }
                    }
                }
                Ok(concepts)
            }
            Err(e) => {
                warn!("Failed to parse concept extraction response: {}", e);
                Ok(vec![])
            }
        }
    }

    /// Build cross-reference edges between concepts.
    ///
    /// `concept_ids` is a slice of `(concept_node_id, concept_name)` pairs.
    /// Returns the number of cross-reference edges created.
    pub async fn build_cross_references(&self, concept_ids: &[(String, String)]) -> Result<usize> {
        if concept_ids.is_empty() {
            return Ok(0);
        }

        let names: String = concept_ids
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        let prompt = format!(
            "Given these concepts: [{}]. Identify relationships between them.\nReturn JSON array: [{{\"source\": \"name1\", \"target\": \"name2\", \"relationship\": \"description\"}}]",
            names
        );

        let response = self.llm.complete(&prompt).await?;

        match serde_json::from_str::<Vec<CrossReference>>(&response) {
            Ok(refs) => {
                // Build a name → node_id lookup
                let name_to_id: std::collections::HashMap<&str, &str> = concept_ids
                    .iter()
                    .map(|(id, name)| (name.as_str(), id.as_str()))
                    .collect();

                let mut count = 0;
                for xref in &refs {
                    let src_id = name_to_id.get(xref.source.as_str());
                    let tgt_id = name_to_id.get(xref.target.as_str());

                    if let (Some(src), Some(tgt)) = (src_id, tgt_id) {
                        if let Err(e) = self.insert_kb_edge(src, tgt, &KbEdgeType::CrossReference) {
                            warn!("Failed to insert CrossReference edge '{}' → '{}': {}", xref.source, xref.target, e);
                        } else {
                            count += 1;
                        }
                    }
                }
                Ok(count)
            }
            Err(e) => {
                warn!("Failed to parse cross-reference response: {}", e);
                Ok(0)
            }
        }
    }

    /// Create or update the single index node summarising the entire knowledge base.
    pub async fn update_index_node(&self) -> Result<()> {
        let nodes = self.query_all_nodes()?;

        let mut by_type: BTreeMap<&str, usize> = BTreeMap::new();
        for node in &nodes {
            *by_type.entry(node_type_str(&node.node_type)).or_insert(0) += 1;
        }

        let summary = by_type
            .iter()
            .map(|(t, c)| format!("{}: {}", t, c))
            .collect::<Vec<_>>()
            .join(", ");

        let index_content = format!(
            "Knowledge base index — {} total nodes. {}\nLast updated: {:.0}",
            nodes.len(),
            summary,
            now_f64()
        );

        // Use a fixed well-known ID so there is always exactly one index node
        self.insert_kb_node(
            "kb-index-singleton",
            &KbNodeType::Index,
            "Knowledge Base Index",
            &index_content,
            "system",
            "index",
        )?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Row conversion helper (mirrors mod.rs)
// ---------------------------------------------------------------------------

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
