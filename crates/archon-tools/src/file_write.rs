use std::fs;
use std::path::Path;

use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

pub struct WriteTool;

#[async_trait::async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "Write"
    }

    fn description(&self) -> &str {
        "Writes content to a file. Creates parent directories if needed. Overwrites existing files."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let file_path = match input.get("file_path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::error("file_path is required and must be a string"),
        };

        let content = match input.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::error("content is required and must be a string"),
        };

        let path = Path::new(file_path);

        // Create parent directories
        if let Some(parent) = path.parent()
            && !parent.exists()
            && let Err(e) = fs::create_dir_all(parent)
        {
            return ToolResult::error(format!("Failed to create parent directory: {e}"));
        }

        match fs::write(path, content) {
            Ok(()) => ToolResult::success(format!("File created successfully at: {file_path}")),
            Err(e) => ToolResult::error(format!("Failed to write file: {e}")),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}
