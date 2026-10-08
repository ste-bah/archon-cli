//! Dedicated v3 execution path for the immutable fixed decomposition script.

use super::*;

#[path = "workflow_live_v2_lost_ownership.rs"]
pub(super) mod lost_ownership;
use lost_ownership::{lost_ownership, lost_ownership_report};

pub(crate) async fn execute_fixed_decomposition_v2_run(
    store: &WorkflowStore,
    mut run: WorkflowRun,
    plan: WorkflowScriptPlan,
    llm: Arc<dyn WorkflowLlmClient>,
    ui_sink: SharedWorkflowUiSink,
    agent_names: Vec<String>,
    host_command_executor: Arc<
        dyn crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor,
    >,
) -> Result<String> {
    const LABEL: &str = "Fixed decomposition";
    let v2_store = match persist_fixed_start(store, &mut run) {
        Ok(v2_store) => v2_store,
        // A newer executor owns the run: this launch changed nothing.
        Err(refusal) => match refusal {
            WorkflowError::ControlCancelled(refused) => {
                return Ok(lost_ownership_report(LABEL, &run.id, &refused));
            }
            other => return Err(other.into()),
        },
    };
    let owner_store = store.for_executor(&run.id, run.generation);
    let store = &owner_store;
    let execution_generation = run.generation;
    let pause_executor = host_command_executor.clone();
    let runner = fixed_runner(
        &run,
        &plan,
        llm,
        ui_sink,
        agent_names,
        host_command_executor,
        store,
        &v2_store,
    );
    let summary = match runner.run(&plan.harness_source).await {
        Ok(summary) => summary,
        Err(WorkflowError::ControlPaused(message)) => {
            if lost_ownership(store, &run.id, execution_generation).is_some() {
                return Ok(lost_ownership_report(LABEL, &run.id, &message));
            }
            return Ok(format!(
                "Fixed decomposition paused: {}\n{}\nResume with: archon workflow resume --live --yes {}\n",
                run.id, message, run.id
            ));
        }
        Err(WorkflowError::ControlCancelled(message)) => {
            if lost_ownership(store, &run.id, execution_generation).is_some() {
                return Ok(lost_ownership_report(LABEL, &run.id, &message));
            }
            return Ok(format!(
                "Fixed decomposition cancelled: {}\n{}\n",
                run.id, message
            ));
        }
        Err(error) => {
            // An unplanned host/runtime fault carries no terminal verdict.
            // If even the pause cannot be stored, report that fault explicitly;
            // never fall back to terminal failure.
            if lost_ownership(store, &run.id, execution_generation).is_some() {
                return Ok(lost_ownership_report(LABEL, &run.id, &error.to_string()));
            }
            let text =
                crate::command::workflow_decompose_events::bounded_log_field(&error.to_string());
            // Issue 337: this pause covers what the run recorded, as any
            // script-error pause of a fixed script does.
            let coverage = super::workflow_live_v2_script::HostPauseCoverage::snapshot(
                &v2_store,
                Some(&pause_executor),
            );
            // Written in the pause's own lock section (no resume between).
            let paused = archon_workflow::control_pause::pause_owned_then(
                store,
                &run.id,
                archon_workflow::control_pause::PauseOwner::Executor(execution_generation),
                serde_json::json!({"event":"fixed_host_error_pause","error":text}),
                |locked, seq| coverage.record(locked, &run.id, "boundary-error", seq),
            );
            match paused {
                Ok(Ok(_)) => {
                    return Ok(format!(
                        "Fixed decomposition paused: {}\n{}\nResume with: archon workflow resume --live --yes {}\n",
                        run.id, text, run.id
                    ));
                }
                Ok(Err(evidence_error)) => {
                    return Err(anyhow::anyhow!(
                        "fixed decomposition {} is paused after {text}, but its pause evidence could not be recorded: {evidence_error}",
                        run.id
                    ));
                }
                Err(WorkflowError::ControlPaused(message)) => {
                    return Ok(format!(
                        "Fixed decomposition paused: {}\n{}\n",
                        run.id, message
                    ));
                }
                Err(WorkflowError::ControlCancelled(message)) => {
                    return Ok(
                        if lost_ownership(store, &run.id, execution_generation).is_some() {
                            lost_ownership_report(LABEL, &run.id, &message)
                        } else {
                            format!("Fixed decomposition cancelled: {}\n{}\n", run.id, message)
                        },
                    );
                }
                Err(pause_error) => {
                    return Err(anyhow::anyhow!(
                        "fixed decomposition host error: {text}; its pause could not be persisted: {pause_error}"
                    ));
                }
            }
        }
    };
    super::workflow_live_v2_finalizer::finalize_summary(
        store,
        &run.id,
        archon_workflow::WorkflowRunKind::FixedDecompositionV1,
        None,
        &summary,
        &v2_store,
        None,
        Some(execution_generation),
    )
    .await?;
    let mut report = format!(
        "Fixed decomposition {}: status {:?}, completed {}, executed {}, reused {}\n",
        run.id, summary.status, summary.completed, summary.executed, summary.reused
    );
    // A terminal status with no reason is what turns a failure into a
    // run-and-inspect cycle. The summary has carried the failing call, its
    // result path and the next action all along; none of it was printed, so a
    // run could burn twenty-five minutes and report `Failed` with no cause
    // anywhere in stdout, stderr or the durable log.
    // `NeedsReview` is a terminal status a run reaches correctly -- the fixed
    // synthetic PRD guarantees one, so labelling it `run_failed` and printing
    // "restart or resume" for a run that already finished sends a reader
    // chasing a failure that does not exist. The diagnostic fields are still
    // worth recording; only the label was wrong.
    let transition = match summary.status {
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop => None,
        WorkflowV2Status::NeedsReview => Some("run_needs_review"),
        _ => Some("run_failed"),
    };
    if let Some(transition) = transition {
        for (label, value) in [
            ("failed_call", summary.failed_call.as_deref()),
            ("failed_result", summary.failed_result_path.as_deref()),
            ("script_error", summary.script_error.as_deref()),
            ("next_action", summary.next_action.as_deref()),
        ] {
            let Some(value) = value.filter(|text| !text.trim().is_empty()) else {
                continue;
            };
            report.push_str(&format!("  {label}: {value}\n"));
            record_terminal_reason(store, &run.id, transition, label, value);
        }
    }
    Ok(report)
}

