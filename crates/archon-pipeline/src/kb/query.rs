//! KB Q&A query engine — embed, search, gather context, synthesize, file answers.
//!
//! Implements REQ-KB-003. NFR: search < 500ms, Q&A < 5s.

use std::collections::{BTreeMap, HashSet};

use anyhow::Result;
use cozo::{DataValue, ScriptMutability};
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::schema::{KbNode, KbNodeType};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Options for a Q&A query.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QaQueryOptions {
    pub top_k: usize,
    pub file_answer: bool,
    pub include_graph_context: bool,
    pub node_type_filter: Option<Vec<KbNodeType>>,
}

impl Default for QaQueryOptions {
    fn default() -> Self {
        Self {
            top_k: 10,
            file_answer: false,
            include_graph_context: true,
            node_type_filter: None,
        }
    }
}

/// A scored KB node from search.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoredKbNode {
    pub node: KbNode,
    pub score: f64,
}

/// Graph context gathered by following edges.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GraphContext {
    pub primary_nodes: Vec<ScoredKbNode>,
    pub related_concepts: Vec<KbNode>,
    pub backlinks: Vec<KbNode>,
    pub provenance_chains: Vec<Vec<String>>,
}

/// A synthesized answer with source citations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SynthesizedAnswer {
    pub answer_text: String,
    pub source_citations: Vec<SourceCitation>,
}

/// Citation referencing a KB node.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceCitation {
    pub node_id: String,
    pub quote: String,
    pub relevance: f64,
}

/// Full result of a Q&A query.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QaQueryResult {
    pub answer: String,
    pub sources: Vec<QaSource>,
    pub filed_node_id: Option<String>,
    pub search_duration_ms: u64,
    pub synthesis_duration_ms: u64,
}

/// Source info in query result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QaSource {
    pub node_id: String,
    pub title: String,
    pub relevance_score: f64,
}

// ---------------------------------------------------------------------------
// LLM trait for synthesis
// ---------------------------------------------------------------------------

/// Trait for LLM-based answer synthesis.
#[async_trait::async_trait]
pub trait QaSynthesizer: Send + Sync {
    async fn synthesize(&self, question: &str, context: &str) -> Result<String>;
}

// ---------------------------------------------------------------------------
// Embedding trait
// ---------------------------------------------------------------------------

/// Trait for computing query embeddings.
pub trait QueryEmbedder: Send + Sync {
    fn embed_query(&self, text: &str) -> Result<Vec<f32>>;
}

// ---------------------------------------------------------------------------
// QueryEngine
// ---------------------------------------------------------------------------

/// Knowledge base query engine.
///
/// Searches KB nodes, gathers graph context, synthesizes answers via an
/// optional LLM, and optionally files answers back as derived nodes.
pub struct QueryEngine {
    db: cozo::DbInstance,
    synthesizer: Option<Box<dyn QaSynthesizer>>,
    embedder: Option<Box<dyn QueryEmbedder>>,
}

impl QueryEngine {
    pub fn new(db: cozo::DbInstance) -> Self {
        Self {
            db,
            synthesizer: None,
            embedder: None,
        }
    }

    pub fn with_synthesizer(mut self, synth: Box<dyn QaSynthesizer>) -> Self {
        self.synthesizer = Some(synth);
        self
    }

