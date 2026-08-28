//! Central durable terminal finalization for generated v3 runs.
//!
//! Script hosts return summaries. This composition boundary alone persists the
//! terminal run projection, appends the stable terminal event, records its
//! commit, and only then invokes an eligible observe-only run-end observer.

use std::path::Path;

use archon_workflow::{
    FinalizationRecordV1, RunEndAcceptanceObserverSnapshotV1, RunEndObserverOutcomeV1,
    RunEndObserverStateV1, RunStatus, WorkflowError, WorkflowEventKind, WorkflowEventLog,
    WorkflowResult, WorkflowRunKind, WorkflowStore, WorkflowV2ResultStore, WorkflowV2Status,
};

use super::workflow_live_v2_script::WorkflowV2ScriptSummary;

pub(super) const FINALIZATION_RECORD_PATH: &str = "v2/finalization.json";

pub(super) struct RunEndObserverContext<'a> {
    pub(super) run_id: &'a str,
    pub(super) terminal_status: WorkflowV2Status,
    pub(super) snapshot: &'a RunEndAcceptanceObserverSnapshotV1,
}

pub(super) trait WorkflowRunEndObserver: Send + Sync {
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
    let path = store.run_dir(run_id).join(FINALIZATION_RECORD_PATH);
    let mut record = if path.exists() {
        read_record(&path)?
    } else {
        FinalizationRecordV1::new(run_kind, summary.status, snapshot)
    };
    verify_summary_record_identity(&record, run_kind, summary.status)?;

    if !record.terminal_event_committed {
        store.with_run_lock(run_id, |locked| {
            require_generation_owner(locked, run_id, expected_generation)?;
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
            locked.write_run_json(run_id, FINALIZATION_RECORD_PATH, &record)
        })?;
    }

    if record.observer_state != Some(RunEndObserverStateV1::Pending) {
        return Ok(());
    }
    let Some(observer) = observer else {
        return Ok(());
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
    };
    match observer.observe(&context) {
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
    Ok(())
}

pub(super) fn finalize_run_status(
    store: &WorkflowStore,
    run_id: &str,
    run_kind: WorkflowRunKind,
    status: RunStatus,
    detail: &str,
    expected_generation: Option<u64>,
) -> WorkflowResult<()> {
    let path = store.run_dir(run_id).join(FINALIZATION_RECORD_PATH);
    let mut record = if path.exists() {
        read_record(&path)?
    } else {
        FinalizationRecordV1::for_run_status(run_kind, status.clone())
    };
    if record.run_kind != run_kind
        || record.terminal_status != status
        || record.terminal_v2_status.is_some()
    {
        return Err(WorkflowError::StateCorrupt(format!(
            "terminal finalization identity changed for run {run_id}"
        )));
    }
    if record.terminal_event_committed {
        return Ok(());
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

fn require_generation_owner(
    store: &WorkflowStore,
    run_id: &str,
    expected_generation: Option<u64>,
) -> WorkflowResult<()> {
    let Some(expected) = expected_generation else {
        return Ok(());
    };
    let current = store.load_state(run_id)?;
    if current.generation != expected {
        return Err(WorkflowError::ControlCancelled(format!(
            "fixed executor generation {expected} no longer owns run {run_id}; current generation is {}",
            current.generation
        )));
    }
    Ok(())
}

fn verify_summary_record_identity(
    record: &FinalizationRecordV1,
    run_kind: WorkflowRunKind,
    status: WorkflowV2Status,
) -> WorkflowResult<()> {
    if record.run_kind != run_kind || record.terminal_v2_status != Some(status) {
        return Err(WorkflowError::StateCorrupt(format!(
            "finalization identity changed: persisted {:?}/{:?}, current {:?}/{:?}",
            record.run_kind, record.terminal_v2_status, run_kind, status
        )));
    }
    Ok(())
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
    let raw = std::fs::read_to_string(&path).map_err(|source| WorkflowError::Io {
        path: path.clone(),
        source,
    })?;
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let event: archon_workflow::WorkflowEvent = serde_json::from_str(line)?;
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
