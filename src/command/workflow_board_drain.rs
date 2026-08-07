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

/// The port handed to the gate when this process installed no board.
///
/// A port that refuses, not the absence of one. `LifecycleDriver` distinguishes
/// three cases and only two of them are honest here: a board that reads clean
/// passes, a board that cannot be read fails, and *no board configured* passes
/// on the grounds that a runtime which never had a board should not be forced
/// to have one. That last exemption is right for the crate — `LifecycleDriver`
/// has consumers with no memory at all — and wrong for this binary, where every
/// production entry point installs a board and its absence means something
/// broke. #142 is what that costs: a standalone `archon workflow` run installed
/// nothing, the gate read `None`, and every such run reported a drained board it
/// had never looked at.
///
/// So the composition root always names a board, and one it cannot reach says
/// so. The reason travels into the run's `blocked-board-drain` record, which is
/// the difference between "this run left no gaps" and "nobody checked".
struct UnreachableBoardDrain {
    reason: String,
}

impl WorkflowBoardPort for UnreachableBoardDrain {
    fn drain_items_for_run(&self, run_id: &str) -> WorkflowResult<Vec<DrainItem>> {
        Err(WorkflowError::port(format!(
            "run {run_id} has no task board in this process, so its completion could not be \
             checked: {}",
            self.reason
        )))
    }
}

/// The board this process drains against — always a port, never nothing.
///
/// The global resolves for every surface that boots one: the TUI, `--print` and
/// `--headless` through `src/session/build_agent_board.rs` (#137), and a
/// standalone `archon workflow` through `workflow_live_board.rs` (#142). It does
/// not resolve in a test binary that installed no board, or after a session
/// whose memory would not open — and those runs now fail their drain gate with
/// the reason attached, rather than passing it in silence.
pub(crate) fn process_board_drain() -> Arc<dyn WorkflowBoardPort> {
    match archon_tools::board::BoardHandle::Global.resolve() {
        Ok(board) => Arc::new(MemoryBoardDrain::new(board)) as Arc<dyn WorkflowBoardPort>,
        Err(reason) => {
            tracing::warn!(
                %reason,
                "no task board in this process; the drain gate will refuse the run"
            );
            Arc::new(UnreachableBoardDrain { reason }) as Arc<dyn WorkflowBoardPort>
        }
    }
}

#[cfg(test)]
#[path = "workflow_board_drain_tests.rs"]
mod tests;
