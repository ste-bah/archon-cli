use std::fs;

use serde_json::json;

use crate::path_guard::resolve_write_target_path;
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

pub struct WriteTool;

const LARGE_REWRITE_MAX_BYTES: usize = 64 * 1024;
const LARGE_REWRITE_MAX_LINES: usize = 300;

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

        if let Err(message) = reject_large_existing_rewrite(&path, &content) {
            return ToolResult::error(message);
        }

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

fn reject_large_existing_rewrite(path: &std::path::Path, content: &str) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    let existing = fs::read(path).map_err(|e| {
        format!(
            "Failed to inspect existing file before Write '{}': {e}",
            path.display()
        )
    })?;
    let existing_lines = byte_line_count(&existing);
    let incoming_lines = byte_line_count(content.as_bytes());
    let large_existing =
        existing.len() > LARGE_REWRITE_MAX_BYTES || existing_lines > LARGE_REWRITE_MAX_LINES;
    let large_incoming =
        content.len() > LARGE_REWRITE_MAX_BYTES || incoming_lines > LARGE_REWRITE_MAX_LINES;
    if large_existing || large_incoming {
        return Err(format!(
            "Write refuses large full-file rewrites for existing files (existing: {} bytes/{existing_lines} lines, incoming: {} bytes/{incoming_lines} lines). \
             Use LargeEditBegin, LargeEditReplaceSection/LargeEditInsertAfter/LargeEditDeleteSection, then LargeEditCommit so Archon edits by anchors in small transactional chunks.",
            existing.len(),
            content.len()
        ));
    }

    Ok(())
}

fn byte_line_count(bytes: &[u8]) -> usize {
    if bytes.is_empty() {
        0
    } else {
        bytes.iter().filter(|byte| **byte == b'\n').count() + 1
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
