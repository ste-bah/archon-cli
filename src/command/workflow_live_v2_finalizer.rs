//! Central durable terminal finalization for generated v3 runs.
//!
//! Script hosts return summaries. This composition boundary alone persists the
//! terminal run projection, appends the stable terminal event and records its
//! commit. An eligible run-end observer runs BEFORE that commit (ACC-A9,
//! `heal`): a failed observation re-enters acceptance or blocks the run by
//! name, and the committed outcome is the one the observation left. Only a
//! record an older binary committed with its observation still pending is
//! observed after the commit, recording the result beside the outcome.

use std::path::Path;

use archon_workflow::{
    AuthoredAcceptanceGateV1, FinalizationRecordV1, RunEndAcceptanceObserverSnapshotV1,
    RunEndObserverOutcomeV1, RunEndObserverStateV1, RunStatus, WorkflowError, WorkflowEventKind,
    WorkflowEventLog, WorkflowResult, WorkflowRunKind, WorkflowStore, WorkflowV2ResultStore,
    WorkflowV2Status,
};

use super::workflow_live_v2_script::WorkflowV2ScriptSummary;
#[path = "workflow_repository_audit_finalizer.rs"]
mod audit_finalizer;
#[path = "workflow_live_v2_finalizer_heal.rs"]
mod heal;
#[path = "workflow_finalization_identity.rs"]
mod identity;
#[path = "workflow_run_end_reobserve.rs"]
pub(crate) mod reobserve;
pub(super) use heal::{Reopened, RunEndReopen};

pub(super) const FINALIZATION_RECORD_PATH: &str = "v2/finalization.json";

pub(super) struct RunEndObserverContext<'a> {
    pub(super) run_id: &'a str,
    pub(super) terminal_status: WorkflowV2Status,
    pub(super) snapshot: &'a RunEndAcceptanceObserverSnapshotV1,
    /// The terminal outcome is not committed yet: the finalizer observes
    /// before it commits. False for an observation of a committed outcome.
    pub(super) pre_commit: bool,
}

#[async_trait::async_trait]
pub(super) trait WorkflowRunEndObserver: Send + Sync {
    async fn observe_async(
        &self,
        context: &RunEndObserverContext<'_>,
    ) -> WorkflowResult<RunEndObserverOutcomeV1> {
        self.observe(context)
    }
    fn observe(
        &self,
        context: &RunEndObserverContext<'_>,
    ) -> WorkflowResult<RunEndObserverOutcomeV1>;
}

pub(super) async fn finalize_summary(
    store: &WorkflowStore,
    run_id: &str,
    run_kind: WorkflowRunKind,
    snapshot: Option<RunEndAcceptanceObserverSnapshotV1>,
    summary: &WorkflowV2ScriptSummary,
    v2_store: &WorkflowV2ResultStore,
    observer: Option<&dyn WorkflowRunEndObserver>,
    expected_generation: Option<u64>,
) -> WorkflowResult<()> {
    finalize_summary_with_gate(
        store,
        run_id,
        run_kind,
        snapshot,
        summary,
        v2_store,
        observer,
        expected_generation,
        None,
        None,
    )
    .await
    .map(|_| ())
}

