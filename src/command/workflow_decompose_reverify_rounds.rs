//! Opt-in convergence through the existing author and gate continuation.
use super::*;
use std::collections::BTreeSet;

pub(crate) async fn reverify(
    cwd: &Path,
    run_id: &str,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<String> {
    Box::pin(observe(cwd, run_id, config, env_vars, None)).await
}

pub(super) fn summary(report: &ReverifyReport, run_id: &str, manual: bool) -> String {
    let mut text = format!(
        "Reverify report: {}\nOutcome: {}\nSubjects: {}",
        report.id,
        report.outcome,
        heal::verdicts(report)
    );
    if manual {
        text.push_str(&format!(
            "\nResume: archon workflow resume --live --yes {run_id}"
        ));
    }
    text
}

pub(crate) fn check_control(
    run: &archon_workflow::WorkflowRun,
    expected: Option<&(u64, RunStatus)>,
) -> archon_workflow::WorkflowResult<()> {
    if expected
        .is_some_and(|(generation, status)| run.generation != *generation || run.status != *status)
    {
        let text = format!(
            "reverify healing stopped: run {} control or ownership changed (status {:?}, generation {})",
            run.id, run.status, run.generation
        );
        return Err(if run.status == RunStatus::Paused {
            archon_workflow::WorkflowError::ControlPaused(text)
        } else {
            archon_workflow::WorkflowError::ControlCancelled(text)
        });
    }
    Ok(())
}

/// The invariant is the failing subject and gate, or the check's failure class
/// and repair route. Findings are free text: judge wording, diagnostic paths,
/// call ids and elapsed times are observations, not evidence of progress.
/// Conservatively require a failing invariant to disappear before retrying it.
pub(super) fn invariants(report: &ReverifyReport) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for subject in &report.subjects {
        if subject.verdict != "accepted" {
            keys.insert(format!("body:{}:{}", subject.task_id, subject.verdict));
        }
    }
    for (scope, gates) in [("set", &report.set_gates), ("frozen", &report.frozen_gates)] {
        for gate in gates.iter().filter(|gate| gate.verdict != "accepted") {
            let subjects = gate.subjects.iter().cloned().collect::<BTreeSet<_>>();
            keys.insert(format!(
                "{scope}:{}:{}:{subjects:?}",
                gate.command_id, gate.verdict
            ));
        }
    }
    for check in report
        .checks
        .iter()
        .filter(|check| check.verdict != "accepted")
    {
        keys.insert(format!(
            "check:{}:{}:{:?}:setup={}:failure={}",
            check.check_id,
            check.verdict,
            check.remediation_route,
            check.setup_failure,
            check.real_failure
        ));
    }
    keys
}

/// Reuse validator-owned defect identities and host-owned judged obligations
/// from the durable gate calls. This distinguishes repairs within one body
/// without treating a changed diagnostic or model explanation as progress.
fn call_invariants(
    report: &ReverifyReport,
    records: &[WorkflowV2CallRecord],
) -> Result<BTreeSet<String>> {
    let mut keys = invariants(report);
    for call_id in &report.calls {
        let record = records
            .iter()
            .find(|record| &record.call.id == call_id)
            .ok_or_else(|| {
                anyhow!(
                    "reverify report {} has no recorded gate {call_id}",
                    report.id
                )
            })?;
        let result: archon_workflow::HostCommandResult =
            serde_json::from_value(record.result.data.clone())
                .with_context(|| format!("reading reverify gate {call_id}"))?;
        if result.reusable() {
            continue;
        }
        let command = record
            .call
            .options
            .host_command
            .as_ref()
            .ok_or_else(|| anyhow!("reverify gate {call_id} has no command identity"))?;
        if let Some(envelope) = result.gate_envelope {
            for finding in envelope.policy_findings {
                let identity = serde_json::to_string(&(
                    &command.command_id,
                    finding.deterministic_defect,
                    finding.subject,
                    finding.remediation_scope,
                    finding.gate_stage,
                    finding.judged_item,
                ))?;
                keys.insert(identity);
            }
            if let Some(error) = envelope.operational_error {
                keys.insert(serde_json::to_string(&(&command.command_id, error.kind))?);
            }
        }
    }
    Ok(keys)
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Accepted,
    Repair,
    NoProgress,
    Oscillation,
    OperationalFailure,
    Controlled,
}

