//! How an executor a resume replaced ends (Issue 329).
//!
//! A resume hands a run to a newer executor while an older one may still be
//! unwinding. The older one's control refusal is not a pause or a cancel of
//! the run -- the run goes on under the newer executor -- so it must not say
//! "cancelled". It says that it lost ownership and changed nothing after
//! that, which every write fence (`control_pause::require_executor`) makes
//! true.
//! Shared by the generated and the fixed decomposition executors.

use super::*;

/// The refusal when the executor launched at `generation` no longer owns
/// `run_id`, by the shared rule; `None` while it does. An unreadable state
/// proves nothing: `None`, and the caller keeps its ordinary ending.
pub(in super::super) fn lost_ownership(
    store: &WorkflowStore,
    run_id: &str,
    generation: u64,
) -> Option<WorkflowError> {
    let run = store.load_state(run_id).ok()?;
    archon_workflow::control_pause::require_executor(&run, generation).err()
}

/// What a replaced executor of a `label` run reports, with the refusal or
/// control message that stopped it.
pub(in super::super) fn lost_ownership_report(label: &str, run_id: &str, message: &str) -> String {
    format!(
        "{label} {run_id}: this session lost ownership of the run to a newer executor and changed nothing after that; the newer executor keeps the run.\n{message}\n"
    )
}
