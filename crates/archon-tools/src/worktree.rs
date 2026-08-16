use serde_json::json;

use crate::git::open_repo;
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};
use crate::worktree_manager::{ExitAction, WorktreeManager};

/// Tool to enter (create) a git worktree for isolated agent work.
pub struct EnterWorktreeTool;

#[async_trait::async_trait]
impl Tool for EnterWorktreeTool {
    fn name(&self) -> &str {
        "EnterWorktree"
    }

    fn description(&self) -> &str {
        "Create an isolated git worktree for the current session. \
         Work happens on a separate branch so the main working tree is unaffected."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "Optional description of what work will be done in this worktree"
                }
            },
            "required": []
        })
    }

    async fn execute(&self, _input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let repo = match open_repo(&ctx.working_dir) {
            Ok(r) => r,
            Err(e) => return ToolResult::error(format!("Cannot open git repo: {e}")),
        };

        // Keyed by the calling agent, not the session. `session_id` is copied
        // verbatim from parent to child, so it cannot separate one agent from
        // its siblings — every agent in a session resolved to one directory
        // and the second deleted the first's work (#184 M4).
        let owner_id =
            crate::worktree_ownership::owner_key_for(&ctx.session_id, ctx.subagent_id.as_deref());

        match WorktreeManager::create_worktree(&repo, &ctx.session_id, &owner_id) {
            Ok(info) => ToolResult::success(format!(
                "Worktree created.\n  Path: {}\n  Branch: {}\n  Owner: {}\n  Session: {}",
                info.worktree_path.display(),
                info.branch_name,
                info.owner_id,
                info.session_id,
            )),
            Err(e) => ToolResult::error(format!("Failed to create worktree: {e}")),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}

/// Tool to exit (close) a git worktree, optionally merging or discarding changes.
pub struct ExitWorktreeTool;

#[async_trait::async_trait]
impl Tool for ExitWorktreeTool {
    fn name(&self) -> &str {
        "ExitWorktree"
    }

    fn description(&self) -> &str {
        "Exit the current session worktree. Action can be 'merge' (integrate changes), \
         'keep' (leave worktree as-is), or 'discard' (delete worktree and branch). \
         Default is 'keep'."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["merge", "keep", "discard"],
                    "description": "What to do with the worktree: merge, keep, or discard (default: keep)"
                }
            },
            "required": []
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let action_str = input
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("keep");

        let action = match action_str {
            "merge" => ExitAction::Merge,
            "keep" => ExitAction::Keep,
            "discard" => ExitAction::Discard,
            other => {
                return ToolResult::error(format!(
                    "Invalid action '{other}'. Must be 'merge', 'keep', or 'discard'."
                ));
            }
        };

        let repo = match open_repo(&ctx.working_dir) {
            Ok(r) => r,
            Err(e) => return ToolResult::error(format!("Cannot open git repo: {e}")),
        };

        // Resolve by OWNER, not session. Matching on `session_id` handed a
        // subagent its parent's worktree — because subagent trees are filed
        // under a different key, the lookup skipped past its own and found the
        // parent's — so `discard` destroyed the parent's work (#184 M4).
        let owner_id =
            crate::worktree_ownership::owner_key_for(&ctx.session_id, ctx.subagent_id.as_deref());

        let info = match WorktreeManager::find_by_owner(&owner_id) {
            Some(i) => i,
            None => {
                return ToolResult::error(format!(
                    "No active worktree found for agent '{owner_id}'"
                ));
            }
        };

        match WorktreeManager::exit_worktree(&repo, &info, action) {
            Ok(msg) => ToolResult::success(msg),
            Err(e) => ToolResult::error(format!("Failed to exit worktree: {e}")),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}
