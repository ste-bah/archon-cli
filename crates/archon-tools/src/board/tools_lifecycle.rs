//! `BoardClaim` and `BoardResolve` — the two tools that move an item through
//! its lifecycle.
//!
//! Both go through the storage layer's compare-and-set, and both therefore have
//! an outcome the caller cannot infer from the absence of an error: the write
//! may simply not have applied, because somebody else got there first. Reporting
//! that honestly — and naming who won — is most of what these two do.

use std::sync::Arc;

use archon_memory::board::{BoardAccess, BoardStatus, BoardUpdate};

use super::tools::{access, render, required_str, sweep};
use super::{BoardHandle, caller_id, run_id_for_session};
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// BoardClaim
// ---------------------------------------------------------------------------

/// Take ownership of an open board item.
pub struct BoardClaimTool {
    handle: BoardHandle,
}

impl BoardClaimTool {
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

impl Default for BoardClaimTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Tool for BoardClaimTool {
    fn name(&self) -> &str {
        "BoardClaim"
    }

    fn description(&self) -> &str {
        "Claim a board item before working on it, so no two agents do the same \
         work. Exactly one caller wins a contested claim; if you lose, the result \
         names the agent that holds it and you should pick a different item."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Board item id, as returned by BoardList" }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let board = match access(&self.handle) {
            Ok(board) => board,
            Err(result) => return result,
        };
        let id = match required_str(&input, "id") {
            Ok(value) => value,
            Err(result) => return result,
        };
        // An item held by an agent that has since died is available, and the
        // claimant is the one who cares. Sweep before trying rather than
        // refusing a claim on a lease nobody is holding.
        sweep(board.as_ref(), run_id_for_session(&ctx.session_id));

        let agent_id = caller_id(ctx);
        match board.claim_board_item(id, &agent_id) {
            Ok(BoardUpdate {
                applied: true,
                item,
            }) => ToolResult::success(format!(
                "Claimed [{}] as {}.\n{}",
                item.id,
                agent_id,
                render(&item)
            )),
            Ok(BoardUpdate {
                applied: false,
                item,
            }) => {
                // The CAS hands back the authoritative row, so the holder is
                // reported from the same transaction that refused the claim
                // rather than from a follow-up read that could disagree.
                let holder = item.claimed_by.as_deref().unwrap_or("(nobody)");
                ToolResult::error(format!(
                    "claim refused: [{}] is held by {holder} (status {}). Pick a different item.",
                    item.id, item.status
                ))
            }
            Err(error) => ToolResult::error(format!("failed to claim board item: {error}")),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

// ---------------------------------------------------------------------------
// BoardResolve
// ---------------------------------------------------------------------------

/// Close a board item out as resolved or declined.
pub struct BoardResolveTool {
    handle: BoardHandle,
}

impl BoardResolveTool {
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

impl Default for BoardResolveTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Tool for BoardResolveTool {
    fn name(&self) -> &str {
        "BoardResolve"
    }

    fn description(&self) -> &str {
        "Close a board item as 'resolved' (the work is done) or 'declined' (it \
         should not be done). A reason is required either way — an item that \
         disappears without one is indistinguishable from one that was dropped."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Board item id" },
                "outcome": {
                    "type": "string",
                    "enum": ["resolved", "declined"],
                    "description": "'resolved' if the work is done, 'declined' if it should not be"
                },
                "reason": {
                    "type": "string",
                    "description": "REQUIRED. Why this item is being closed, with evidence."
                }
            },
            "required": ["id", "outcome", "reason"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let board = match access(&self.handle) {
            Ok(board) => board,
            Err(result) => return result,
        };
        let id = match required_str(&input, "id") {
            Ok(value) => value,
            Err(result) => return result,
        };
        let Ok(reason) = required_str(&input, "reason") else {
            return ToolResult::error(
                "reason is required: say why the item is being closed, with evidence.",
            );
        };
        let target = match input.get("outcome").and_then(|value| value.as_str()) {
            Some("resolved") => BoardStatus::Resolved,
            Some("declined") => BoardStatus::Declined,
            Some(other) => {
                return ToolResult::error(format!(
                    "unknown outcome '{other}': use 'resolved' or 'declined'"
                ));
            }
            None => return ToolResult::error("outcome is required: 'resolved' or 'declined'"),
        };

        // The transition is a compare-and-set on the status the caller last
        // saw, so it needs that status. Read it now: if another agent moves the
        // item between this read and the write, the CAS refuses and we report
        // the row it refused against instead of overwriting a verdict we never
        // saw.
        let current = match board.get_board_item(id) {
            Ok(item) => item,
            Err(error) => return ToolResult::error(format!("no such board item {id}: {error}")),
        };
        if current.status == target {
            return ToolResult::success(format!("[{id}] is already {target}; nothing to do."));
        }

        let closed_by = caller_id(ctx);
        // A decline goes through its own call because the storage layer refuses
        // one with no reason behind it -- the drain gate will not accept a
        // declined item it cannot explain, so the reason has to be durable
        // rather than only logged here.
        let transition = match target {
            BoardStatus::Declined => board.decline_board_item(id, current.status, reason),
            _ => board.set_board_item_status(id, current.status, target),
        };
        match transition {
            Ok(BoardUpdate {
                applied: true,
                item,
            }) => {
                // A resolve's reason is still only logged: the durable column
                // exists for declines, which is where the drain gate needs it.
                tracing::info!(
                    item_id = %item.id,
                    outcome = %target,
                    %closed_by,
                    reason,
                    "board item closed"
                );
                ToolResult::success(format!(
                    "[{}] is now {target} (by {closed_by}): {reason}\n{}",
                    item.id,
                    render(&item)
                ))
            }
            Ok(BoardUpdate {
                applied: false,
                item,
            }) => ToolResult::error(format!(
                "[{}] moved to {} before this call landed; it was not set to {target}.",
                item.id, item.status
            )),
            Err(error) => ToolResult::error(format!("failed to close board item: {error}")),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
