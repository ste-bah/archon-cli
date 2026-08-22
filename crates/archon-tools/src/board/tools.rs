//! `BoardRaise` and `BoardList`, plus the helpers both halves of the tool
//! surface share. The two compare-and-set transitions live next door in
//! [`super::tools_lifecycle`].
//!
//! See the module docs in `board.rs` for why attribution comes from the
//! context rather than from a tool argument.

use std::sync::Arc;

use archon_memory::board::{BoardAccess, BoardItem, BoardItemKind, BoardStatus, NewBoardItem};

use super::{BoardHandle, caller_id, leases, run_id_for_session};
use crate::tool::{
    PermissionLevel, Tool, ToolCapability, ToolContext, ToolResult, WorkingTreeEffect,
};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

pub(super) fn access(handle: &BoardHandle) -> Result<Arc<dyn BoardAccess>, ToolResult> {
    handle.resolve().map_err(ToolResult::error)
}

/// One block per item, dense enough to read a whole board in a tool result.
pub(super) fn render(item: &BoardItem) -> String {
    let holder = match &item.claimed_by {
        Some(agent) => format!(" claimed_by={agent}"),
        None => String::new(),
    };
    format!(
        "[{}] {} {} round={}{}\n  title: {}\n  evidence: {}\n  acceptance: {}\n  raised_by: {}",
        item.id,
        item.kind,
        item.status,
        item.round,
        holder,
        item.title,
        item.evidence,
        if item.acceptance.is_empty() {
            "(none)"
        } else {
            &item.acceptance
        },
        item.raised_by,
    )
}

pub(super) fn required_str<'a>(
    input: &'a serde_json::Value,
    field: &str,
) -> Result<&'a str, ToolResult> {
    match input.get(field).and_then(|value| value.as_str()) {
        Some(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(ToolResult::error(format!(
            "{field} is required and must be non-empty"
        ))),
    }
}

/// Take back claims whose holders have died, before reading or claiming.
///
/// Run inline rather than on a timer: a stale claim only matters at the moment
/// someone looks at the board, and the check is a couple of in-memory registry
/// lookups per claimed item. A failure here is logged and swallowed — a board
/// that cannot be tidied is still a board that can be read.
pub(super) fn sweep(board: &dyn BoardAccess, run_id: &str) {
    if let Err(error) = leases::release_dead_claims(board, run_id) {
        tracing::warn!(%error, run_id, "board lease sweep failed; continuing with stale claims");
    }
}

// ---------------------------------------------------------------------------
// BoardRaise
// ---------------------------------------------------------------------------

/// Raise an issue or a note on the run's board.
pub struct BoardRaiseTool {
    handle: BoardHandle,
}

impl BoardRaiseTool {
    pub fn new() -> Self {
        Self {
            handle: BoardHandle::Global,
        }
    }

    pub fn with_access(access: Arc<dyn BoardAccess>) -> Self {
        Self {
            handle: BoardHandle::Direct(access),
        }
    }
}

impl Default for BoardRaiseTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Tool for BoardRaiseTool {
    fn name(&self) -> &str {
        "BoardRaise"
    }

    fn capability(&self) -> ToolCapability {
        ToolCapability::HostLocal
    }

