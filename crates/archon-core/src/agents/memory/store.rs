use archon_memory::MemoryTrait;
use archon_memory::types::{MemoryError, MemoryType, SearchFilter};

use super::tags::{agent_tag, scope_tag};
use crate::agents::definition::AgentMemoryScope;

/// Load agent-scoped memories from the memory store.
///
/// For each query in `recall_queries`, searches memories tagged with
/// `agent:{agent_type}` (and optionally a scope tag) and returns the
/// matching content strings.
///
/// If `memory_scope` is `None`, returns empty — AC-101: no persistent memory.
pub fn load_agent_memory(
    agent_type: &str,
    recall_queries: &[String],
    memory: &dyn MemoryTrait,
    memory_scope: Option<&AgentMemoryScope>,
) -> Vec<String> {
    // AC-101: None = no persistent memory
    let scope = match memory_scope {
        Some(s) => s,
        None => return Vec::new(),
    };

    let agent = agent_tag(agent_type);
    let scope = scope_tag(scope);
    let mut results = Vec::new();

    for query in recall_queries {
        let filter = SearchFilter {
            memory_type: None,
            tags: vec![agent.clone(), scope.clone()],
            require_all_tags: true,
            text: Some(query.clone()),
            date_from: None,
            date_to: None,
        };

        match memory.search_memories(&filter) {
            Ok(memories) => {
                for m in memories {
                    results.push(m.content.clone());
                }
            }
            Err(e) => {
                tracing::warn!(agent = agent_type, query = query.as_str(), error = %e,
                    "failed to recall agent memory");
            }
        }
    }

    results
}

/// Save a memory scoped to a specific agent.
///
/// Tags the memory with `agent:{agent_type}` and `scope:{scope}` plus any
/// additional user tags.
///
/// If `memory_scope` is `None`, this is a no-op (AC-101: no persistent memory).
pub fn save_agent_memory(
    agent_type: &str,
    content: &str,
    title: &str,
    extra_tags: &[String],
    memory: &dyn MemoryTrait,
    project_path: &str,
    memory_scope: Option<&AgentMemoryScope>,
) -> Result<String, MemoryError> {
    // AC-101: None = no persistent memory
    let scope = match memory_scope {
        Some(s) => s,
        None => return Ok("skipped-no-scope".to_string()),
    };

    let mut tags = vec![agent_tag(agent_type), scope_tag(scope)];
    tags.extend(extra_tags.iter().cloned());

    memory.store_memory(
        content,
        title,
        MemoryType::Fact,
        0.5,
        &tags,
        "agent",
        project_path,
    )
}
