mod ops;
mod session;

use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

pub struct LargeEditBeginTool;
pub struct LargeEditInsertAfterTool;
pub struct LargeEditReplaceSectionTool;
pub struct LargeEditDeleteSectionTool;
pub struct LargeEditCommitTool;
pub struct LargeEditAbortTool;

#[async_trait::async_trait]
impl Tool for LargeEditBeginTool {
    fn name(&self) -> &str {
        "LargeEditBegin"
    }

    fn description(&self) -> &str {
        "Begin a transactional large-file edit. Creates a staged copy and returns an edit_id. Use this instead of full-file Write for existing files above a few hundred lines."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Existing file to edit. Must resolve inside working_dir or an allowed extra_dir."
                }
            },
            "required": ["file_path"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let file_path = match string_field(&input, "file_path") {
            Some(value) => value,
            None => return ToolResult::error("file_path is required and must be a string"),
        };
        match session::begin(&file_path, ctx) {
            Ok(session) => ToolResult::success(
                json!({
                    "edit_id": session.meta.edit_id,
                    "target_path": session.meta.target_path,
                    "original_hash": session.meta.original_hash,
                    "status": "staged"
                })
                .to_string(),
            ),
            Err(err) => ToolResult::error(err),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}

#[async_trait::async_trait]
impl Tool for LargeEditInsertAfterTool {
    fn name(&self) -> &str {
        "LargeEditInsertAfter"
    }

    fn description(&self) -> &str {
        "Insert a content chunk after an anchor line in a staged large edit. Requires an edit_id from LargeEditBegin."
    }

    fn input_schema(&self) -> serde_json::Value {
        mutation_schema(&["edit_id", "anchor", "content"])
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let Some(args) = mutation_args(&input, true) else {
            return ToolResult::error("edit_id, anchor, and content are required strings");
        };
        let result = session::mutate(&args.edit_id, ctx, |staged| {
            ops::insert_after(staged, &args.anchor, &args.content, args.occurrence)
        });
        tool_result(result)
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}

#[async_trait::async_trait]
impl Tool for LargeEditReplaceSectionTool {
    fn name(&self) -> &str {
        "LargeEditReplaceSection"
    }

    fn description(&self) -> &str {
        "Replace a staged section by start anchor and optional end anchor. Without end_anchor, markdown headings replace through the next peer/parent heading."
    }

    fn input_schema(&self) -> serde_json::Value {
        mutation_schema(&["edit_id", "start_anchor", "content"])
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let Some(args) = mutation_args(&input, true) else {
            return ToolResult::error("edit_id, start_anchor, and content are required strings");
        };
        let result = session::mutate(&args.edit_id, ctx, |staged| {
            ops::replace_section(
                staged,
                &args.anchor,
                args.end_anchor.as_deref(),
                &args.content,
                args.occurrence,
            )
        });
        tool_result(result)
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}

#[async_trait::async_trait]
impl Tool for LargeEditDeleteSectionTool {
    fn name(&self) -> &str {
        "LargeEditDeleteSection"
    }

    fn description(&self) -> &str {
        "Delete a staged section by start anchor and optional end anchor. Without end_anchor, markdown headings delete through the next peer/parent heading."
    }

    fn input_schema(&self) -> serde_json::Value {
        mutation_schema(&["edit_id", "start_anchor"])
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let Some(args) = mutation_args(&input, false) else {
            return ToolResult::error("edit_id and start_anchor are required strings");
        };
        let result = session::mutate(&args.edit_id, ctx, |staged| {
            ops::delete_section(
                staged,
                &args.anchor,
                args.end_anchor.as_deref(),
                args.occurrence,
            )
        });
        tool_result(result)
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}

#[async_trait::async_trait]
impl Tool for LargeEditCommitTool {
    fn name(&self) -> &str {
        "LargeEditCommit"
    }

    fn description(&self) -> &str {
        "Commit a staged large edit. Fails if the original file changed since LargeEditBegin, preserving the staged copy for review or abort."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "edit_id": { "type": "string" },
                "required_fragments": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional strings that must exist in the staged file before commit."
                }
            },
            "required": ["edit_id"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let edit_id = match string_field(&input, "edit_id") {
            Some(value) => value,
            None => return ToolResult::error("edit_id is required and must be a string"),
        };
        let required = string_array_field(&input, "required_fragments");
        tool_result(session::commit(&edit_id, ctx, &required))
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}

#[async_trait::async_trait]
impl Tool for LargeEditAbortTool {
    fn name(&self) -> &str {
        "LargeEditAbort"
    }

    fn description(&self) -> &str {
        "Abort a staged large edit and remove its temporary files without changing the target."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "edit_id": { "type": "string" } },
            "required": ["edit_id"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let edit_id = match string_field(&input, "edit_id") {
            Some(value) => value,
            None => return ToolResult::error("edit_id is required and must be a string"),
        };
        tool_result(session::abort(&edit_id, ctx))
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}

struct MutationArgs {
    edit_id: String,
    anchor: String,
    end_anchor: Option<String>,
    content: String,
    occurrence: usize,
}

fn mutation_args(input: &serde_json::Value, require_content: bool) -> Option<MutationArgs> {
    let anchor = string_field(input, "anchor").or_else(|| string_field(input, "start_anchor"))?;
    let content = string_field(input, "content").unwrap_or_default();
    if require_content && content.is_empty() {
        return None;
    }
    Some(MutationArgs {
        edit_id: string_field(input, "edit_id")?,
        anchor,
        end_anchor: string_field(input, "end_anchor"),
        content,
        occurrence: input
            .get("occurrence")
            .and_then(|value| value.as_u64())
            .unwrap_or(1)
            .max(1) as usize,
    })
}

fn mutation_schema(required: &[&str]) -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "edit_id": { "type": "string" },
            "anchor": { "type": "string", "description": "Anchor line to insert after." },
            "start_anchor": { "type": "string", "description": "Section start anchor." },
            "end_anchor": { "type": "string", "description": "Optional section end anchor. The end anchor line is preserved." },
            "content": { "type": "string", "description": "Replacement or inserted content chunk." },
            "occurrence": { "type": "integer", "description": "1-based anchor occurrence, default 1." }
        },
        "required": required
    })
}

fn tool_result(result: Result<String, String>) -> ToolResult {
    match result {
        Ok(message) => ToolResult::success(message),
        Err(err) => ToolResult::error(err),
    }
}

fn string_field(input: &serde_json::Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn string_array_field(input: &serde_json::Value, key: &str) -> Vec<String> {
    input
        .get(key)
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::Tool;
    use std::fs;

    fn ctx(dir: &tempfile::TempDir) -> ToolContext {
        ToolContext {
            working_dir: dir.path().to_path_buf(),
            session_id: "large-edit-test".into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn stages_replaces_and_commits_large_edit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.md");
        fs::write(&path, "# A\nold\n# B\nkeep\n").unwrap();
        let ctx = ctx(&dir);

        let begun = LargeEditBeginTool
            .execute(json!({ "file_path": path.display().to_string() }), &ctx)
            .await;
        assert!(!begun.is_error, "{}", begun.content);
        let edit_id = serde_json::from_str::<serde_json::Value>(&begun.content).unwrap()["edit_id"]
            .as_str()
            .unwrap()
            .to_string();

        let replaced = LargeEditReplaceSectionTool
            .execute(
                json!({ "edit_id": edit_id, "start_anchor": "# A", "content": "# A\nnew\n" }),
                &ctx,
            )
            .await;
        assert!(!replaced.is_error, "{}", replaced.content);

        let committed = LargeEditCommitTool
            .execute(
                json!({ "edit_id": edit_id, "required_fragments": ["new"] }),
                &ctx,
            )
            .await;
        assert!(!committed.is_error, "{}", committed.content);
        assert_eq!(fs::read_to_string(path).unwrap(), "# A\nnew\n# B\nkeep\n");
    }

    #[tokio::test]
    async fn commit_fails_when_target_changed_after_begin() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.md");
        fs::write(&path, "# A\nold\n").unwrap();
        let ctx = ctx(&dir);
        let begun = LargeEditBeginTool
            .execute(json!({ "file_path": path.display().to_string() }), &ctx)
            .await;
        let edit_id = serde_json::from_str::<serde_json::Value>(&begun.content).unwrap()["edit_id"]
            .as_str()
            .unwrap()
            .to_string();

        fs::write(&path, "# A\nchanged elsewhere\n").unwrap();
        let committed = LargeEditCommitTool
            .execute(json!({ "edit_id": edit_id }), &ctx)
            .await;
        assert!(committed.is_error);
        assert!(committed.content.contains("Target changed"));
    }
}
