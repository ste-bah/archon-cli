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
//!   difference only invalidates the audit when control stopped the run or
//!   replaced its executor. Edits on a running run keep ownership and the
//!   audit snapshot. A superseded execution is a reason to stop and be
//!   re-dispatched, not a reason to declare the state broken and fail the
//!   stage that happened to ask.
//!
//! The state must belong to the current run, AND the handle must belong to
//! the captured executor. A new executor reinitializing shared state never
//! grants an older handle authority to dispatch or write that state.

use super::runtime::{AuditState, STATE_PATH};
use crate::{RunStatus, WorkflowError, WorkflowResult, WorkflowStore};

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
    let run = store.load_state(run_id)?;
    if state.generation != run.generation
        && (matches!(run.status, RunStatus::Paused | RunStatus::Cancelled)
            || !run.execution_owned_at(state.generation))
    {
        return Err(WorkflowError::ControlPaused(
            "repository audit generation superseded".into(),
        ));
    }
    Ok(())
}

impl super::runtime::AuditRuntime {
    /// Capture audit artifacts while this handle still owns the run. The
    /// capture's git work holds no run lock; ownership is checked in short
    /// critical sections before and after it (Issue 291).
    pub fn capture_snapshot(
        &self,
        root: &std::path::Path,
        paths: &[String],
        store: &crate::WorkflowV2ResultStore,
    ) -> WorkflowResult<super::runtime::Snapshot> {
        self.fenced_snapshot(root, || {
            super::runtime::Snapshot::capture(root, paths, store)
        })
    }
    /// A view of already captured wave source, fenced the same way.
    pub fn snapshot_from_sealed(
        &self,
        root: &std::path::Path,
        source: &crate::write_coordinator::worktree_isolation::SealedSource,
        plan: &crate::write_coordinator::WritePlan,
        store: &crate::WorkflowV2ResultStore,
    ) -> WorkflowResult<super::runtime::Snapshot> {
        self.fenced_snapshot(root, || {
            super::runtime::Snapshot::from_sealed(root, source, plan, store)
        })
    }
    fn fenced_snapshot(
        &self,
        root: &std::path::Path,
        capture: impl FnOnce() -> WorkflowResult<super::runtime::Snapshot>,
    ) -> WorkflowResult<super::runtime::Snapshot> {
        self.with_executor_lock(|| Ok(()))?;
        let snapshot = capture()?;
        if let Err(refused) = self.with_executor_lock(|| Ok(())) {
            // Not this handle's to publish: the private view goes too.
            super::snapshot::discard(root, &snapshot.root);
            return Err(refused);
        }
        Ok(snapshot)
    }
    pub(super) fn require_executor(&self) -> WorkflowResult<()> {
        crate::control_pause::require_executor(
            &self.store.load_state(&self.run_id)?,
            self.generation,
        )
    }
    pub(super) fn require_active_executor(&self) -> WorkflowResult<()> {
        crate::control_pause::PauseOwner::Executor(self.generation)
            .require_pauser(&self.store.load_state(&self.run_id)?)
    }
    pub(in crate::repository_audit) fn with_executor_lock<T>(
        &self,
        operation: impl FnOnce() -> WorkflowResult<T>,
    ) -> WorkflowResult<T> {
        self.store.with_run_lock(&self.run_id, |_| operation())
    }
}
