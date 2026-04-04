use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

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

        let base_dir = input
            .get("path")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());

        let full_pattern = base_dir.join(pattern);
        let pattern_str = full_pattern.to_string_lossy();

        let entries = match glob::glob(&pattern_str) {
            Ok(paths) => paths,
            Err(e) => {
                return ToolResult::error(format!("Invalid glob pattern: {e}"));
            }
        };

        let mut files: Vec<(std::path::PathBuf, std::time::SystemTime)> = Vec::new();

        for entry in entries {
            match entry {
                Ok(path) => {
                    let mtime = path
                        .metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    files.push((path, mtime));
                }
                Err(e) => {
                    tracing::debug!("glob entry error: {e}");
                }
            }
        }

        // Sort by mtime, newest first
        files.sort_by(|a, b| b.1.cmp(&a.1));

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

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
