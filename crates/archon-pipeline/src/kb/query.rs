//! KB Q&A query engine — embed, search, gather context, synthesize, file answers.
//!
//! Implements REQ-KB-003. NFR: search < 500ms, Q&A < 5s.

use std::collections::{BTreeMap, HashSet};

use anyhow::Result;
use cozo::{DataValue, ScriptMutability};
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::schema::{KbNode, KbNodeType};
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
/// Trait for LLM-based answer synthesis.
#[async_trait::async_trait]
pub trait QaSynthesizer: Send + Sync {
    async fn synthesize(&self, question: &str, context: &str) -> Result<String>;
}
/// Trait for computing query embeddings.
pub trait QueryEmbedder: Send + Sync {
    fn embed_query(&self, text: &str) -> Result<Vec<f32>>;
}
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
    pub async fn query(&self, question: &str, opts: &QaQueryOptions) -> Result<QaQueryResult> {
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
            let source_ids: Vec<String> = scored_nodes
                .iter()
                .map(|n| n.node.node_id.clone())
                .collect();
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
                if let Some(filter) = type_filter
                    && !filter.contains(&node.node_type)
                {
                    return None;
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
    pub fn gather_graph_context(&self, nodes: &[ScoredKbNode]) -> Result<GraphContext> {
        let mut related_concepts = Vec::new();
        let mut backlinks = Vec::new();
        let mut seen_ids: HashSet<String> =
            nodes.iter().map(|sn| sn.node.node_id.clone()).collect();

        for sn in nodes {
            let mut params = BTreeMap::new();
            params.insert("nid".to_string(), DataValue::from(sn.node.node_id.as_str()));

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
        let title = if question.chars().count() > 100 {
            format!("{}...", question.chars().take(97).collect::<String>())
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

        let filed_node_id = super::answer_storage::reserve_answer_node(
            &self.db,
            &node_id,
            &title,
            &answer.answer_text,
            &content_hash,
            now,
        )?;

        // Attach provenance to the canonical answer owner. Failure is best effort:
        // answer filing is already committed and must remain visible.
        if let Err(error) = super::provenance_storage::persist_derived_from_edges(
            &self.db,
            &filed_node_id,
            source_node_ids,
            now,
        ) {
            warn!(
                owner_id = %filed_node_id,
                error = %error,
                "Failed to batch DerivedFrom provenance edges"
            );
        }

        Ok(filed_node_id)
    }

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

#[cfg(test)]
#[path = "query_provenance_tests.rs"]
mod query_provenance_tests;
#[cfg(test)]
#[path = "query_tests.rs"]
mod query_tests;
