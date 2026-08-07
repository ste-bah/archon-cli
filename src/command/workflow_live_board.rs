//! The task board handle for a standalone `archon workflow` run.
//!
//! `archon workflow` is a subcommand: `main_modes::handle_subcommand_if_present`
//! dispatches it and returns from `main`, so the process never builds a session
//! and neither `src/session/build_agent_board.rs` nor `interactive_bootstrap`
//! ever runs. `BoardHandle::Global` therefore resolved to nothing here — the
//! four board tools answered *"the task board is unavailable"* to every stage
//! subagent, and, worse, the lifecycle's drain gate read `None` and passed the
//! run without inspecting anything (#142).
//!
//! The second half is why this file exists rather than a bug report. A gate that
//! reports a clean run because it cannot see the board is worse than no gate at
//! all: it produces the exact record an enforced run produces. The companion
//! change is in `workflow_board_drain.rs`, which no longer lets an absent board
//! read as an empty one.

use archon_core::config::ArchonConfig;

/// Open this run's memory and put a real handle behind the board tools.
///
/// The body is `crate::command::board_access`, shared with the print/headless
/// path since #146 — including the already-installed guard that started here,
/// which the test binary needs because it can reach every install site in one
/// process.
///
/// What is specific to a workflow run is the cost of failure. Nothing here is
/// fatal to the *run*, but it is no longer free of consequence: a run that
/// reaches the drain gate with no board is refused rather than accepted, so the
/// warning says so. The election matters most on this path for a mundane
/// reason — it is normal to start a workflow from a terminal while a TUI
/// session holds the same database in another.
pub(crate) async fn install_workflow_board_access(config: &ArchonConfig) {
    crate::command::board_access::install_process_board_access(
        config,
        "this workflow run has no task board and its drain gate will refuse it",
    )
    .await;
}
