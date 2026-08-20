use serde_json::json;

use crate::path_guard::resolve_existing_path;
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult, WorkingTreeEffect};

pub struct GlobTool;

#[async_trait::async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "Glob"
    }

    fn description(&self) -> &str {
        "Fast file pattern matching. Returns matching file paths sorted by modification time."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match files (e.g., '**/*.rs')"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (defaults to working directory)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let pattern = match input.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::error("pattern is required and must be a string"),
        };

        let base_dir = match input.get("path").and_then(|v| v.as_str()) {
            Some(path) => match resolve_existing_path(path, ctx) {
                Ok(path) => path,
                Err(err) => return ToolResult::error(err),
            },
            None => match resolve_existing_path(".", ctx) {
                Ok(path) => path,
                Err(err) => return ToolResult::error(err),
            },
        };

        let fs = ctx.fs();
        let matched = match fs.glob(&base_dir, pattern).await {
            Ok(paths) => paths,
            Err(e) => {
                return ToolResult::error(format!("Invalid glob pattern: {e}"));
            }
        };

        let mut files: Vec<(std::path::PathBuf, Option<u128>)> = Vec::new();
        for path in matched {
            let mtime = fs.metadata(&path).await.ok().and_then(|m| m.modified_nanos);
            files.push((path, mtime));
        }

        // Sort by mtime, newest first. A path whose world reports no time
        // sorts last rather than as the epoch, so "unknown" cannot masquerade
        // as "oldest".
        files.sort_by_key(|(path, mtime)| (std::cmp::Reverse(*mtime), path.clone()));

        if files.is_empty() {
            return ToolResult::success("No files matched the pattern.");
        }

        let result: String = files
            .iter()
            .map(|(path, _)| path.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("\n");

        ToolResult::success(result)
    }

    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::None
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
