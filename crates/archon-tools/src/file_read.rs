use std::fs;
use std::path::Path;

use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

pub struct ReadTool;

#[async_trait::async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "Read"
    }

    fn description(&self) -> &str {
        "Reads a file from the filesystem. Returns content with line numbers."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (0-based)",
                    "minimum": 0
                },
                "limit": {
                    "type": "integer",
                    "description": "Number of lines to read",
                    "minimum": 1
                }
            },
            "required": ["file_path"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let file_path = match input.get("file_path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::error("file_path is required and must be a string"),
        };

        let path = Path::new(file_path);
        if !path.exists() {
            return ToolResult::error(format!("File does not exist: {file_path}"));
        }

        // Check if file is likely binary
        let content = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => return ToolResult::error(format!("Failed to read file: {e}")),
        };

        if content.iter().take(8192).any(|&b| b == 0) {
            return ToolResult::error(format!(
                "File appears to be binary: {file_path}. Use a specialized tool for binary files."
            ));
        }

        let text = match String::from_utf8(content) {
            Ok(s) => s,
            Err(_) => {
                return ToolResult::error(format!(
                    "File is not valid UTF-8: {file_path}"
                ))
            }
        };

        let lines: Vec<&str> = text.lines().collect();
        let offset = input
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(2000);

        let end = (offset + limit).min(lines.len());
        if offset >= lines.len() {
            return ToolResult::error(format!(
                "Offset {offset} is beyond file length ({} lines)",
                lines.len()
            ));
        }

        let numbered: String = lines[offset..end]
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{}\t{}", offset + i + 1, line))
            .collect::<Vec<_>>()
            .join("\n");

        ToolResult::success(numbered)
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