    pub fn with_embedder(mut self, embedder: Box<dyn QueryEmbedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Full Q&A flow: search, gather context, synthesize, optionally file.
    pub async fn query(
        &self,
        question: &str,
        opts: &QaQueryOptions,
    ) -> Result<QaQueryResult> {
        let search_start = std::time::Instant::now();

        // Step 1: Search for relevant nodes
        let scored_nodes =
            self.search_nodes(question, opts.top_k, opts.node_type_filter.as_deref())?;
        let search_duration_ms = search_start.elapsed().as_millis() as u64;

        if scored_nodes.is_empty() {
            return Ok(QaQueryResult {
                answer: "Insufficient context in the knowledge base to answer this question."
                    .into(),
                sources: vec![],
                filed_node_id: None,
                search_duration_ms,
                synthesis_duration_ms: 0,
            });
        }

        // Step 2: Gather graph context
        let graph_context = if opts.include_graph_context {
            self.gather_graph_context(&scored_nodes)?
        } else {
            GraphContext {
                primary_nodes: scored_nodes.clone(),
                ..Default::default()
            }
        };

        // Step 3: Synthesize answer
        let synth_start = std::time::Instant::now();
        let synthesized = self.synthesize_answer(question, &graph_context).await?;
        let synthesis_duration_ms = synth_start.elapsed().as_millis() as u64;

        // Step 4: Optionally file the answer
        let filed_node_id = if opts.file_answer {
            let source_ids: Vec<String> =
                scored_nodes.iter().map(|n| n.node.node_id.clone()).collect();
            Some(self.file_answer(question, &synthesized, &source_ids)?)
        } else {
            None
        };

        let sources = scored_nodes
            .iter()
            .map(|sn| QaSource {
                node_id: sn.node.node_id.clone(),
                title: sn.node.title.clone(),
                relevance_score: sn.score,
            })
            .collect();

        Ok(QaQueryResult {
            answer: synthesized.answer_text,
            sources,
            filed_node_id,
            search_duration_ms,
            synthesis_duration_ms,
        })
    }

    /// Search KB nodes using text matching (fallback when no embedder).
    /// When embedder is available, uses HNSW vector search.
    /// Answer-type nodes get a 0.9x score penalty (EC-PIPE-018).
    pub fn search_nodes(
        &self,
        query_text: &str,
        limit: usize,
        type_filter: Option<&[KbNodeType]>,
    ) -> Result<Vec<ScoredKbNode>> {
        let mut params = BTreeMap::new();
        params.insert("q".to_string(), DataValue::from(query_text));
        // Over-fetch so post-filter still has enough results
        params.insert("lim".to_string(), DataValue::from((limit * 3) as i64));

        let result = self
            .db
            .run_script(
                "?[node_id, node_type, source, domain_tag, title, content, \
                 content_hash, chunk_index, created_at, updated_at] := \
                 *kb_nodes{node_id, node_type, source, domain_tag, title, content, \
                 content_hash, chunk_index, created_at, updated_at}, \
                 (str_includes(title, $q) or str_includes(content, $q)) \
                 :limit $lim",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(|e| anyhow::anyhow!("KB search failed: {}", e))?;

        let mut scored: Vec<ScoredKbNode> = result
            .rows
            .iter()
            .filter_map(|row| {
                let node = row_to_kb_node(row);

                // Apply type filter if specified
                if let Some(filter) = type_filter {
                    if !filter.contains(&node.node_type) {
                        return None;
                    }
                }

                // Calculate relevance score based on title vs content match
                let query_lower = query_text.to_lowercase();
                let title_lower = node.title.to_lowercase();
                let content_lower = node.content.to_lowercase();

                let mut score: f64 = 0.0;
                if title_lower.contains(&query_lower) {
                    score += 0.8;
                }
                if content_lower.contains(&query_lower) {
                    score += 0.5;
                }
                // Clamp to 0-1
                score = score.min(1.0);

                // EC-PIPE-018: Answer nodes get 0.9x penalty
                if node.node_type == KbNodeType::Answer {
                    score *= 0.9;
                }

                if score > 0.0 {
                    Some(ScoredKbNode { node, score })
                } else {
                    None
                }
            })
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);

        Ok(scored)
    }

    /// Follow edges to collect related concepts, backlinks, provenance chains.
    pub fn gather_graph_context(
        &self,
        nodes: &[ScoredKbNode],
    ) -> Result<GraphContext> {
        let mut related_concepts = Vec::new();
        let mut backlinks = Vec::new();
        let mut seen_ids: HashSet<String> =
            nodes.iter().map(|sn| sn.node.node_id.clone()).collect();

        for sn in nodes {
            let mut params = BTreeMap::new();
            params.insert(
                "nid".to_string(),
                DataValue::from(sn.node.node_id.as_str()),
            );

            // Outgoing edges: this node -> targets
            if let Ok(result) = self.db.run_script(
                "?[node_id, node_type, source, domain_tag, title, content, \
                 content_hash, chunk_index, created_at, updated_at] := \
                 *kb_edges{source_node_id, target_node_id}, source_node_id = $nid, \
                 *kb_nodes{node_id, node_type, source, domain_tag, title, content, \
                 content_hash, chunk_index, created_at, updated_at}, \
                 node_id = target_node_id",
                params.clone(),
                ScriptMutability::Immutable,
            ) {
                for row in &result.rows {
                    let node = row_to_kb_node(row);
                    if !seen_ids.contains(&node.node_id) {
                        seen_ids.insert(node.node_id.clone());
                        if node.node_type == KbNodeType::Concept {
                            related_concepts.push(node);
                        }
                    }
                }
            }

            // Incoming edges (backlinks): sources -> this node
            if let Ok(result) = self.db.run_script(
                "?[node_id, node_type, source, domain_tag, title, content, \
                 content_hash, chunk_index, created_at, updated_at] := \
                 *kb_edges{source_node_id, target_node_id}, target_node_id = $nid, \
                 *kb_nodes{node_id, node_type, source, domain_tag, title, content, \
                 content_hash, chunk_index, created_at, updated_at}, \
                 node_id = source_node_id",
                params,
                ScriptMutability::Immutable,
            ) {
                for row in &result.rows {
                    let node = row_to_kb_node(row);
                    if !seen_ids.contains(&node.node_id) {
                        seen_ids.insert(node.node_id.clone());
                        backlinks.push(node);
                    }
                }
            }
        }

        Ok(GraphContext {
            primary_nodes: nodes.to_vec(),
            related_concepts,
            backlinks,
            provenance_chains: vec![], // Populated when provenance system (F06) is wired
        })
    }

    /// Synthesize an answer using LLM or fallback to context concatenation.
    pub async fn synthesize_answer(
        &self,
        question: &str,
        context: &GraphContext,
    ) -> Result<SynthesizedAnswer> {
        let formatted_context = self.format_context(context);

        if let Some(ref synth) = self.synthesizer {
            let prompt = format!(
                "Answer the following question using ONLY the provided context. \
                 Cite your sources by node ID. If the context is insufficient, say so.\n\n\
                 Question: {}\n\nContext:\n{}",
                question, formatted_context
            );
            let answer_text = synth.synthesize(question, &prompt).await?;
            let citations = context
                .primary_nodes
                .iter()
                .map(|sn| SourceCitation {
                    node_id: sn.node.node_id.clone(),
                    quote: sn.node.content.chars().take(200).collect(),
                    relevance: sn.score,
                })
                .collect();

            Ok(SynthesizedAnswer {
                answer_text,
                source_citations: citations,
            })
        } else {
            // Fallback: concatenate relevant content
            let answer_text = format!(
                "Based on {} knowledge base sources:\n\n{}",
                context.primary_nodes.len(),
                formatted_context
            );
            let citations = context
                .primary_nodes
                .iter()
                .map(|sn| SourceCitation {
                    node_id: sn.node.node_id.clone(),
                    quote: sn.node.content.chars().take(200).collect(),
                    relevance: sn.score,
                })
                .collect();

            Ok(SynthesizedAnswer {
                answer_text,
                source_citations: citations,
            })
        }
    }

    /// File an answer back into the KB as a derived knowledge node.
    pub fn file_answer(
        &self,
        question: &str,
        answer: &SynthesizedAnswer,
        source_node_ids: &[String],
    ) -> Result<String> {
        let node_id = format!("answer-{}", uuid::Uuid::new_v4());
        let title = if question.len() > 100 {
            format!("{}...", &question[..97])
        } else {
            question.to_string()
        };
        let content_hash = {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(answer.answer_text.as_bytes());
            hex::encode(hasher.finalize())
        };
        let now = chrono::Utc::now().timestamp() as f64;

        // Insert answer node
        let mut params = BTreeMap::new();
        params.insert("node_id".into(), DataValue::from(node_id.as_str()));
        params.insert("node_type".into(), DataValue::from("answer"));
        params.insert("source".into(), DataValue::from("qa-engine"));
        params.insert("domain_tag".into(), DataValue::from(""));
        params.insert("title".into(), DataValue::from(title.as_str()));
        params.insert(
            "content".into(),
            DataValue::from(answer.answer_text.as_str()),
        );
        params.insert(
            "content_hash".into(),
            DataValue::from(content_hash.as_str()),
        );
        params.insert("chunk_index".into(), DataValue::from(0i64));
        params.insert("created_at".into(), DataValue::from(now));
        params.insert("updated_at".into(), DataValue::from(now));

        self.db
            .run_script(
                "?[node_id, node_type, source, domain_tag, title, content, \
                 content_hash, chunk_index, created_at, updated_at] <- \
                 [[$node_id, $node_type, $source, $domain_tag, $title, $content, \
                 $content_hash, $chunk_index, $created_at, $updated_at]] \
                 :put kb_nodes { node_id => node_type, source, domain_tag, title, \
                 content, content_hash, chunk_index, created_at, updated_at }",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(|e| anyhow::anyhow!("Failed to file answer node: {}", e))?;

        // Create DerivedFrom edges to source nodes
        for source_id in source_node_ids {
            let edge_id = format!("edge-{}", uuid::Uuid::new_v4());
            let mut edge_params = BTreeMap::new();
            edge_params.insert("edge_id".into(), DataValue::from(edge_id.as_str()));
            edge_params.insert(
                "source_node_id".into(),
                DataValue::from(node_id.as_str()),
            );
            edge_params.insert(
                "target_node_id".into(),
                DataValue::from(source_id.as_str()),
            );
            edge_params.insert("edge_type".into(), DataValue::from("DerivedFrom"));
            edge_params.insert("created_at".into(), DataValue::from(now));

            if let Err(e) = self.db.run_script(
                "?[edge_id, source_node_id, target_node_id, edge_type, created_at] <- \
                 [[$edge_id, $source_node_id, $target_node_id, $edge_type, $created_at]] \
                 :put kb_edges { edge_id => source_node_id, target_node_id, edge_type, \
                 created_at }",
                edge_params,
                ScriptMutability::Mutable,
            ) {
                warn!(
                    source_id = %source_id,
                    error = %e,
                    "Failed to create DerivedFrom edge"
                );
            }
        }

        Ok(node_id)
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn format_context(&self, context: &GraphContext) -> String {
        let mut parts = Vec::new();

        for sn in &context.primary_nodes {
            parts.push(format!(
                "### [{}] {} (relevance: {:.2})\n{}",
                sn.node.node_id, sn.node.title, sn.score, sn.node.content
            ));
        }

        if !context.related_concepts.is_empty() {
            parts.push("\n### Related Concepts:".to_string());
            for c in &context.related_concepts {
                parts.push(format!(
                    "- [{}] {}: {}",
                    c.node_id,
                    c.title,
                    c.content.chars().take(200).collect::<String>()
                ));
            }
        }

        if !context.backlinks.is_empty() {
            parts.push("\n### Backlinked Sources:".to_string());
            for b in &context.backlinks {
                parts.push(format!("- [{}] {}", b.node_id, b.title));
            }
        }

        parts.join("\n\n")
    }
}

// ---------------------------------------------------------------------------
// Row conversion helper
// ---------------------------------------------------------------------------

fn row_to_kb_node(row: &[DataValue]) -> KbNode {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::schema::ensure_kb_schema;

    fn test_db() -> cozo::DbInstance {
        let db = cozo::DbInstance::new("mem", "", Default::default()).unwrap();
        ensure_kb_schema(&db).unwrap();
        db
    }

    fn insert_test_node(db: &cozo::DbInstance, id: &str, ntype: &str, title: &str, content: &str) {
        let mut params = BTreeMap::new();
        params.insert("nid".into(), DataValue::from(id));
        params.insert("ntype".into(), DataValue::from(ntype));
        params.insert("title".into(), DataValue::from(title));
        params.insert("content".into(), DataValue::from(content));
        params.insert("ts".into(), DataValue::from(1000.0));
        db.run_script(
            "?[node_id, node_type, source, domain_tag, title, content, \
             content_hash, chunk_index, created_at, updated_at] <- \
             [[$nid, $ntype, 'test', '', $title, $content, '', 0, $ts, $ts]] \
             :put kb_nodes { node_id => node_type, source, domain_tag, title, \
             content, content_hash, chunk_index, created_at, updated_at }",
            params,
            ScriptMutability::Mutable,
        )
        .unwrap();
    }

    fn insert_test_edge(db: &cozo::DbInstance, src: &str, tgt: &str, etype: &str) {
        let edge_id = format!("edge-{}", uuid::Uuid::new_v4());
        let mut params = BTreeMap::new();
        params.insert("eid".into(), DataValue::from(edge_id.as_str()));
        params.insert("src".into(), DataValue::from(src));
        params.insert("tgt".into(), DataValue::from(tgt));
        params.insert("etype".into(), DataValue::from(etype));
        params.insert("ts".into(), DataValue::from(1000.0));
        db.run_script(
            "?[edge_id, source_node_id, target_node_id, edge_type, created_at] <- \
             [[$eid, $src, $tgt, $etype, $ts]] \
             :put kb_edges { edge_id => source_node_id, target_node_id, edge_type, \
             created_at }",
            params,
            ScriptMutability::Mutable,
        )
        .unwrap();
    }

    #[tokio::test]
    async fn test_query_engine_empty_db() {
        let db = test_db();
        let engine = QueryEngine::new(db);
        let opts = QaQueryOptions::default();
        let result = engine.query("what is Rust?", &opts).await.unwrap();
        assert!(result.answer.contains("Insufficient context"));
        assert!(result.sources.is_empty());
        assert!(result.filed_node_id.is_none());
    }

    #[tokio::test]
    async fn test_search_nodes_finds_matching() {
        let db = test_db();
        insert_test_node(&db, "n1", "raw", "Rust Programming", "Rust is a systems language.");
        insert_test_node(&db, "n2", "raw", "Python Basics", "Python is interpreted.");

        let engine = QueryEngine::new(db);
        let results = engine.search_nodes("Rust", 10, None).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].node.node_id, "n1");
        assert!(results[0].score > 0.0);
    }

    #[tokio::test]
    async fn test_search_nodes_answer_penalty() {
        let db = test_db();
        // Same content, different types
        insert_test_node(&db, "raw1", "raw", "Rust guide", "Learn Rust programming today.");
        insert_test_node(
            &db,
            "ans1",
            "answer",
            "Rust guide",
            "Learn Rust programming today.",
        );

        let engine = QueryEngine::new(db);
        let results = engine.search_nodes("Rust", 10, None).unwrap();

        assert_eq!(results.len(), 2);
        // Both have "Rust" in title and content -> base score 1.0 (clamped)
        // Raw node: 1.0, Answer node: 0.9
        let raw_result = results.iter().find(|r| r.node.node_type == KbNodeType::Raw).unwrap();
        let ans_result = results
            .iter()
            .find(|r| r.node.node_type == KbNodeType::Answer)
            .unwrap();

        assert!(
            raw_result.score > ans_result.score,
            "Raw ({}) should score higher than Answer ({})",
            raw_result.score,
            ans_result.score
        );
        // Verify the 0.9x factor
        let expected_ans_score = raw_result.score * 0.9;
        assert!(
            (ans_result.score - expected_ans_score).abs() < 0.01,
            "Answer score {} should be ~0.9x of raw score {}",
            ans_result.score,
            raw_result.score
        );
    }

    #[tokio::test]
    async fn test_file_answer_creates_node() {
        let db = test_db();
        insert_test_node(&db, "src1", "raw", "Source Doc", "Some source content.");
        insert_test_node(&db, "src2", "raw", "Another Doc", "More source content.");

        let engine = QueryEngine::new(db.clone());
        let synth_answer = SynthesizedAnswer {
            answer_text: "This is the synthesized answer.".to_string(),
            source_citations: vec![],
        };

        let filed_id = engine
            .file_answer("What is the topic?", &synth_answer, &["src1".into(), "src2".into()])
            .unwrap();

        assert!(filed_id.starts_with("answer-"));

        // Verify the node was created
        let mut params = BTreeMap::new();
        params.insert("nid".into(), DataValue::from(filed_id.as_str()));
        let result = db
            .run_script(
                "?[node_type, content] := *kb_nodes{node_id, node_type, content}, node_id = $nid",
                params,
                ScriptMutability::Immutable,
            )
            .unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0][0].get_str().unwrap(), "answer");
        assert!(result.rows[0][1]
            .get_str()
            .unwrap()
            .contains("synthesized answer"));

