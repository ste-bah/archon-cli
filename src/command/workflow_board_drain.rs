// The task board, as `archon_workflow`'s drain gate needs it.
//
// `archon-workflow` declares `WorkflowBoardPort` and deliberately does not
// depend on `archon-memory`: doing it for one read at one barrier would put
// CozoDB, fastembed and a blocking HTTP client into the dependency graph of
// every consumer of the workflow runtime. The adapter therefore lives in the
// binary, which already depends on both — the same shape as the LLM client and
// the script host.
//
// WHY THE HANDLE COMES FROM THE PROCESS-GLOBAL AND NOT FROM A PARAMETER
//
// The gate has to read the board the board TOOLS wrote to. Those resolve
// `archon_tools::board::BoardHandle::Global`, installed once at session boot
// from the session's own `MemoryAccess` — direct in the process that owns the
// CozoDB writer, over the socket in every other. Threading a second handle down
// through `run_generated_v2_workflow` would be a second answer to "which board",
// and the failure when the two disagreed would be a gate reading an empty
// partition and passing the run. There is exactly one board per process, so
// there is exactly one place to ask.

use std::sync::Arc;

use archon_memory::board::{BoardAccess, BoardItem, BoardItemKind, BoardStatus};
use archon_workflow::{
    DrainItem, DrainItemKind, DrainStatus, WorkflowBoardPort, WorkflowError, WorkflowResult,
};

/// `WorkflowBoardPort` over `archon_memory::BoardAccess`.
pub(crate) struct MemoryBoardDrain {
    board: Arc<dyn BoardAccess>,
}

impl MemoryBoardDrain {
    pub(crate) fn new(board: Arc<dyn BoardAccess>) -> Self {
        Self { board }
    }
}

impl WorkflowBoardPort for MemoryBoardDrain {
    fn drain_items_for_run(&self, run_id: &str) -> WorkflowResult<Vec<DrainItem>> {
        // Every status, not just the open ones. The gate reports what it
        // inspected and how each item ended, and a filter here would make an
        // empty board and a fully drained one indistinguishable in the record.
        let items = self
            .board
            .list_board_items_by_run(run_id, &[])
            .map_err(|error| {
                WorkflowError::port(format!(
                    "reading the task board for run {run_id} failed: {error}"
                ))
            })?;
        Ok(items.iter().map(drain_item).collect())
    }
}

/// Project a stored row onto the four fields the gate judges.
///
/// A total match on both enums rather than a string round-trip: a status added
/// to the board and not to the port stops this compiling, where a `_ =>` arm
/// would quietly classify it as whatever was convenient and let a run through.
fn drain_item(item: &BoardItem) -> DrainItem {
    DrainItem {
        id: item.id.clone(),
        title: item.title.clone(),
        kind: match item.kind {
            BoardItemKind::Issue => DrainItemKind::Issue,
            BoardItemKind::Note => DrainItemKind::Note,
        },
        status: match item.status {
            BoardStatus::Open => DrainStatus::Open,
            BoardStatus::Claimed => DrainStatus::Claimed,
            BoardStatus::InReview => DrainStatus::InReview,
            BoardStatus::GapsRemain => DrainStatus::GapsRemain,
            BoardStatus::Resolved => DrainStatus::Resolved,
            BoardStatus::Declined => DrainStatus::Declined,
            BoardStatus::Promoted => DrainStatus::Promoted,
            BoardStatus::Escalated => DrainStatus::Escalated,
        },
        decline_reason: item.decline_reason.clone(),
    }
}

/// The board this process drains against, if it has one.
///
/// `None` when no memory service is open — a subcommand that builds no session,
/// a test binary, an early failure before session boot, or a session whose
/// memory would not open. `--print` and `--headless` are no longer on that list:
/// they install a handle in `src/session/build_agent_board.rs` like the TUI
/// does (#137). The driver treats a missing board as "no board configured" and
/// passes, which is correct: the gate asserts that a board which exists was
/// drained, and inventing a failure for a runtime that never had one would make
/// the board mandatory by accident. A board that exists and cannot be READ is a
/// different thing entirely, and fails.
pub(crate) fn process_board_drain() -> Option<Arc<dyn WorkflowBoardPort>> {
    match archon_tools::board::BoardHandle::Global.resolve() {
        Ok(board) => Some(Arc::new(MemoryBoardDrain::new(board)) as Arc<dyn WorkflowBoardPort>),
        Err(reason) => {
            tracing::debug!(%reason, "no task board in this process; the drain gate will not run");
            None
        }
    }
}

#[cfg(test)]
#[path = "workflow_board_drain_tests.rs"]
mod tests;
