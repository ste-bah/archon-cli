//! ToolSearch tool — lets the LLM discover deferred tools by name or keyword.

use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

const DEFAULT_MAX_RESULTS: usize = 5;

/// A tool that searches the tool registry and returns full schema definitions.
///
/// Two query modes:
/// - `"select:Tool1,Tool2"` — exact name match
/// - `"keyword terms"` — fuzzy substring search across names and descriptions
pub struct ToolSearchTool {
    tool_defs: Vec<serde_json::Value>,
}

impl ToolSearchTool {
    pub fn new(tool_defs: Vec<serde_json::Value>) -> Self {
        Self { tool_defs }
    }
}

/// Compute a simple relevance score for `haystack` against a set of search `terms`.
/// Returns the count of terms that appear as case-insensitive substrings.
fn score_match(haystack: &str, terms: &[&str]) -> usize {
    let lower = haystack.to_lowercase();
    terms.iter().filter(|t| lower.contains(&t.to_lowercase())).count()
}

#[async_trait::async_trait]
impl Tool for ToolSearchTool {
    fn name(&self) -> &str {
        "ToolSearch"
    }

    fn description(&self) -> &str {
        "Fetches full schema definitions for deferred tools so they can be called. \
         Supports \"select:Tool1,Tool2\" for exact lookup or keyword search."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "\"select:Name1,Name2\" for exact match, or keywords for fuzzy search"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum results to return (default 5)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let query = match input.get("query").and_then(|v| v.as_str()) {
            Some(q) => q.to_string(),
            None => return ToolResult::error("query is required and must be a string"),
        };

        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_MAX_RESULTS);

        let all_defs = self.tool_defs.clone();

        let matched: Vec<serde_json::Value> = if let Some(names_csv) = query.strip_prefix("select:") {
            // Exact name match mode
            let requested: Vec<&str> = names_csv.split(',').map(|s| s.trim()).collect();
            all_defs
                .into_iter()
                .filter(|def| {
                    def.get("name")
                        .and_then(|n| n.as_str())
                        .is_some_and(|name| requested.iter().any(|r| r.eq_ignore_ascii_case(name)))
                })
                .collect()
        } else {
            // Keyword search mode
            let terms: Vec<&str> = query.split_whitespace().collect();
            if terms.is_empty() {
                return ToolResult::error("query must not be empty");
            }

            let mut scored: Vec<(usize, serde_json::Value)> = all_defs
                .into_iter()
                .filter_map(|def| {
                    let name = def.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let desc = def.get("description").and_then(|d| d.as_str()).unwrap_or("");
                    let combined = format!("{name} {desc}");
                    let s = score_match(&combined, &terms);
                    if s > 0 { Some((s, def)) } else { None }
                })
                .collect();

            // Sort descending by score, then alphabetically by name for stability
            scored.sort_by(|a, b| {
                b.0.cmp(&a.0).then_with(|| {
                    let an = a.1.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let bn = b.1.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    an.cmp(bn)
                })
            });

            scored.into_iter().take(max_results).map(|(_, def)| def).collect()
        };

        if matched.is_empty() {
            return ToolResult::success("No matching tools found.");
        }

        match serde_json::to_string_pretty(&matched) {
            Ok(json_str) => ToolResult::success(json_str),
            Err(e) => ToolResult::error(format!("Failed to serialize results: {e}")),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{AgentMode, ToolContext};

    fn test_ctx() -> ToolContext {
        ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: "test".into(),
            mode: AgentMode::Normal,
        }
    }

    fn populated_tool_defs() -> Vec<serde_json::Value> {
        vec![
            serde_json::json!({"name": "Read", "description": "Reads a file from the filesystem", "input_schema": {"type": "object"}}),
            serde_json::json!({"name": "Write", "description": "Writes a file to the filesystem", "input_schema": {"type": "object"}}),
            serde_json::json!({"name": "Grep", "description": "Search file contents with regex", "input_schema": {"type": "object"}}),
            serde_json::json!({"name": "Bash", "description": "Execute a bash command", "input_schema": {"type": "object"}}),
            serde_json::json!({"name": "WebFetch", "description": "Fetch a web page", "input_schema": {"type": "object"}}),
            serde_json::json!({"name": "NotebookEdit", "description": "Edit Jupyter notebooks", "input_schema": {"type": "object"}}),
        ]
    }

    #[test]
    fn metadata() {
        let tool = ToolSearchTool::new(vec![]);
        assert_eq!(tool.name(), "ToolSearch");
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert_eq!(schema["required"][0], "query");
    }

    #[test]
    fn permission_is_safe() {
        let tool = ToolSearchTool::new(vec![]);
        assert_eq!(tool.permission_level(&json!({})), PermissionLevel::Safe);
    }

    #[tokio::test]
    async fn select_exact_names() {
        let tool = ToolSearchTool::new(populated_tool_defs());
        let result = tool.execute(json!({"query": "select:Read,Grep"}), &test_ctx()).await;
        assert!(!result.is_error);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result.content).expect("valid json");
        let names: Vec<&str> = parsed.iter().filter_map(|d| d["name"].as_str()).collect();
        assert!(names.contains(&"Read"));
        assert!(names.contains(&"Grep"));
        assert!(!names.contains(&"Write"));
    }

    #[tokio::test]
    async fn select_missing_tool_returns_empty() {
        let tool = ToolSearchTool::new(populated_tool_defs());
        let result = tool.execute(json!({"query": "select:NonExistent"}), &test_ctx()).await;
        assert!(!result.is_error);
        assert!(result.content.contains("No matching tools found"));
    }

    #[tokio::test]
    async fn keyword_search_matches() {
        let tool = ToolSearchTool::new(populated_tool_defs());
        let result = tool.execute(json!({"query": "file"}), &test_ctx()).await;
        assert!(!result.is_error);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result.content).expect("valid json");
        let names: Vec<&str> = parsed.iter().filter_map(|d| d["name"].as_str()).collect();
        // "file" appears in Read and Write descriptions
        assert!(names.contains(&"Read"));
        assert!(names.contains(&"Write"));
    }

    #[tokio::test]
    async fn keyword_search_respects_max_results() {
        let tool = ToolSearchTool::new(populated_tool_defs());
        let result = tool.execute(json!({"query": "a", "max_results": 2}), &test_ctx()).await;
        assert!(!result.is_error);
        // "a" is very common — should match many, but cap at 2
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result.content).expect("valid json");
        assert!(parsed.len() <= 2);
    }

    #[tokio::test]
    async fn missing_query_is_error() {
        let tool = ToolSearchTool::new(vec![]);
        let result = tool.execute(json!({}), &test_ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("query is required"));
    }

    #[tokio::test]
    async fn empty_keyword_is_error() {
        let tool = ToolSearchTool::new(vec![]);
        let result = tool.execute(json!({"query": "   "}), &test_ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("must not be empty"));
    }

    #[test]
    fn score_match_counts_terms() {
        assert_eq!(score_match("Read a file from disk", &["file", "read"]), 2);
        assert_eq!(score_match("Read a file from disk", &["write"]), 0);
        assert_eq!(score_match("Read a file from disk", &["FILE"]), 1); // case insensitive
    }
}
