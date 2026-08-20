use std::fs;

use serde_json::json;

use crate::path_guard::resolve_existing_file_path;
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult, WorkingTreeEffect};

pub struct EditTool;

#[async_trait::async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "Edit"
    }

    /// Claims the default, which the old one-liner did not.
    ///
    /// "Performs exact string replacements in files." is accurate and says
    /// nothing about when to choose it, so it read as one option beside Write
    /// rather than the one to reach for first.
    fn description(&self) -> &str {
        "The default way to change a file that already exists: an exact string          replacement that leaves everything outside the match untouched.          old_string must appear exactly once — widen it with surrounding lines          to disambiguate. A series of small edits is cheaper and safer than one          whole-file Write, because only the changed region is emitted."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file to modify. Must resolve inside working_dir or an allowed extra_dir."
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

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
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

        let path = match resolve_existing_file_path(file_path, ctx) {
            Ok(path) => path,
            Err(e) => return ToolResult::error(e),
        };
        let content = match fs::read_to_string(&path) {
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

        match fs::write(&path, new_content) {
            Ok(()) => ToolResult::success(format!("File {file_path} updated successfully.")),
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
