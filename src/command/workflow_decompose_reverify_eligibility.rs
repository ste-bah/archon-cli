use anyhow::{Context, Result, anyhow};
use archon_workflow::{RunStatus, WorkflowEventKind, WorkflowStore, WorkflowV2ResultStore};
use std::path::Path;

pub(super) fn check_run_status(
    store: &WorkflowStore,
    run_dir: &Path,
    run_id: &str,
    status: RunStatus,
    latest: Option<&crate::command::workflow_decompose_latest_report::LatestReport>,
    v2: &WorkflowV2ResultStore,
) -> Result<bool> {
    let reverify_paused = if status == RunStatus::Paused {
        paused_reverify_is_unconsumed(store, run_dir, run_id, latest, v2)?
    } else {
        false
    };
    if status == RunStatus::Completed || reverify_paused {
        return Ok(reverify_paused);
    }
    if status == RunStatus::Paused {
        anyhow::bail!(
            "workflow {run_id} is not completed: the paused run is not eligible for a fresh reverify: its prior report may be absent or unauthenticated, its remediation ledger may be missing or consumed, or later activity already started; use `archon workflow resume --live --yes` to continue remediation"
        );
    }
    let next_action = match status {
        RunStatus::Running => "wait for it to finish, or pause/cancel it before reverify",
        RunStatus::Cancelled => {
            "resume it with `archon workflow resume --live --yes`, or start a new decomposition"
        }
        RunStatus::Failed => "start a new decomposition",
        _ => "start a new decomposition",
    };
    anyhow::bail!(
        "workflow {run_id} has status {status:?}; reverify requires a completed run or a paused run whose prior reverify ledger is unconsumed; {next_action}"
    )
}

pub(super) fn set_reverify_run_status(
    store: &WorkflowStore,
    run_id: &str,
    status: RunStatus,
) -> Result<()> {
    store.with_run_lock(run_id, |locked| {
        let mut latest = locked.load_state(run_id)?;
        latest.status = status;
        latest.updated_at = chrono::Utc::now();
        locked.save_state(&latest)
    })?;
    Ok(())
}

pub(super) fn recover_accepted_report_status(
    store: &WorkflowStore,
    run_id: &str,
    paused_reverify: bool,
    latest: Option<&crate::command::workflow_decompose_latest_report::LatestReport>,
) -> Result<Option<String>> {
    if let Some(report) = latest.filter(|report| paused_reverify && report.outcome == "accepted") {
        set_reverify_run_status(store, run_id, RunStatus::Completed)?;
        return Ok(Some(format!(
            "Recovered accepted reverify report: {}\nOutcome: accepted",
            report.id
        )));
    }
    Ok(None)
}

pub(crate) fn recover_accepted_reverify_before_resume(
    store: &WorkflowStore,
    run_id: &str,
    latest: Option<&crate::command::workflow_decompose_latest_report::LatestReport>,
) -> Result<Option<String>> {
    let Some(report) =
        latest.filter(|report| report.id.starts_with("reverify-") && report.outcome == "accepted")
    else {
        return Ok(None);
    };
    let v2 = WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"));
    let Some((_, _)) = authenticate_reverify_report(store, run_id, report, &v2, true)? else {
        return Ok(None);
    };
    set_reverify_run_status(store, run_id, RunStatus::Completed)?;
    Ok(Some(format!(
        "Recovered accepted reverify report: {}\nOutcome: accepted\nRun status: Completed; execution was not started",
        report.id
    )))
}

pub(crate) fn paused_reverify_is_unconsumed(
    store: &WorkflowStore,
    run_dir: &Path,
    run_id: &str,
    latest: Option<&crate::command::workflow_decompose_latest_report::LatestReport>,
    v2: &WorkflowV2ResultStore,
) -> Result<bool> {
    let Some(latest) = latest.filter(|report| report.id.starts_with("reverify-")) else {
        return Ok(false);
    };
    let Some((report, _)) = authenticate_reverify_report(store, run_id, latest, v2, false)? else {
        return Ok(false);
    };
    let outcome = report["outcome"].as_str().unwrap_or("");
    if outcome != "needs_remediation" && outcome != "accepted" && outcome != "operational_failure" {
        return Err(anyhow!(
            "reverify report {} has ineligible outcome '{outcome}'",
            latest.id
        ));
    }
    if outcome == "needs_remediation" {
        let ledger = run_dir.join(format!(
            "decomposition/reverify/remediation-{}.json",
            latest.id
        ));
        if !ledger.is_file() {
            return Err(anyhow!(
                "reverify report {} needs remediation but its ledger is missing: {}",
                latest.id,
                ledger.display()
            ));
        }
        let consumed = run_dir.join("decomposition/reverify/consumed.jsonl");
        if consumed.exists()
            && std::fs::read_to_string(&consumed)?
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .any(|row| row["report_id"].as_str() == Some(latest.id.as_str()))
        {
            return Err(anyhow!(
                "reverify report {} remediation ledger is already consumed",
                latest.id
            ));
        }
    };
    Ok(true)
}

