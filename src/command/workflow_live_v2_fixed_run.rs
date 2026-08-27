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

    let runtime = WorkflowV2ScriptRuntime {
        target_repository_root: None,
        generated_config: plan.generated_config.clone(),
    };
    let client =
        LiveV2AgentClient::new(llm, ui_sink, agent_names, run.id.clone(), None, Some(1_500))
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
            persist_terminal_run_status(store, &run.id, RunStatus::Paused)?;
            return Ok(format!(
                "Fixed decomposition paused: {}\n{}\nResume with: archon workflow resume --live --yes {}\n",
                run.id, message, run.id
            ));
        }
        Err(WorkflowError::ControlCancelled(message)) => {
            persist_terminal_run_status(store, &run.id, RunStatus::Cancelled)?;
            return Ok(format!(
                "Fixed decomposition cancelled: {}\n{}\n",
                run.id, message
            ));
        }
        Err(error) => {
            if let Err(state_error) = persist_terminal_run_status(store, &run.id, RunStatus::Failed)
            {
                tracing::warn!(
                    run_id = %run.id,
                    error = %state_error,
                    "failed to persist fixed decomposition failure state"
                );
            }
            return Err(error.into());
        }
    };
    sync_v2_summary_to_run(store, &run.id, &summary.calls, &v2_store, summary.status)?;
    Ok(format!(
        "Fixed decomposition {}: status {:?}, completed {}, executed {}, reused {}\n",
        run.id, summary.status, summary.completed, summary.executed, summary.reused
    ))
}
