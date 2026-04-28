//! Semantic codebase search via the LEANN engine.
//!
//! Wraps `archon_leann::CodeIndex::search_code()` as an agent-callable tool.
//! Read-only — no index mutation, no shell, no network.

use std::sync::Arc;

use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};
use archon_leann::CodeIndex;

pub struct LeannSearchTool {
    index: Arc<CodeIndex>,
}

impl LeannSearchTool {
    pub fn new(index: Arc<CodeIndex>) -> Self {
        Self { index }
    }
}

#[async_trait::async_trait]
impl Tool for LeannSearchTool {
    fn name(&self) -> &str {
        "LeannSearch"
    }

    fn description(&self) -> &str {
        "Semantic search over the indexed codebase. Given a natural-language \
         query, returns matching code chunks with file paths, line ranges, \
         and relevance scores. Use this when grep/glob would miss conceptually \
         similar code (e.g. \"find functions that handle authentication\")."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": { "type": "string", "description": "Natural-language search query" },
                "limit": { "type": "integer", "description": "Max results (default 10, max 50)" }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let query = match input
            .get("query")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            Some(q) => q,
            None => return ToolResult::error("query is required"),
        };
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .min(50) as usize;

        match self.index.search_code(query, limit) {
            Ok(results) => {
                let formatted: Vec<_> = results
                    .iter()
                    .map(|r| {
                        json!({
                            "file": r.file_path.display().to_string(),
                            "lines": format!("{}-{}", r.line_start, r.line_end),
                            "language": r.language,
                            "score": r.relevance_score,
                            "content": r.content,
                        })
                    })
                    .collect();
                ToolResult::success(
                    serde_json::to_string_pretty(&json!({
                        "query": query,
                        "result_count": formatted.len(),
                        "results": formatted,
                    }))
                    .unwrap_or_default(),
                )
            }
            Err(e) => ToolResult::error(format!("LEANN search failed: {e}")),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
