//! Dedicated v3 execution path for the immutable fixed decomposition script.

use super::*;

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
    run.status = RunStatus::Running;
    run.mark_updated();
    store.save_state(&run)?;
    let execution_generation = run.generation;

    let runtime = WorkflowV2ScriptRuntime {
        target_repository_root: None,
        generated_config: plan.generated_config.clone(),
    };
    // The fixed decomposition author gets the SAME configured host-call timeout
    // as a generated run (`workflow_live_v2_run.rs`), not a literal.
    //
    // This was `Some(1_500)` from 67a97c6e8 (2026-08-27) — 25 minutes, in no
    // config file, so an operator raising `host_call_timeout_secs` changed
    // nothing here. Run wf-7d2a5ba2 lost five of six acceptance-author attempts
    // to it on a clean 25-minute cadence while the provider answered normally
    // (0 max_tokens, 0 empty replies, 71 completed responses); attempt 3 did
    // finish, so the work fits the model, just not the timeout. Authoring a full
    // acceptance contract from a 36KB PRD is legitimately longer work than an
    // ordinary host call.
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
        "CartographerScan".to_string(),
    ]);
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let runner = WorkflowV2ScriptRunner::new(
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
    .with_raw_outcomes(true);
    let summary = match runner.run(&plan.harness_source).await {
        Ok(summary) => summary,
        Err(WorkflowError::ControlPaused(message)) => {
            return Ok(format!(
                "Fixed decomposition paused: {}\n{}\nResume with: archon workflow resume --live --yes {}\n",
                run.id, message, run.id
            ));
        }
        Err(WorkflowError::ControlCancelled(message)) => {
            return Ok(format!(
                "Fixed decomposition cancelled: {}\n{}\n",
                run.id, message
            ));
        }
        Err(error) => {
            super::workflow_live_v2_finalizer::finalize_run_status(
                store,
                &run.id,
                archon_workflow::WorkflowRunKind::FixedDecompositionV1,
                RunStatus::Failed,
                &error.to_string(),
                Some(execution_generation),
            )?;
            return Err(error.into());
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
    let _ = crate::command::workflow_decompose_log::append_nofollow_line(
        &log_path,
        &format!("transition={transition} field={label} text={text}"),
    );
}
