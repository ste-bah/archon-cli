//! `archon workflow observe-run-end`: run a finished run's observe-only
//! run-end acceptance observation again after it failed.
//!
//! Nothing here can change the run's status: the terminal state and event
//! are committed and only read, the outcome keeps observe-only authority,
//! and the only records written are the finalization record's observer
//! state (the earlier failure is kept beside it), the observer's own files
//! and its events. The current pin must first be proven reached from the
//! launch pin; an unproven chain is refused before anything is written.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use archon_workflow::task_set_contract::AcceptancePin;
use archon_workflow::{
    RunEndObserverStateV1, RunStatus, WorkflowEventKind, WorkflowEventLog, WorkflowStore,
};

use super::{
    FINALIZATION_RECORD_PATH, RunEndObserverContext, WorkflowRunEndObserver, read_record,
    require_generation_owner,
};

fn emit(
    store: &WorkflowStore,
    run_id: &str,
    kind: WorkflowEventKind,
    detail: serde_json::Value,
) -> archon_workflow::WorkflowResult<()> {
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone())
        .emit(run_id, seq, kind, detail)
        .map(|_| ())
}

const NATIVE_OBSERVATION_PATH: &str = "observer/native-observation.json";

pub(crate) async fn observe_run_end(project: &Path, run_id: &str) -> Result<String> {
    let store = WorkflowStore::project(project);
    let path = store.run_dir(run_id).join(FINALIZATION_RECORD_PATH);
    let record = read_record(&path)?;
    let snapshot = record
        .observer_snapshot
        .clone()
        .ok_or_else(|| anyhow!("run {run_id} has no launch-time observer snapshot to observe"))?;
    let terminal_status = record
        .terminal_v2_status
        .ok_or_else(|| anyhow!("run {run_id} has no terminal v2 status to observe"))?;
    // Refuse an unproven chain before anything is written.
    if let Some(launch) = &snapshot.portable_acceptance_identity {
        let task_root = PathBuf::from(&snapshot.canonical_task_root_identity);
        let pin_path = crate::command::workflow_task_set::acceptance_pin_path(project, &task_root);
        let pin: AcceptancePin = serde_json::from_slice(&std::fs::read(&pin_path)?)?;
        crate::command::acceptance_chain::verify_launch_chain(
            launch,
            crate::command::acceptance_chain::launch_lineage(&snapshot),
            &pin,
            &pin_path,
            &task_root,
            run_id,
        )
        .map_err(|detail| anyhow!("refusing to observe run {run_id}: {detail}"))?;
    }
    let (mut record, generation) = store.with_run_lock(run_id, |locked| {
        let run = locked.load_state(run_id)?;
        if !matches!(run.status, RunStatus::Completed | RunStatus::NeedsReview) {
            return Err(archon_workflow::WorkflowError::StateCorrupt(format!(
                "run-end observation re-runs only for a finished run; {run_id} is {:?}",
                run.status
            )));
        }
        let mut record = read_record(&path)?;
        let prior = record.reopen_observer()?;
        // The failed observation's evidence is kept, never read as this one's.
        let native = locked.run_dir(run_id).join(NATIVE_OBSERVATION_PATH);
        if native.exists() {
            let kept = native.with_file_name(format!(
                "native-observation.prior-{}-{}.json",
                record.prior_observer_failures.len(),
                uuid::Uuid::new_v4().simple()
            ));
            std::fs::rename(&native, &kept).map_err(|source| {
                archon_workflow::WorkflowError::Io {
                    path: native.clone(),
                    source,
                }
            })?;
        }
        locked.write_run_json(run_id, FINALIZATION_RECORD_PATH, &record)?;
        emit(
            locked,
            run_id,
            WorkflowEventKind::RunEndAcceptanceObserverStarted,
            serde_json::json!({
                "event": "run_end_acceptance_observer_reopened",
                "authority": "observe_only",
                "prior_failure": prior,
            }),
        )?;
        Ok((record, run.generation))
    })?;
    let observer =
        super::super::workflow_run_end_observer::FixedRunEndAcceptanceObserver::new(store.clone());
    let context = RunEndObserverContext {
        run_id,
        terminal_status,
        snapshot: &snapshot,
    };
    let outcome = observer.observe_async(&context).await;
    store.with_run_lock(run_id, |locked| {
        require_generation_owner(locked, run_id, Some(generation))?;
        // Only the reopen this call made may be completed by it.
        if read_record(&path)? != record {
            return Err(archon_workflow::WorkflowError::StateCorrupt(format!(
                "run {run_id}'s finalization record changed during the re-observation; its outcome was not recorded"
            )));
        }
        match &outcome {
            Ok(outcome) => record.complete_observer(outcome.clone())?,
            Err(error) => record.fail_observer(error.to_string())?,
        }
        locked.write_run_json(run_id, FINALIZATION_RECORD_PATH, &record)?;
        if let Err(error) = &outcome {
            emit(
                locked,
                run_id,
                WorkflowEventKind::RunEndAcceptanceObserverFailed,
                serde_json::json!({
                    "event": "run_end_acceptance_observer_failed",
                    "reason": error.to_string(),
                }),
            )?;
        }
        Ok(())
    })?;
    Ok(match record.observer_state {
        Some(RunEndObserverStateV1::Completed { outcome }) => format!(
            "observer: completed authority=observe_only evaluated_floors={} policy_findings={} operational_deferrals={}\n",
            outcome.evaluated_floor_count,
            outcome.policy_finding_count,
            outcome.operational_deferral_count
        ),
        Some(RunEndObserverStateV1::Failed { reason }) => format!("observer: failed: {reason}\n"),
        other => format!("observer: {other:?}\n"),
    })
}
