//! Native memory operations for pipelines.
//!
//! This module wraps [`archon_memory::MemoryGraph`] for pipeline use. Agents
//! never call MCP — the runner calls these functions directly.

use anyhow::Result;
use chrono::{DateTime, Utc};

use archon_memory::{MemoryGraph, MemoryType, RelType, SearchFilter};

use crate::runner::PipelineType;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Newtype for memory IDs returned by store operations.
pub struct MemoryId(pub String);

/// Metadata about an agent's output being stored.
pub struct AgentOutputMetadata {
    pub pipeline_type: PipelineType,
    pub phase: u32,
    pub session_id: String,
    pub timestamp: DateTime<Utc>,
    pub quality_score: f64,
    pub tags: Vec<String>,
}

/// A memory entry returned by recall/search operations.
pub struct MemoryEntry {
    pub id: String,
    pub content: String,
    pub source_agent: String,
    pub timestamp: DateTime<Utc>,
    pub relevance_score: f64,
    pub tags: Vec<String>,
}

/// Filters for searching pipeline memories.
pub struct MemoryFilters {
    pub pipeline_type: Option<PipelineType>,
    pub phase: Option<u32>,
    pub agent_keys: Vec<String>,
    pub tags: Vec<String>,
    pub time_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a pipeline type to its string tag.
fn pipeline_type_tag(pt: &PipelineType) -> &'static str {
    match pt {
        PipelineType::Coding => "coding",
        PipelineType::Research => "research",
        PipelineType::Learning => "learning",
        PipelineType::Kb => "kb",
    }
}

/// Well-known pipeline type tags used to exclude them when inferring the source
/// agent from a memory's tag list.
const PIPELINE_TYPE_TAGS: &[&str] = &["coding", "research", "learning", "kb"];

/// Attempt to extract the source agent key from a memory's tags.
///
/// The agent key is the first tag that is neither `"pipeline"` nor a known
/// pipeline-type tag.
fn extract_source_agent(tags: &[String]) -> String {
    tags.iter()
        .find(|t| *t != "pipeline" && !PIPELINE_TYPE_TAGS.contains(&t.as_str()))
        .cloned()
        .unwrap_or_default()
}

/// Convert an `archon_memory::Memory` into a pipeline `MemoryEntry`.
fn memory_to_entry(m: archon_memory::Memory) -> MemoryEntry {
    let source_agent = extract_source_agent(&m.tags);
    MemoryEntry {
        id: m.id,
        content: m.content,
        source_agent,
        timestamp: m.created_at,
        relevance_score: m.importance,
        tags: m.tags,
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Store an agent's output in the memory graph.
///
/// Tags the memory with `["pipeline", "<pipeline_type>", "<agent_key>"]` plus
/// any extra tags from `metadata.tags`.
pub async fn store_agent_output(
    graph: &MemoryGraph,
    session_id: &str,
    agent_key: &str,
    output: &str,
    metadata: &AgentOutputMetadata,
) -> Result<MemoryId> {
    let pt_tag = pipeline_type_tag(&metadata.pipeline_type);

    let mut tags = vec![
        "pipeline".to_string(),
        pt_tag.to_string(),
        agent_key.to_string(),
    ];
    tags.extend(metadata.tags.iter().cloned());
    // Deduplicate while preserving order.
    let mut seen = std::collections::HashSet::new();
    tags.retain(|t| seen.insert(t.clone()));

    let title = format!("{agent_key} output");

    let id = graph.store_memory(
        output,
        &title,
        MemoryType::Fact,
        metadata.quality_score,
        &tags,
        "pipeline",
        session_id,
    )?;

    Ok(MemoryId(id))
}

/// Recall memories relevant to an agent's task.
///
/// Builds a query from the agent key and task description, then delegates to
/// [`MemoryGraph::recall_memories`].
pub async fn recall_for_agent(
    graph: &MemoryGraph,
    agent_key: &str,
    task: &str,
    limit: usize,
) -> Result<Vec<MemoryEntry>> {
    let query = format!("{agent_key}: {task}");
    let memories = graph.recall_memories(&query, limit)?;
    Ok(memories.into_iter().map(memory_to_entry).collect())
}

/// Search memories for prompt enrichment.
///
/// Uses keyword-based recall for the text query, then applies pipeline-aware
/// tag and date-range filters on the results. This gives better relevance
/// ranking than substring matching when the query contains multiple words.
pub async fn search_for_prompt(
    graph: &MemoryGraph,
    query: &str,
    filters: &MemoryFilters,
) -> Result<Vec<MemoryEntry>> {
    // Collect required tags from filters.
    let mut required_tags: Vec<String> = filters.tags.clone();

    if let Some(ref pt) = filters.pipeline_type {
        required_tags.push(pipeline_type_tag(pt).to_string());
    }

    required_tags.extend(filters.agent_keys.iter().cloned());

    // Use keyword-based recall for the query text (handles multi-word queries).
    // Fetch a generous number then filter down.
    let recall_limit = 100;
    let mut memories = graph.recall_memories(query, recall_limit)?;

    // If recall returned nothing (keywords didn't match), fall back to
    // structured search which does substring matching per-word is not
    // available — try the structured search path instead.
    if memories.is_empty() {
        let search_filter = SearchFilter {
            memory_type: None,
            require_all_tags: !required_tags.is_empty(),
            tags: required_tags.clone(),
            text: Some(query.to_string()),
            date_from: filters.time_range.map(|(from, _)| from),
            date_to: filters.time_range.map(|(_, to)| to),
        };
        memories = graph.search_memories(&search_filter)?;
    }

    // Apply tag filters on recall results.
    if !required_tags.is_empty() {
        memories.retain(|m| required_tags.iter().all(|rt| m.tags.contains(rt)));
    }

    // Apply date-range filter.
    if let Some((from, to)) = filters.time_range {
        memories.retain(|m| m.created_at >= from && m.created_at <= to);
    }

    Ok(memories.into_iter().map(memory_to_entry).collect())
}

/// Create a relationship between two agent outputs.
///
/// The `relationship_type` string is mapped to a [`RelType`] variant. Unknown
/// types default to [`RelType::RelatedTo`].
pub async fn create_agent_relationship(
    graph: &MemoryGraph,
    from_id: &str,
    to_id: &str,
    relationship_type: &str,
    session_id: &str,
) -> Result<()> {
    let rel_type = RelType::from_str_opt(relationship_type).unwrap_or(RelType::RelatedTo);

    graph.create_relationship(
        from_id,
        to_id,
        rel_type,
        Some(&format!("pipeline session: {session_id}")),
        1.0,
    )?;

    Ok(())
}