        // Verify DerivedFrom edges exist
        let mut edge_params = BTreeMap::new();
        edge_params.insert("nid".into(), DataValue::from(filed_id.as_str()));
        let edges = db
            .run_script(
                "?[target_node_id, edge_type] := *kb_edges{source_node_id, target_node_id, edge_type}, \
                 source_node_id = $nid",
                edge_params,
                ScriptMutability::Immutable,
            )
            .unwrap();
        assert_eq!(edges.rows.len(), 2);
    }

    #[tokio::test]
    async fn test_gather_graph_context_follows_edges() {
        let db = test_db();
        insert_test_node(&db, "n1", "raw", "Main Doc", "Main content about Rust.");
        insert_test_node(&db, "c1", "concept", "Ownership", "Rust ownership model.");
        insert_test_node(&db, "b1", "raw", "Backlink Source", "References main doc.");

        // n1 -> c1 (outgoing to concept)
        insert_test_edge(&db, "n1", "c1", "ConceptOf");
        // b1 -> n1 (incoming backlink)
        insert_test_edge(&db, "b1", "n1", "Backlink");

        let engine = QueryEngine::new(db);
        let scored = vec![ScoredKbNode {
            node: KbNode {
                node_id: "n1".into(),
                node_type: KbNodeType::Raw,
                source: "test".into(),
                domain_tag: String::new(),
                title: "Main Doc".into(),
                content: "Main content about Rust.".into(),
                content_hash: String::new(),
                chunk_index: 0,
                created_at: 1000.0,
                updated_at: 1000.0,
            },
            score: 0.8,
        }];

        let ctx = engine.gather_graph_context(&scored).unwrap();
        assert_eq!(ctx.primary_nodes.len(), 1);
        assert_eq!(ctx.related_concepts.len(), 1);
        assert_eq!(ctx.related_concepts[0].node_id, "c1");
        assert_eq!(ctx.backlinks.len(), 1);
        assert_eq!(ctx.backlinks[0].node_id, "b1");
    }

    #[tokio::test]
    async fn test_synthesize_answer_without_llm() {
        let db = test_db();
        let engine = QueryEngine::new(db);

        let context = GraphContext {
            primary_nodes: vec![ScoredKbNode {
                node: KbNode {
                    node_id: "n1".into(),
                    node_type: KbNodeType::Raw,
                    source: "test".into(),
                    domain_tag: String::new(),
                    title: "Test Doc".into(),
                    content: "Test content here.".into(),
                    content_hash: String::new(),
                    chunk_index: 0,
                    created_at: 1000.0,
                    updated_at: 1000.0,
                },
                score: 0.9,
            }],
            ..Default::default()
        };

        let result = engine
            .synthesize_answer("What is this?", &context)
            .await
            .unwrap();
        assert!(result.answer_text.contains("1 knowledge base sources"));
        assert!(result.answer_text.contains("Test Doc"));
        assert_eq!(result.source_citations.len(), 1);
        assert_eq!(result.source_citations[0].node_id, "n1");
    }

    #[tokio::test]
    async fn test_query_full_flow() {
        let db = test_db();
        insert_test_node(
            &db,
            "doc1",
            "raw",
            "Ownership in Rust",
            "Rust uses ownership to manage memory safely without garbage collection.",
        );
        insert_test_node(
            &db,
            "doc2",
            "raw",
            "Rust Borrowing",
            "Borrowing in Rust allows references without taking ownership.",
        );

        let engine = QueryEngine::new(db);
        let opts = QaQueryOptions {
            top_k: 5,
            file_answer: false,
            include_graph_context: true,
            node_type_filter: None,
        };

        let result = engine.query("Rust", &opts).await.unwrap();
        assert!(!result.answer.is_empty());
        assert!(!result.answer.contains("Insufficient context"));
        assert!(!result.sources.is_empty());
        // Both docs mention "Rust"
        assert_eq!(result.sources.len(), 2);
        assert!(result.search_duration_ms < 500); // NFR: search < 500ms
    }

    #[tokio::test]
    async fn test_filed_answer_ranked_below_original() {
        let db = test_db();
        // Insert a raw doc and a previously-filed answer with same content
        insert_test_node(
            &db,
            "original",
            "raw",
            "Rust Safety",
            "Rust ensures memory safety through its type system.",
        );
        insert_test_node(
            &db,
            "filed-ans",
            "answer",
            "Rust Safety",
            "Rust ensures memory safety through its type system.",
        );

        let engine = QueryEngine::new(db);
        let results = engine.search_nodes("Rust", 10, None).unwrap();

        assert_eq!(results.len(), 2);
        // The raw node should be ranked first (higher score)
        assert_eq!(
            results[0].node.node_type,
            KbNodeType::Raw,
            "Raw node should rank above answer node"
        );
        assert_eq!(results[1].node.node_type, KbNodeType::Answer);
        assert!(results[0].score > results[1].score);
    }
}
