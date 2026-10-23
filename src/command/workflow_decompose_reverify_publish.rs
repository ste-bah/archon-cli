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
    if let Some(cause) = &report.operational_failure_cause {
        return Some(cause.clone());
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_operational_failure(
    store: &WorkflowStore,
    run_id: &str,
    run_dir: &Path,
    lease: &crate::command::workflow_executor_lease::ExecutionLease,
    generation: u64,
    status: &RunStatus,
    v2: &WorkflowV2ResultStore,
    prior_id: &str,
    prior_digest: &str,
    invocation_id: &str,
    error: &anyhow::Error,
) -> anyhow::Error {
    let cause = archon_observability::redaction::redact_text(&error.to_string());
    match publish_operational_failure(
        store,
        run_id,
        run_dir,
        lease,
        generation,
        status,
        v2,
        prior_id,
        prior_digest,
        invocation_id,
        error,
    ) {
        Ok(recorded_error) => recorded_error,
        Err(publication_error) => {
            let publication_error =
                archon_observability::redaction::redact_text(&publication_error.to_string());
            anyhow::anyhow!(
                "reverify operational failure: {cause}; failure record was not written: {publication_error}"
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn publish_operational_failure(
    store: &WorkflowStore,
    run_id: &str,
    run_dir: &Path,
    lease: &crate::command::workflow_executor_lease::ExecutionLease,
    generation: u64,
    status: &RunStatus,
    v2: &WorkflowV2ResultStore,
    prior_id: &str,
    prior_digest: &str,
    invocation_id: &str,
    error: &anyhow::Error,
) -> Result<anyhow::Error> {
    let cause = archon_observability::redaction::redact_text(&error.to_string());
    let report_event_seq = store.next_event_seq(run_id)?;
    let report_id = format!("{invocation_id}-report");
    let mut call_record_ids_at_report = v2
        .load_call_records()?
        .into_iter()
        .map(|record| record.call.id)
        .collect::<Vec<_>>();
    call_record_ids_at_report.push(report_id.clone());
    call_record_ids_at_report.sort();
    let report = super::ReverifyReport {
        schema_version: 1,
        id: report_id.clone(),
        supersedes_id: prior_id.to_string(),
        supersedes_digest: prior_digest.to_string(),
        outcome: "operational_failure".into(),
        report_event_seq,
        call_record_ids_at_report,
        calls: Vec::new(),
        subjects: Vec::new(),
        set_gates: Vec::new(),
        frozen_gates: Vec::new(),
        checks: Vec::new(),
        contract_digest: None,
        judged_check_digests: std::collections::BTreeMap::new(),
        judged_check_identity: None,
        operational_failure_cause: Some(cause.clone()),
    };
    let report_bytes = serde_json::to_vec_pretty(&report)?;
    let report_digest = archon_workflow::task_set_contract::content_digest(&report_bytes);
    let record = WorkflowV2CallRecord::new(
        run_id,
        archon_workflow::WorkflowV2HostCall {
            id: report_id.clone(),
            method: archon_workflow::WorkflowV2HostMethod::FinalReport,
            write_mode: None,
            options: archon_workflow::WorkflowV2HostOptions::default(),
        },
        1,
        report_digest.clone(),
        archon_workflow::WorkflowV2Result {
            status: archon_workflow::WorkflowV2Status::NeedsReview,
            summary: "reverify outcome: operational_failure".into(),
            data: serde_json::to_value(&report)?,
            ..archon_workflow::WorkflowV2Result::default()
        },
        Vec::new(),
    );
    let report_event = serde_json::json!({
        "report_id": report_id,
        "invocation_id": invocation_id,
        "report_digest": report_digest,
        "supersedes_id": prior_id,
        "outcome": "operational_failure",
        "operational_failure_cause": cause
    });
    final_report(
        store,
        run_id,
        run_dir,
        lease,
        generation,
        status,
        v2,
        &format!("decomposition/reverify/reports/{report_id}.json"),
        &report_bytes,
        &record,
        report_event_seq,
        report_event,
        None,
        None,
    )?;
    let cause = reverify_retry_guidance(&cause, run_id);
    Ok(anyhow::anyhow!(
        "reverify recorded operational_failure: {cause}"
    ))
}

fn reverify_retry_guidance(cause: &str, run_id: &str) -> String {
    let command = format!("archon workflow reverify {run_id}");
    let cause = cause.replace("archon workflow reverify <run>", &command);
    if cause.contains(&command) {
        cause
    } else {
        format!("{cause}; run `{command}` again")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn retry_guidance_uses_the_run_id_and_does_not_duplicate_existing_advice() {
        let run_id = "run-452";
        let advised = super::reverify_retry_guidance(
            "provider failed; run `archon workflow reverify <run>` again",
            run_id,
        );
        let command = format!("archon workflow reverify {run_id}");
        assert!(advised.contains(&command), "{advised}");
        assert_eq!(advised.matches(&command).count(), 1, "{advised}");
        assert!(!advised.contains("Retry the freeze"), "{advised}");
        let missing = super::reverify_retry_guidance("provider failed", run_id);
        assert!(missing.contains(&command), "{missing}");
        assert_eq!(missing.matches(&command).count(), 1, "{missing}");
        assert!(!missing.contains("Retry the freeze"), "{missing}");
    }
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