/// The runner of a fixed decomposition launched at `run.generation`, writing
/// through `v2_store`, whose session `persist_fixed_start` bound.
#[allow(clippy::too_many_arguments)]
pub(super) fn fixed_runner(
    run: &WorkflowRun,
    plan: &WorkflowScriptPlan,
    llm: Arc<dyn WorkflowLlmClient>,
    ui_sink: SharedWorkflowUiSink,
    agent_names: Vec<String>,
    host_command_executor: Arc<
        dyn crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor,
    >,
    store: &WorkflowStore,
    v2_store: &WorkflowV2ResultStore,
) -> WorkflowV2ScriptRunner {
    let runtime = WorkflowV2ScriptRuntime {
        target_repository_root: None,
        generated_config: plan.generated_config.clone(),
    };
    // The fixed decomposition author gets the SAME configured host-call timeout
    // as a generated run (`workflow_live_v2_run.rs`), not a literal.
    //
    // This was `Some(1_500)` from 67a97c6e8 (2026-08-27) — 25 minutes, in no
    // config file, so an operator raising `host_call_timeout_secs` changed
    // nothing here. A live run lost five of six acceptance-author attempts
    // to it on a clean 25-minute cadence while the provider answered normally
    // (0 max_tokens, 0 empty replies, 71 completed responses); attempt 3 did
    // finish, so the work fits the model, just not the timeout. Authoring a full
    // acceptance contract from a 36KB PRD is legitimately longer work than an
    // ordinary host call. Issue 288: the value is a no-progress window that
    // novel author activity renews, never a total limit on the session.
    let client = LiveV2AgentClient::new(
        llm,
        ui_sink,
        agent_names,
        run.id.clone(),
        None,
        Some(u64::from(runtime.generated_config.host_call_timeout_secs)),
    )
    .with_fixed_raw_tool_policy(vec![
        "Read".to_string(),
        "Grep".to_string(),
        "Glob".to_string(),
    ]);
    WorkflowV2ScriptRunner::new(
        run.spec.task.clone(),
        runtime,
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        store.clone(),
        run.id.clone(),
        true,
        None,
        plan.script_args.clone(),
    )
    .with_host_command_executor(host_command_executor)
    .with_raw_outcomes(true)
}

/// Marks the run running and binds this executor's session to the
/// generation it launched at, under the run lock (Issue 329). Bound here, at
/// launch, and not when the script starts: a resume in between would
/// otherwise have its newer generation read at script start and taken for
/// this executor's own, leaving the session unfenced. A newer executor that
/// already owns the run refuses the start
/// (`control_pause::require_executor`), and nothing is written.
pub(super) fn persist_fixed_start(
    store: &WorkflowStore,
    run: &mut WorkflowRun,
) -> archon_workflow::WorkflowResult<WorkflowV2ResultStore> {
    let run_id = run.id.clone();
    store.with_run_lock(&run_id, |locked| {
        // An unreadable state proves nothing; the save below reports it.
        if let Ok(current) = locked.load_state(&run_id) {
            archon_workflow::control_pause::require_executor(&current, run.generation)?;
        }
        run.status = RunStatus::Running;
        run.mark_updated();
        locked.save_state(run)?;
        let v2_store = WorkflowV2ResultStore::new(locked.run_dir(&run_id).join("v2"));
        v2_store.bind_session_executor(run.generation);
        Ok(v2_store)
    })
}

#[cfg(test)]
#[path = "workflow_live_v2_fixed_error_pause_tests.rs"]
mod error_pause_tests;
#[cfg(test)]
#[path = "workflow_live_v2_fixed_replay_guard_tests.rs"]
mod replay_guard_tests;
#[cfg(test)]
#[path = "workflow_live_v2_fixed_start_tests.rs"]
mod start_tests;

/// Mirrors a terminal run's reason into `.decompose.log`.
///
/// Best effort: the run has already reached its terminal state, so failing to
/// annotate it must not replace the reason with a different error.
fn record_terminal_reason(
    store: &WorkflowStore,
    run_id: &str,
    transition: &str,
    label: &str,
    value: &str,
) {
    let path = store
        .run_dir(run_id)
        .join(crate::command::workflow_decompose_state::FIXED_STATE_PATH);
    let Ok(raw) = std::fs::read(&path) else {
        return;
    };
    let Ok(state) = serde_json::from_slice::<archon_workflow::FixedDecompositionStateV1>(&raw)
    else {
        return;
    };
    let Ok(log_path) = crate::command::workflow_decompose_log::validated_fixed_log_path(
        std::path::Path::new(&state.log_path),
        &state.identity,
    ) else {
        return;
    };
    let text = crate::command::workflow_decompose_events::log_field(value);
    if let Err(error) = store.with_run_lock(run_id, |_| {
        crate::command::workflow_decompose_log::append_nofollow_line(
            &log_path,
            &format!("transition={transition} field={label} text={text}"),
        )
    }) {
        tracing::warn!(%error, "terminal reason not appended");
    }
}