/// Authenticate a reverify report against its file bytes, event and v2 record.
/// `require_last_event` distinguishes resume recovery (later activity means
/// this is no longer a recovery candidate) from reverify admission (later
/// activity is a refusal).
fn authenticate_reverify_report(
    store: &WorkflowStore,
    run_id: &str,
    latest: &crate::command::workflow_decompose_latest_report::LatestReport,
    v2: &WorkflowV2ResultStore,
    require_last_event: bool,
) -> Result<Option<(serde_json::Value, u64)>> {
    let event_path = store.events_path(run_id);
    let (events, damaged) = archon_workflow::events::read_event_log(&event_path)?;
    if damaged != 0 {
        return Err(anyhow!(
            "event log {} contains {damaged} damaged or torn line(s); refusing reverify report authentication",
            event_path.display()
        ));
    }
    let Some(report_event) = events.iter().rev().find(|event| {
        event.kind == WorkflowEventKind::DecompositionReverified
            && event
                .detail
                .get("report_id")
                .and_then(serde_json::Value::as_str)
                == Some(latest.id.as_str())
    }) else {
        return Ok(None);
    };
    if events.iter().any(|event| event.seq > report_event.seq) {
        if require_last_event {
            return Ok(None);
        }
        return Err(anyhow!(
            "remediation already started after reverify report {}; resume the workflow",
            latest.id
        ));
    }
    let report_path = store
        .run_dir(run_id)
        .join(format!("decomposition/reverify/reports/{}.json", latest.id));
    let report_bytes = match std::fs::read(&report_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(anyhow!(
                "reverify report {} is missing",
                report_path.display()
            ));
        }
        Err(error) => return Err(error.into()),
    };
    let report: serde_json::Value = serde_json::from_slice(&report_bytes)
        .with_context(|| format!("parsing reverify report {}", report_path.display()))?;
    let digest = archon_workflow::task_set_contract::content_digest(&report_bytes);
    if report_event
        .detail
        .get("report_digest")
        .and_then(serde_json::Value::as_str)
        != Some(digest.as_str())
    {
        return Err(anyhow!(
            "reverify report {} does not match its reverified event digest",
            report_path.display()
        ));
    }
    let record = v2
        .load_call_records()?
        .into_iter()
        .find(|record| {
            record.call.id == latest.id
                && record.call.method == archon_workflow::WorkflowV2HostMethod::FinalReport
        })
        .ok_or_else(|| anyhow!("reverify report {} has no v2 report record", latest.id))?;
    if record.input_hash != digest || record.result.data != report {
        return Err(anyhow!(
            "reverify report {} does not match its v2 report record",
            latest.id
        ));
    }
    if let Some(seq) = report["report_event_seq"].as_u64()
        && seq != report_event.seq
    {
        return Err(anyhow!(
            "reverify report {} event sequence does not match event log",
            latest.id
        ));
    }
    if let Some(snapshot_value) = report.get("call_record_ids_at_report") {
        let snapshot = snapshot_value.as_array().ok_or_else(|| {
            anyhow!(
                "reverify report {} call-record snapshot is invalid",
                latest.id
            )
        })?;
        let expected = snapshot
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>();
        let actual = v2
            .load_call_records()?
            .into_iter()
            .map(|record| record.call.id)
            .collect::<std::collections::BTreeSet<_>>();
        if expected.len() != snapshot.len() || expected != actual {
            return Err(anyhow!(
                "reverify report {} call-record snapshot does not match current records",
                latest.id
            ));
        }
    } else {
        // Legacy reports rely on the paused fixed-run ordering invariant:
        // resume_fixed_decomposition_at_binary_revision applies the Resume
        // lifecycle transition (and appends its event) before invoking the
        // executor, so no v2 call record can be created without a later event.
        // workflow_decompose_resume_event_precedes_any_v2_call_record_on_a_paused_fixed_run pins it.
    }
    Ok(Some((report, report_event.seq)))
}