/// `finalize_summary` with the authored lifecycle's acceptance gate (Obs-32)
/// stamped on the terminal record, and the acceptance stage a failed
/// pre-commit observation re-enters. The record refuses a completing status
/// beside a failing gate. Returns the committed summary: the post-observation
/// outcome, which re-entered acceptance or a standing observation failure may
/// have changed.
#[allow(clippy::too_many_arguments)]
pub(super) async fn finalize_summary_with_gate(
    store: &WorkflowStore,
    run_id: &str,
    run_kind: WorkflowRunKind,
    mut snapshot: Option<RunEndAcceptanceObserverSnapshotV1>,
    summary: &WorkflowV2ScriptSummary,
    v2_store: &WorkflowV2ResultStore,
    observer: Option<&dyn WorkflowRunEndObserver>,
    expected_generation: Option<u64>,
    acceptance_gate: Option<AuthoredAcceptanceGateV1>,
    reopen: Option<&dyn RunEndReopen>,
) -> WorkflowResult<WorkflowV2ScriptSummary> {
    let mut gated = audit_finalizer::gate(store, run_id, summary)?;
    let persisted = read_persisted_record(store, run_id)?;
    let disposition = identity::summary_disposition(persisted.as_ref(), run_kind, gated.status)?;
    // A resumed run that ends on the same resumable status after a new
    // acceptance round decided a new outcome: it supersedes, never replays
    // (a run blocked by a standing observation failure resumes this way).
    let disposition = match persisted.as_ref() {
        Some(record)
            if disposition == identity::Disposition::Replay
                && record.terminal_event_committed
                && !record.is_completing()
                && acceptance_gate.is_some()
                && record.acceptance_gate != acceptance_gate =>
        {
            identity::Disposition::Commit
        }
        _ => disposition,
    };
    if disposition == identity::Disposition::Commit
        && let Some(native) = snapshot.as_mut().and_then(|s| s.native_execution.as_mut())
    {
        *native = match crate::command::acceptance_scratch_policy::record_final_source(
            store, run_id, native,
        ) {
            Ok(binding) => binding,
            Err(error) => serde_json::json!({"capture_error":error.to_string()}),
        };
    }
    // A superseding call writes its own record: the outcome it carries has
    // never been committed, whatever an earlier resume point recorded.
    let mut record = match (disposition, persisted) {
        (identity::Disposition::Replay, Some(record)) => record,
        _ => {
            let record = FinalizationRecordV1::new(run_kind, gated.status, snapshot);
            match acceptance_gate.clone() {
                Some(gate) => record.with_acceptance_gate(gate)?,
                None => record,
            }
        }
    };
    if acceptance_gate.is_some() && record.acceptance_gate != acceptance_gate {
        return Err(WorkflowError::StateCorrupt(format!(
            "acceptance gate changed during terminal finalization of run {run_id}"
        )));
    }

    if !record.terminal_event_committed {
        if record.observer_state == Some(RunEndObserverStateV1::Pending)
            && let Some(observer) = observer
        {
            let pre_commit = heal::PreCommit {
                store,
                run_id,
                expected_generation,
                observer,
                reopen,
            };
            gated = heal::observe_before_commit(&pre_commit, gated, &mut record).await?;
        }
        let summary = &gated;
        store.with_run_lock(run_id, |locked| {
            require_generation_owner(locked, run_id, expected_generation)?;
            archon_workflow::poll_v2_run_control(locked, run_id, "finalization")?;
            let checked = audit_finalizer::gate(locked, run_id, summary)?;
            if checked.status != summary.status {
                return Err(WorkflowError::StateCorrupt(
                    "repository audit changed during terminal finalization".into(),
                ));
            }
            archon_workflow::v2::run_state_sync::sync_v2_summary_to_run(
                locked,
                run_id,
                &summary.calls,
                v2_store,
                summary.status,
            )?;
            locked.write_run_json(run_id, FINALIZATION_RECORD_PATH, &record)?;
            emit_terminal_event(locked, run_id, summary)?;
            record.mark_terminal_event_committed();
            locked.write_run_json(run_id, FINALIZATION_RECORD_PATH, &record)?;
            // The outcome the pre-commit re-entries were counted for is decided.
            heal::clear_reopen_ledger(locked, run_id)
        })?;
        return Ok(gated);
    }

    store.with_run_lock(run_id, |locked| {
        require_generation_owner(locked, run_id, expected_generation)?;
        archon_workflow::poll_v2_run_control(locked, run_id, "finalization")?;
        archon_workflow::v2::run_state_sync::sync_v2_summary_to_run(
            locked,
            run_id,
            &gated.calls,
            v2_store,
            record.terminal_v2_status.ok_or_else(|| {
                WorkflowError::StateCorrupt("summary finalization has no summary status".into())
            })?,
        )
    })?;

    // A record committed with its observation pending: written by an older
    // binary that observed after the commit. Its outcome is final; the
    // observation is finished beside it.
    let summary = &gated;
    if record.observer_state != Some(RunEndObserverStateV1::Pending) {
        return Ok(gated);
    }
    let Some(observer) = observer else {
        return Ok(gated);
    };
    let snapshot = record.observer_snapshot.as_ref().ok_or_else(|| {
        WorkflowError::StateCorrupt(
            "observer_pending finalization has no launch-time observer snapshot".to_string(),
        )
    })?;
    emit_observer_event(
        store,
        run_id,
        WorkflowEventKind::RunEndAcceptanceObserverStarted,
        "run_end_acceptance_observer_started",
        serde_json::json!({"authority": "observe_only"}),
    )?;
    let context = RunEndObserverContext {
        run_id,
        terminal_status: summary.status,
        snapshot,
        pre_commit: false,
    };
    match observer.observe_async(&context).await {
        Ok(outcome) => {
            record.complete_observer(outcome)?;
            store.with_run_lock(run_id, |locked| {
                require_generation_owner(locked, run_id, expected_generation)?;
                locked.write_run_json(run_id, FINALIZATION_RECORD_PATH, &record)
            })?;
        }
        Err(error) => {
            let reason = error.to_string();
            record.fail_observer(reason.clone())?;
            store.with_run_lock(run_id, |locked| {
                require_generation_owner(locked, run_id, expected_generation)?;
                locked.write_run_json(run_id, FINALIZATION_RECORD_PATH, &record)?;
                emit_observer_event(
                    locked,
                    run_id,
                    WorkflowEventKind::RunEndAcceptanceObserverFailed,
                    "run_end_acceptance_observer_failed",
                    serde_json::json!({"reason": reason}),
                )
            })?;
        }
    }
    Ok(gated)
}

