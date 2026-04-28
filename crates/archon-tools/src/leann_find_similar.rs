//! Find similar code via the LEANN engine.
//!
//! Wraps `archon_leann::CodeIndex::find_similar_code()` as an agent-callable
//! tool. Read-only — no index mutation, no shell, no network.

use std::sync::Arc;

use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};
use archon_leann::CodeIndex;

pub struct LeannFindSimilarTool {
    index: Arc<CodeIndex>,
}

impl LeannFindSimilarTool {
    pub fn new(index: Arc<CodeIndex>) -> Self {
        Self { index }
    }
}

#[async_trait::async_trait]
impl Tool for LeannFindSimilarTool {
    fn name(&self) -> &str {
        "LeannFindSimilar"
    }

    fn description(&self) -> &str {
        "Find code snippets semantically similar to the provided snippet. \
         Given a code fragment, returns matching chunks from across the \
         indexed codebase with file paths, line ranges, and relevance scores. \
         Use this to discover related implementations, duplicate logic, or \
         existing patterns before writing new code."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["snippet"],
            "properties": {
                "snippet": { "type": "string", "description": "Code snippet to find similar code for" },
                "limit": { "type": "integer", "description": "Max results (default 10, max 50)" }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let snippet = match input
            .get("snippet")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            Some(s) => s,
            None => return ToolResult::error("snippet is required"),
        };
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .min(50) as usize;

        match self.index.find_similar_code(snippet, limit) {
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
                        "snippet_preview": &snippet[..snippet.len().min(120)],
                        "result_count": formatted.len(),
                        "results": formatted,
                    }))
                    .unwrap_or_default(),
                )
            }
            Err(e) => ToolResult::error(format!("LEANN find-similar failed: {e}")),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
