use std::fs;

use serde_json::json;

use crate::path_guard::resolve_write_target_path;
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
                    "description": "Path to the file to write. Must resolve inside working_dir or an allowed extra_dir."
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let file_path = match string_field_any(
            &input,
            &[
                "file_path",
                "filePath",
                "filepath",
                "path",
                "output_path",
                "output_file",
                "target_path",
                "target_file",
                "destination_path",
                "destination",
                "save_path",
                "filename",
                "file_name",
                "file",
            ],
        ) {
            Some(p) => p,
            None => return ToolResult::error("file_path is required and must be a string"),
        };

        let content = match string_field_any(
            &input,
            &[
                "content",
                "contents",
                "file_content",
                "fileContents",
                "text",
                "body",
                "document",
                "markdown",
                "data",
                "value",
            ],
        ) {
            Some(c) => c,
            None => return ToolResult::error("content is required and must be a string"),
        };

        let path = match resolve_write_target_path(&file_path, ctx) {
            Ok(path) => path,
            Err(e) => return ToolResult::error(e),
        };

        // Create parent directories
        if let Some(parent) = path.parent()
            && !parent.exists()
            && let Err(e) = fs::create_dir_all(parent)
        {
            return ToolResult::error(format!("Failed to create parent directory: {e}"));
        }

        match fs::write(&path, content) {
            Ok(()) => ToolResult::success(format!("File created successfully at: {file_path}")),
            Err(e) => ToolResult::error(format!("Failed to write file: {e}")),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}

fn string_field_any(input: &serde_json::Value, keys: &[&str]) -> Option<String> {
    direct_string_field(input, keys)
        .or_else(|| parsed_string_field(input, keys))
        .or_else(|| {
            [
                "input",
                "tool_input",
                "arguments",
                "args",
                "parameters",
                "params",
            ]
            .iter()
            .find_map(|key| input.get(*key).and_then(|v| wrapped_string_field(v, keys)))
        })
}

fn direct_string_field(input: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| input.get(*key))
        .find_map(string_value)
}

fn parsed_string_field(input: &serde_json::Value, keys: &[&str]) -> Option<String> {
    input
        .as_str()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|parsed| direct_string_field(&parsed, keys))
}

fn wrapped_string_field(input: &serde_json::Value, keys: &[&str]) -> Option<String> {
    direct_string_field(input, keys).or_else(|| parsed_string_field(input, keys))
}

fn string_value(value: &serde_json::Value) -> Option<String> {
    value.as_str().map(ToOwned::to_owned).or_else(|| {
        ["value", "text", "path", "file_path"]
            .iter()
            .find_map(|key| value.get(*key).and_then(|v| v.as_str()))
            .map(ToOwned::to_owned)
    })
}
