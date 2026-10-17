use std::path::Path;

#[cfg(test)]
use std::cell::RefCell;

use anyhow::Result;
use archon_workflow::{
    RunStatus, WorkflowEventKind, WorkflowEventLog, WorkflowStore, WorkflowV2CallRecord,
    WorkflowV2ResultStore,
};

pub(super) fn operational_failure_cause(
    report: &super::ReverifyReport,
    operational_failure: bool,
) -> Option<String> {
    if !operational_failure {
        return None;
    }
    let causes = report
        .subjects
        .iter()
        .flat_map(|subject| subject.findings.iter())
        .chain(
            report
                .set_gates
                .iter()
                .chain(&report.frozen_gates)
                .flat_map(|gate| gate.findings.iter()),
        )
        .chain(report.checks.iter().flat_map(|check| check.findings.iter()))
        .cloned()
        .collect::<Vec<_>>();
    Some(
        causes
            .first()
            .cloned()
            .unwrap_or_else(|| "gate gave no verdict".to_string()),
    )
}

#[cfg(test)]
thread_local! {
    static BEFORE_FINAL_PUBLICATION: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn set_before_final_publication_hook(hook: impl FnOnce() + 'static) {
    BEFORE_FINAL_PUBLICATION.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[allow(clippy::too_many_arguments)]
pub(super) fn final_report(
    store: &WorkflowStore,
    run_id: &str,
    _run_dir: &Path,
    lease: &crate::command::workflow_executor_lease::ExecutionLease,
    generation: u64,
    status: &RunStatus,
    v2: &WorkflowV2ResultStore,
    report_path: &str,
    report_bytes: &[u8],
    report_record: &WorkflowV2CallRecord,
    report_event_seq: u64,
    report_event: serde_json::Value,
    ledger: Option<(&str, &serde_json::Value)>,
    next_status: Option<RunStatus>,
) -> Result<()> {
    #[cfg(test)]
    BEFORE_FINAL_PUBLICATION.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });

    store.with_run_lock(run_id, |locked| {
        let mut current = locked.load_state(run_id)?;
        let lease_held = lease.is_held_by(
            lease.holder().pid,
            &lease.holder().acquired_at,
        );
        if current.generation != generation || &current.status != status || !lease_held {
            return Err(match current.status {
                RunStatus::Paused => archon_workflow::WorkflowError::ControlPaused(format!(
                    "reverify stopped because workflow {run_id} was paused or its ownership changed; run reverify again after review"
                )),
                _ => archon_workflow::WorkflowError::ControlCancelled(format!(
                    "reverify stopped because workflow {run_id} was cancelled or its ownership changed; run reverify again after review"
                )),
            });
        }
        locked.write_run_file(run_id, report_path, report_bytes)?;
        v2.save_call_record(report_record)?;
        WorkflowEventLog::new(locked.clone()).emit(
            run_id,
            report_event_seq,
            WorkflowEventKind::DecompositionReverified,
            report_event,
        )?;
        if let Some((path, value)) = ledger {
            locked.write_run_json(run_id, path, value)?;
        }
        if let Some(status) = next_status {
            current.status = status;
            current.updated_at = chrono::Utc::now();
            locked.save_state(&current)?;
        }
        Ok(())
    })?;
    Ok(())
}

pub(super) fn persist_owned_call(
    store: &WorkflowStore,
    run_id: &str,
    _run_dir: &Path,
    lease: &crate::command::workflow_executor_lease::ExecutionLease,
    generation: u64,
    status: &RunStatus,
    v2: &WorkflowV2ResultStore,
    record: &WorkflowV2CallRecord,
) -> Result<()> {
    store.with_run_lock(run_id, |locked| {
        let current = locked.load_state(run_id)?;
        let lease_held = lease.is_held_by(
            lease.holder().pid,
            &lease.holder().acquired_at,
        );
        if current.generation != generation || &current.status != status || !lease_held {
            return Err(match current.status {
                RunStatus::Paused => archon_workflow::WorkflowError::ControlPaused(format!(
                    "reverify stopped because workflow {run_id} was paused or its ownership changed"
                )),
                _ => archon_workflow::WorkflowError::ControlCancelled(format!(
                    "reverify stopped because workflow {run_id} was cancelled or its ownership changed"
                )),
            });
        }
        v2.save_call_record(record)?;
        Ok(())
    })?;
    Ok(())
}