    fn description(&self) -> &str {
        "Raise an item on this run's task board so another agent can pick it up. \
         Use kind='issue' for work that must happen and kind='note' for context \
         the next agent touching this area needs. Evidence is required: give \
         file:line references and what you actually observed, because whoever \
         claims the item cannot ask you afterwards."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["issue", "note"],
                    "description": "'issue' is work that must happen; 'note' is context. Defaults to 'issue'."
                },
                "title": { "type": "string", "description": "One line naming the finding" },
                "evidence": {
                    "type": "string",
                    "description": "REQUIRED. file:line references and what was observed."
                },
                "acceptance": {
                    "type": "string",
                    "description": "What 'done' means for this item. Strongly recommended for issues."
                }
            },
            "required": ["title", "evidence"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let board = match access(&self.handle) {
            Ok(board) => board,
            Err(result) => return result,
        };
        let title = match required_str(&input, "title") {
            Ok(value) => value,
            Err(result) => return result,
        };
        // Checked here as well as in storage so the agent gets a sentence it can
        // act on rather than a database error it has to decode.
        let Ok(evidence) = required_str(&input, "evidence") else {
            return ToolResult::error(
                "evidence is required: give file:line references and what you observed. \
                 An item without them cannot be acted on by whoever claims it.",
            );
        };
        let kind = match input.get("kind").and_then(|value| value.as_str()) {
            Some("note") => BoardItemKind::Note,
            Some("issue") | None => BoardItemKind::Issue,
            Some(other) => {
                return ToolResult::error(format!("unknown kind '{other}': use 'issue' or 'note'"));
            }
        };

        let item = NewBoardItem {
            id: None,
            run_id: run_id_for_session(&ctx.session_id).to_string(),
            kind,
            title: title.to_string(),
            evidence: evidence.to_string(),
            acceptance: input
                .get("acceptance")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            raised_by: caller_id(ctx),
        };

        match board.create_board_item(&item) {
            Ok(stored) => ToolResult::success(format!(
                "Raised {} [{}] on run {} as {}.\n{}",
                stored.kind,
                stored.id,
                stored.run_id,
                stored.raised_by,
                render(&stored)
            )),
            Err(error) => ToolResult::error(format!("failed to raise board item: {error}")),
        }
    }

    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::ExternalOnly
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

// ---------------------------------------------------------------------------
// BoardList
// ---------------------------------------------------------------------------

/// Read this run's board, optionally filtered by status.
pub struct BoardListTool {
    handle: BoardHandle,
}

impl BoardListTool {
    pub fn new() -> Self {
        Self {
            handle: BoardHandle::Global,
        }
    }

    pub fn with_access(access: Arc<dyn BoardAccess>) -> Self {
        Self {
            handle: BoardHandle::Direct(access),
        }
    }
}

impl Default for BoardListTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Tool for BoardListTool {
    fn name(&self) -> &str {
        "BoardList"
    }

    fn capability(&self) -> ToolCapability {
        ToolCapability::HostLocal
    }

    fn description(&self) -> &str {
        "List the task board for this run, oldest first. Filter with `status` to \
         find work: 'open' items are unclaimed and available. Claims held by \
         agents that are no longer running are released before the listing is \
         produced, so a claimed item really does have a live owner."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["open", "claimed", "in_review", "gaps_remain",
                                 "resolved", "declined", "promoted", "escalated"]
                    },
                    "description": "Statuses to include. Omit for every item on the run."
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let board = match access(&self.handle) {
            Ok(board) => board,
            Err(result) => return result,
        };
        let run_id = run_id_for_session(&ctx.session_id);

        let mut statuses = Vec::new();
        if let Some(values) = input.get("status").and_then(|value| value.as_array()) {
            for value in values {
                let raw = value.as_str().unwrap_or_default();
                match BoardStatus::from_str_opt(raw) {
                    Some(status) => statuses.push(status),
                    None => return ToolResult::error(format!("unknown status '{raw}'")),
                }
            }
        }

        sweep(board.as_ref(), run_id);

        match board.list_board_items_by_run(run_id, &statuses) {
            Ok(items) if items.is_empty() => {
                ToolResult::success(format!("No board items for run {run_id}."))
            }
            Ok(items) => {
                let rendered: Vec<String> = items.iter().map(render).collect();
                ToolResult::success(format!(
                    "{} board item(s) for run {run_id}:\n\n{}",
                    items.len(),
                    rendered.join("\n\n")
                ))
            }
            Err(error) => ToolResult::error(format!("failed to list board items: {error}")),
        }
    }

    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::ExternalOnly
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
