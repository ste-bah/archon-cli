use std::fs;
use std::path::Path;

use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

pub struct EditTool;

#[async_trait::async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "Edit"
    }

    fn description(&self) -> &str {
        "Performs exact string replacements in files."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to modify"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact text to replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The replacement text"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default false)",
                    "default": false
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let file_path = match input.get("file_path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::error("file_path is required and must be a string"),
        };

        let old_string = match input.get("old_string").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::error("old_string is required and must be a string"),
        };

        let new_string = match input.get("new_string").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::error("new_string is required and must be a string"),
        };

        if old_string == new_string {
            return ToolResult::error("old_string and new_string must be different");
        }

        let replace_all = input
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let path = Path::new(file_path);
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("Failed to read file: {e}")),
        };

        if !content.contains(old_string) {
            return ToolResult::error(format!("old_string not found in {file_path}"));
        }

        let match_count = content.matches(old_string).count();

        if !replace_all && match_count > 1 {
            return ToolResult::error(format!(
                "old_string matches {match_count} locations in {file_path}. \
                 Use replace_all: true or provide more context to make it unique."
            ));
        }

        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        match fs::write(path, new_content) {
            Ok(()) => ToolResult::success(format!("File {file_path} updated successfully.")),
            Err(e) => ToolResult::error(format!("Failed to write file: {e}")),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}
