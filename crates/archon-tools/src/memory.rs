use std::sync::Arc;

use archon_memory::graph::MemoryGraph;
use archon_memory::types::MemoryType;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// memory_store
// ---------------------------------------------------------------------------

/// Store a memory in the persistent CozoDB memory graph.
///
/// Mirrors `mcp__memorygraph__store_memory` in the Claude Code host.
pub struct MemoryStoreTool {
    graph: Arc<MemoryGraph>,
}

impl MemoryStoreTool {
    pub fn new(graph: Arc<MemoryGraph>) -> Self {
        Self { graph }
    }
}

#[async_trait::async_trait]
impl Tool for MemoryStoreTool {
    fn name(&self) -> &str {
        "memory_store"
    }

    fn description(&self) -> &str {
        "Store a memory in the persistent memory graph (CozoDB). \
         Use this when the user asks you to remember something, or when you \
         want to save an important fact, decision, preference, or rule for \
         future recall. Stored memories persist across sessions."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The content to store in memory"
                },
                "title": {
                    "type": "string",
                    "description": "Short descriptive title for the memory (optional)"
                },
                "memory_type": {
                    "type": "string",
                    "enum": ["fact", "decision", "preference", "rule"],
                    "description": "Type of memory. Use 'fact' for information, 'decision' for choices made, 'preference' for user preferences, 'rule' for behavioral rules. Defaults to 'fact'."
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional tags to categorize the memory (e.g. [\"rust\", \"async\"])"
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let content = match input["content"].as_str() {
            Some(c) if !c.trim().is_empty() => c,
            _ => return ToolResult::error("content is required and must be non-empty"),
        };
        let title = input["title"].as_str().unwrap_or("");
        let memory_type = match input["memory_type"].as_str().unwrap_or("fact") {
            "decision"   => MemoryType::Decision,
            "preference" => MemoryType::Preference,
            "rule"       => MemoryType::Rule,
            _            => MemoryType::Fact,
        };
        let tags: Vec<String> = input["tags"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        match self.graph.store_memory(content, title, memory_type, 0.8, &tags, "agent", "") {
            Ok(id) => {
                let short_id = &id[..8.min(id.len())];
                ToolResult::success(format!("Memory stored [{short_id}]: {title}"))
            }
            Err(e) => ToolResult::error(format!("Failed to store memory: {e}")),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

// ---------------------------------------------------------------------------
// memory_recall
// ---------------------------------------------------------------------------

/// Search the persistent CozoDB memory graph by keyword.
///
/// Mirrors `mcp__memorygraph__recall_memories` in the Claude Code host.
pub struct MemoryRecallTool {
    graph: Arc<MemoryGraph>,
}

impl MemoryRecallTool {
    pub fn new(graph: Arc<MemoryGraph>) -> Self {
        Self { graph }
    }
}

#[async_trait::async_trait]
impl Tool for MemoryRecallTool {
    fn name(&self) -> &str {
        "memory_recall"
    }

    fn description(&self) -> &str {
        "Search the persistent memory graph (CozoDB) for stored memories matching \
         a keyword query. Use this when the user asks you to recall something, \
         or when you need to look up past information, decisions, or preferences. \
         Returns results ranked by relevance and recency."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords to search for in stored memories"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default 10)",
                    "default": 10
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let query = match input["query"].as_str() {
            Some(q) if !q.trim().is_empty() => q,
            _ => return ToolResult::error("query is required and must be non-empty"),
        };
        let limit = input["limit"].as_u64().unwrap_or(10).min(50) as usize;

        match self.graph.recall_memories(query, limit) {
            Ok(memories) if memories.is_empty() => {
                ToolResult::success(format!("No memories found for '{query}'."))
            }
            Ok(memories) => {
                let mut out = format!("{} memories found for '{query}':\n\n", memories.len());
                for m in &memories {
                    let title = if m.title.is_empty() { "(untitled)" } else { &m.title };
                    let short_id = &m.id[..8.min(m.id.len())];
                    let snippet: String = m.content.chars().take(300).collect();
                    let ellipsis = if m.content.chars().count() > 300 { "..." } else { "" };
                    out.push_str(&format!(
                        "[{short_id}] {title}\n{snippet}{ellipsis}\n\n"
                    ));
                }
                ToolResult::success(out)
            }
            Err(e) => ToolResult::error(format!("Memory recall failed: {e}")),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_memory::graph::MemoryGraph;
    use std::path::PathBuf;

    fn test_ctx() -> ToolContext {
        ToolContext {
            working_dir: PathBuf::from("/tmp"),
            session_id: "test-session".to_string(),
            mode: crate::tool::AgentMode::Normal,
        }
    }

    #[tokio::test]
    async fn store_and_recall_roundtrip() {
        let graph = Arc::new(MemoryGraph::in_memory().expect("graph"));
        let store = MemoryStoreTool::new(Arc::clone(&graph));
        let recall = MemoryRecallTool::new(Arc::clone(&graph));
        let ctx = test_ctx();

        let result = store.execute(serde_json::json!({
            "content": "Rust async uses tokio runtime",
            "title": "async runtime note",
            "memory_type": "fact"
        }), &ctx).await;
        assert!(!result.is_error, "store failed: {}", result.content);

        let result = recall.execute(serde_json::json!({
            "query": "tokio"
        }), &ctx).await;
        assert!(!result.is_error);
        assert!(result.content.contains("tokio"), "recall missed stored content");
    }

    #[tokio::test]
    async fn store_requires_content() {
        let graph = Arc::new(MemoryGraph::in_memory().expect("graph"));
        let store = MemoryStoreTool::new(Arc::clone(&graph));
        let ctx = test_ctx();

        let result = store.execute(serde_json::json!({}), &ctx).await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn recall_no_results() {
        let graph = Arc::new(MemoryGraph::in_memory().expect("graph"));
        let recall = MemoryRecallTool::new(Arc::clone(&graph));
        let ctx = test_ctx();

        let result = recall.execute(serde_json::json!({
            "query": "xyzzy_nonexistent_term_12345"
        }), &ctx).await;
        assert!(!result.is_error);
        assert!(result.content.contains("No memories found"));
    }
}