pub(super) fn finalize_run_status(
    store: &WorkflowStore,
    run_id: &str,
    run_kind: WorkflowRunKind,
    status: RunStatus,
    detail: &str,
    expected_generation: Option<u64>,
) -> WorkflowResult<()> {
    let persisted = read_persisted_record(store, run_id)?;
    let disposition = identity::run_status_disposition(persisted.as_ref(), run_kind, &status)?;
    let mut record = match (disposition, persisted) {
        (identity::Disposition::Replay, Some(record)) => record,
        _ => FinalizationRecordV1::for_run_status(run_kind, status.clone()),
    };
    if record.terminal_event_committed {
        return reconcile_terminal_status(
            store,
            run_id,
            &record.terminal_status,
            expected_generation,
        );
    }
    store.with_run_lock(run_id, |locked| {
        require_generation_owner(locked, run_id, expected_generation)?;
        archon_workflow::v2::run_state_sync::persist_terminal_run_status(
            locked,
            run_id,
            status.clone(),
        )?;
        locked.write_run_json(run_id, FINALIZATION_RECORD_PATH, &record)?;
        emit_run_status_event(locked, run_id, &status, detail)?;
        record.mark_terminal_event_committed();
        locked.write_run_json(run_id, FINALIZATION_RECORD_PATH, &record)
    })
}

// Restore the committed projection without appending another terminal event.
fn reconcile_terminal_status(
    store: &WorkflowStore,
    run_id: &str,
    status: &RunStatus,
    expected_generation: Option<u64>,
) -> WorkflowResult<()> {
    store.with_run_lock(run_id, |locked| {
        require_generation_owner(locked, run_id, expected_generation)?;
        archon_workflow::v2::run_state_sync::persist_terminal_run_status(
            locked,
            run_id,
            status.clone(),
        )
    })
}

fn require_generation_owner(
    store: &WorkflowStore,
    run_id: &str,
    expected_generation: Option<u64>,
) -> WorkflowResult<()> {
    let Some(expected) = expected_generation else {
        return Ok(());
    };
    let current = store.load_state(run_id)?;
    if !current.execution_owned_at(expected) {
        return Err(WorkflowError::ControlCancelled(format!(
            "executor generation {expected} no longer owns run {run_id}; current generation is {}",
            current.generation
        )));
    }
    Ok(())
}

