//! Is the audit state readable, and does it still belong to this run?
//!
//! Two questions that look alike and are not, which is how Issue-83 happened:
//! the runtime asked them as one comparison and reported both answers as
//! corruption.
//!
//! - Readability is a fact about the FILE. A truncated write, a field this
//!   build does not know, a schema it cannot read: the state is corrupt, and
//!   nothing downstream may proceed on it.
//! - Identity is a fact about CONTROL. Every lifecycle action a run takes —
//!   pause, resume, restart, force-accept — bumps the run's generation, and
//!   the audit state is stamped with the generation it was established at. A
//!   difference says the run moved on, which is a reason to stop and be
//!   re-dispatched, not a reason to declare the state broken and fail the
//!   stage that happened to ask.
//!
//! Identity is therefore measured against the run AS IT IS NOW, never against
//! a generation snapshotted when some long-lived handle was built. A handle
//! outlives any number of lifecycle actions, so a snapshot goes stale for
//! ordinary reasons; a stale snapshot is not evidence of anything being
//! wrong. The finalizer already compares the state against the run's current
//! generation this way.

use super::runtime::{AuditState, STATE_PATH};
use crate::{WorkflowError, WorkflowResult, WorkflowStore};

/// The audit state as written, with every way it can be unreadable reported
/// as corruption — including a parse failure, which used to surface as a bare
/// serialiser message naming neither the run nor the file.
pub(super) fn read_state(store: &WorkflowStore, run_id: &str) -> WorkflowResult<AuditState> {
    let path = store.run_dir(run_id).join(STATE_PATH);
    let raw = std::fs::read(&path).map_err(|e| WorkflowError::io(&path, e))?;
    let state: AuditState = serde_json::from_slice(&raw).map_err(|error| {
        WorkflowError::StateCorrupt(format!(
            "repository audit state for run {run_id} is unreadable: {error}"
        ))
    })?;
    if state.schema_version != 1 {
        return Err(WorkflowError::StateCorrupt(
            "unsupported audit state schema".into(),
        ));
    }
    Ok(state)
}

/// Refuse a state the run has moved past, as the control condition that ends
/// the executor cleanly and invites a re-dispatch — never as corruption,
/// which is terminal and is charged to whatever budget the asking stage runs
/// under.
pub(super) fn require_current_generation(
    store: &WorkflowStore,
    run_id: &str,
    state: &AuditState,
) -> WorkflowResult<()> {
    if state.generation != store.load_state(run_id)?.generation {
        return Err(WorkflowError::ControlPaused(
            "repository audit generation superseded".into(),
        ));
    }
    Ok(())
}