fn decide(
    outcome: &str,
    controlled: bool,
    key: &BTreeSet<String>,
    history: &[BTreeSet<String>],
) -> Decision {
    if controlled {
        return Decision::Controlled;
    }
    match outcome {
        "accepted" => Decision::Accepted,
        "needs_remediation" if history.last() == Some(key) => Decision::NoProgress,
        "needs_remediation" if history.contains(key) => Decision::Oscillation,
        "needs_remediation" => Decision::Repair,
        _ => Decision::OperationalFailure,
    }
}

pub(crate) async fn remediate(
    cwd: &Path,
    run_id: &str,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<String> {
    if run_id.trim().is_empty() || run_id.contains('/') || run_id.contains('\\') {
        return Err(anyhow!("invalid workflow run id '{run_id}'"));
    }
    let store = WorkflowStore::project(cwd.canonicalize()?);
    let run = store.load_state(run_id)?;
    let mut expected = (run.generation, run.status);
    let factory =
        crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::configured_only(
            config, env_vars,
        );
    let mut history = Vec::new();
    loop {
        let observation = Box::pin(observe(cwd, run_id, config, env_vars, Some(&expected))).await;
        // Re-acquire before reading the report or recording the round decision.
        let lease =
            crate::command::workflow_executor_lease::acquire(&store.run_dir(run_id), run_id);
        if let Err(error) = observation {
            let audit = lease.and_then(|_lease| {
                record_observation_stop(&store, run_id, &expected, history.len() + 1, &error)
            });
            return Err(observation_error(error, audit));
        }
        let lease = lease?;
        let latest =
            crate::command::workflow_decompose_latest_report::latest_report(&store, run_id)?
                .ok_or_else(|| anyhow!("reverify healing has no report"))?;
        let report: ReverifyReport = serde_json::from_slice(&std::fs::read(
            store
                .run_dir(run_id)
                .join(format!("{}/{}.json", types::REPORT_PATH, latest.id)),
        )?)?;
        let key = call_invariants(
            &report,
            &WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2")).load_call_records()?,
        )?;
        let status = if report.outcome == "needs_remediation" {
            RunStatus::Paused
        } else {
            RunStatus::Completed
        };
        let next = (expected.0, status);
        let decision = store.with_run_lock(run_id, |locked| {
            let decision = decide(
                &report.outcome,
                check_control(&locked.load_state(run_id)?, Some(&next)).is_err(),
                &key,
                &history,
            );
            if decision == Decision::Controlled {
                return Ok(decision);
            }
            WorkflowEventLog::new(locked.clone()).emit(
                run_id,
                locked.next_event_seq(run_id)?,
                WorkflowEventKind::DecompositionReverifyRound,
                serde_json::json!({"report_id": report.id, "round": history.len() + 1,
                    "decision": format!("{decision:?}"), "invariants": key}),
            )?;
            Ok(decision)
        })?;
        match decision {
            Decision::Accepted => return Ok(summary(&report, run_id, false)),
            Decision::Repair => {}
            _ => {
                let reason = match decision {
                    Decision::NoProgress => {
                        "no progress: the same failing invariants remain after repair"
                    }
                    Decision::Oscillation => {
                        "oscillation: a previously observed failing invariant set returned"
                    }
                    Decision::Controlled => "run control or ownership changed",
                    _ => "operational failure: no accepted observation",
                };
                return Err(anyhow!(
                    "reverify healing stopped: {reason}; failing invariants: {}",
                    key.iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
        }
        history.push(key);
        drop(lease);
        let output = Box::pin(
            crate::command::workflow_decompose::resume_guarded_at_binary_revision(
                cwd,
                run_id,
                true,
                config,
                env_vars,
                &factory,
                crate::command::workflow_decompose_progress::DecompositionCliUiSink::shared(),
                None,
                None,
                &crate::command::workflow_test_seams::binary_revision(),
                Some(&next),
            ),
        )
        .await?;
        // Resume advances the generation once. Inspect its own stop before
        // authorizing another observation, without crossing operator control.
        let generation = next.0.saturating_add(1);
        inspect_resume(
            &store,
            run_id,
            generation,
            &report.id,
            history.len(),
            &output,
        )?;
        expected = (generation, RunStatus::Completed);
    }
}

fn inspect_resume(
    store: &WorkflowStore,
    run_id: &str,
    generation: u64,
    report_id: &str,
    round: usize,
    output: &str,
) -> Result<()> {
    let _lease = crate::command::workflow_executor_lease::acquire(&store.run_dir(run_id), run_id)?;
    let cause = store
        .with_run_lock(run_id, |locked| {
            let run = locked.load_state(run_id)?;
            let (events, _) = archon_workflow::events::read_event_log(&locked.events_path(run_id))?;
            let resumed = events
                .iter()
                .rposition(|event| event.kind == WorkflowEventKind::Resumed);
            let pause = resumed.and_then(|index| {
                events[index + 1..]
                    .iter()
                    .rev()
                    .find(|event| event.kind == WorkflowEventKind::Paused)
            });
            let operator =
                pause.is_some_and(|event| event.detail["cause_kind"] == "operator_pause");
            if !run.execution_owned_at(generation) || operator || run.status == RunStatus::Cancelled
            {
                check_control(&run, Some(&(generation, RunStatus::Completed)))?;
            }
            if run.status != RunStatus::Completed {
                let cause = pause
                    .and_then(|event| event.detail["cause_reason"].as_str())
                    .filter(|_| run.status == RunStatus::Paused)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("resume ended with status {:?}", run.status));
                WorkflowEventLog::new(locked.clone()).emit(
                    run_id,
                    locked.next_event_seq(run_id)?,
                    WorkflowEventKind::DecompositionReverifyRound,
                    serde_json::json!({"report_id": report_id, "round": round,
                        "decision": "ResumeStopped", "status": run.status, "cause": cause}),
                )?;
                return Ok(Some(cause));
            }
            check_control(&run, Some(&(generation, RunStatus::Completed)))?;
            Ok(None)
        })
        .with_context(|| format!("inspecting resume result\n{output}"))?;
    if let Some(cause) = cause {
        return Err(anyhow!("reverify healing stopped: {cause}\n{output}"));
    }
    Ok(())
}

fn record_observation_stop(
    store: &WorkflowStore,
    run_id: &str,
    expected: &(u64, RunStatus),
    round: usize,
    error: &anyhow::Error,
) -> Result<()> {
    let latest = crate::command::workflow_decompose_latest_report::latest_report(store, run_id)?;
    store.with_run_lock(run_id, |locked| {
        // An operator control event already records a changed owner.
        if check_control(&locked.load_state(run_id)?, Some(expected)).is_err() {
            return Ok(());
        }
        WorkflowEventLog::new(locked.clone()).emit(
            run_id,
            locked.next_event_seq(run_id)?,
            WorkflowEventKind::DecompositionReverifyRound,
            serde_json::json!({"report_id": latest.as_ref().map(|report| &report.id),
                "round": round, "decision": "ObservationStopped", "cause": error.to_string()}),
        )?;
        Ok(())
    })?;
    Ok(())
}

fn observation_error(error: anyhow::Error, audit: Result<()>) -> anyhow::Error {
    match audit {
        Ok(()) => error,
        Err(audit_error) => {
            anyhow!("{error:#}; recording observation stop also failed: {audit_error:#}")
        }
    }
}

#[cfg(test)]
#[path = "workflow_decompose_reverify_round_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workflow_decompose_reverify_resume_stop_tests.rs"]
mod resume_stop_tests;