fn read_persisted_record(
    store: &WorkflowStore,
    run_id: &str,
) -> WorkflowResult<Option<FinalizationRecordV1>> {
    let path = store.run_dir(run_id).join(FINALIZATION_RECORD_PATH);
    if !path.exists() {
        return Ok(None);
    }
    read_record(&path).map(Some)
}

fn read_record(path: &Path) -> WorkflowResult<FinalizationRecordV1> {
    let bytes = std::fs::read(path).map_err(|source| WorkflowError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(Into::into)
}

fn emit_terminal_event(
    store: &WorkflowStore,
    run_id: &str,
    summary: &WorkflowV2ScriptSummary,
) -> WorkflowResult<()> {
    let kind = terminal_event_kind(summary.status);
    let detail = serde_json::json!({
        "event": "terminal_status",
        "status": summary.status,
        "call_id": summary.failed_call,
        "result_path": summary.failed_result_path,
        "next_action": summary.next_action,
    });
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone())
        .emit(run_id, seq, kind, detail)
        .map(|_| ())
}

fn emit_observer_event(
    store: &WorkflowStore,
    run_id: &str,
    kind: WorkflowEventKind,
    event: &str,
    detail: serde_json::Value,
) -> WorkflowResult<()> {
    if event_label_exists(store, run_id, event)? {
        return Ok(());
    }
    let mut detail = detail.as_object().cloned().unwrap_or_default();
    detail.insert("event".to_string(), serde_json::json!(event));
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone())
        .emit(run_id, seq, kind, serde_json::Value::Object(detail))
        .map(|_| ())
}

fn emit_run_status_event(
    store: &WorkflowStore,
    run_id: &str,
    status: &RunStatus,
    detail: &str,
) -> WorkflowResult<()> {
    let kind = match status {
        RunStatus::Paused => WorkflowEventKind::Paused,
        RunStatus::Cancelled => WorkflowEventKind::Cancelled,
        RunStatus::Failed => WorkflowEventKind::StageFailed,
        RunStatus::Blocked | RunStatus::NeedsReview => WorkflowEventKind::StageStalled,
        RunStatus::Completed => WorkflowEventKind::Completed,
        RunStatus::Planned | RunStatus::Running => WorkflowEventKind::StageStarted,
    };
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone())
        .emit(
            run_id,
            seq,
            kind,
            serde_json::json!({
                "event": "terminal_status",
                "status": status,
                "detail": detail,
            }),
        )
        .map(|_| ())
}

fn event_label_exists(store: &WorkflowStore, run_id: &str, label: &str) -> WorkflowResult<bool> {
    let path = store.events_path(run_id);
    let raw = std::fs::read(&path).map_err(|source| WorkflowError::Io {
        path: path.clone(),
        source,
    })?;
    for line in raw
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
    {
        let event = match serde_json::from_slice::<archon_workflow::WorkflowEvent>(line) {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error,
                    "Skipping malformed workflow event during label lookup");
                continue;
            }
        };
        if event
            .detail
            .get("event")
            .and_then(serde_json::Value::as_str)
            == Some(label)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn terminal_event_kind(status: WorkflowV2Status) -> WorkflowEventKind {
    match status {
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop => WorkflowEventKind::StageCompleted,
        WorkflowV2Status::Failed | WorkflowV2Status::Cancelled => WorkflowEventKind::StageFailed,
        WorkflowV2Status::Blocked | WorkflowV2Status::NeedsReview => {
            WorkflowEventKind::StageStalled
        }
        WorkflowV2Status::Pending | WorkflowV2Status::Running => WorkflowEventKind::StageStarted,
    }
}

#[cfg(test)]
#[path = "workflow_finalizer_regression_tests.rs"]
mod regression_tests;
