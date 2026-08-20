use serde_json::json;

use crate::filesystem::FileSystem;

use crate::path_guard::resolve_write_target_path;
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult, WorkingTreeEffect};

pub struct WriteTool;

const LARGE_REWRITE_MAX_BYTES: usize = 64 * 1024;
const LARGE_REWRITE_MAX_LINES: usize = 300;

#[async_trait::async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "Write"
    }

    /// Names the alternatives, because the model chooses from this sentence.
    ///
    /// The old text — "Writes content to a file... Overwrites existing files."
    /// — described the mechanism and said nothing about when NOT to reach for
    /// it, so a whole-file rewrite read as the obvious way to change a file.
    /// It is the most expensive one available: every unchanged line is
    /// regenerated, the model pays output tokens to retype code it is not
    /// changing, and the resulting diff is unreviewable.
    ///
    /// Until now the only steer lived in `tool_input_json`'s recovery hint,
    /// which fires AFTER a large Write has already truncated mid-file. That is
    /// the right advice one wasted turn too late.
    fn description(&self) -> &str {
        "Creates a NEW file, or replaces an existing one in full. Creates parent          directories if needed. For a file that already exists, prefer Edit for a          localised change, ApplyPatch for several changes at once, or          LargeEditBegin for a file above a few hundred lines — a whole-file          rewrite regenerates every unchanged line and can be truncated mid-file.          Use Write on an existing file only when the change touches nearly all of          it."
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

        let fs = ctx.fs();

        if let Err(message) = reject_large_existing_rewrite(fs.as_ref(), &path, &content).await {
            return ToolResult::error(message);
        }

        // Create parent directories
        if let Some(parent) = path.parent()
            && !fs.exists(parent).await
            && let Err(e) = fs.create_dir_all(parent).await
        {
            return ToolResult::error(format!("Failed to create parent directory: {e}"));
        }

        match fs.write(&path, content.as_bytes()).await {
            Ok(()) => ToolResult::success(format!("File created successfully at: {file_path}")),
            Err(e) => ToolResult::error(format!("Failed to write file: {e}")),
        }
    }

    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::DeclaredPaths
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}

async fn reject_large_existing_rewrite(
    fs: &dyn FileSystem,
    path: &std::path::Path,
    content: &str,
) -> Result<(), String> {
    if !fs.exists(path).await {
        return Ok(());
    }

    let existing = fs.read(path).await.map_err(|e| {
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

#[cfg(test)]
mod description_tests {
    use super::*;
    use crate::tool::Tool;

    /// The description is the only thing the model reads before choosing a
    /// tool, so the steer has to survive edits to it. Without this, a tidy-up
    /// that shortens the sentence silently restores whole-file Write as the
    /// obvious default and the failure returns as a truncated file.
    #[test]
    fn write_names_the_cheaper_alternatives() {
        let description = WriteTool.description();
        for alternative in ["Edit", "ApplyPatch", "LargeEditBegin"] {
            assert!(
                description.contains(alternative),
                "Write's description must name {alternative} so the model has \
                 somewhere to go: {description}"
            );
        }
    }

    /// Naming the alternatives is not enough on its own — the description also
    /// has to say that a whole-file rewrite is the expensive path, or a model
    /// reads the list as three equal options.
    #[test]
    fn write_says_why_a_full_rewrite_costs_more() {
        let description = WriteTool.description();
        assert!(
            description.contains("regenerates every unchanged line"),
            "Write's description must say what a full rewrite costs: {description}"
        );
    }
}
