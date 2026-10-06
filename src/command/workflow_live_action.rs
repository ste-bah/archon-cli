//! Capture operator policy at the live launch boundary and carry it through planning.

use super::*;

pub(super) async fn run_live_action(
    cwd: &Path,
    action: CommandAction,
    llm: Arc<dyn WorkflowLlmClient>,
    ui_sink: SharedWorkflowUiSink,
    config_path: Option<PathBuf>,
    generated_config: GeneratedWorkflowConfig,
    workspace_boundary_supported: bool,
    approval_mode: LiveApprovalMode,
) -> Result<String> {
    run_live_action_with_policy(
        cwd,
        action,
        llm,
        ui_sink,
        config_path,
        generated_config,
        workspace_boundary_supported,
        approval_mode,
        None,
    )
    .await
}

pub(super) async fn run_live_action_with_policy(
    cwd: &Path,
    action: CommandAction,
    llm: Arc<dyn WorkflowLlmClient>,
    ui_sink: SharedWorkflowUiSink,
    config_path: Option<PathBuf>,
    generated_config: GeneratedWorkflowConfig,
    workspace_boundary_supported: bool,
    approval_mode: LiveApprovalMode,
    captured_policy: Option<Option<archon_workflow::acceptance_check_environment::CheckPolicy>>,
) -> Result<String> {
    let store = WorkflowStore::project(cwd);
    let check_policy = match captured_policy {
        Some(policy) => Some(policy),
        None => crate::command::acceptance_check_policy::for_new_plan(
            &action,
            cwd,
            config_path.as_deref(),
        )?,
    };
    let policy = live_policy(cwd, config_path.as_deref());
    let learning = load_learning_config(cwd, config_path.as_deref());
    // The one place a generated run's limits are decided. SONA is consulted
    // here rather than inside the planner because every downstream consumer —
    // the lifecycle driver's repair caps, the host-call client's timeout, the
    // read-only branch budget, and the metadata a resume replays — reads the
    // same `generated_config`, so a single substitution reaches all of them and
    // no path can end up with a half-tuned config.
    let task_class = live_task_class(&action);
    let mut generated_config = generated_config;
    let mut tuning_decisions = Vec::new();
    if let Some(class) = task_class {
        let tuning = crate::command::sona_workflow_tuning::tune_generated_config(
            cwd,
            class,
            &learning,
            &generated_config,
        );
        let report = tuning.report(class);
        generated_config = tuning.config;
        tuning_decisions = tuning.decisions;
        if !report.is_empty() {
            tracing::info!(class, %report, "generated limits tuned by SONA");
            // Emitted before any work starts: a user who wonders why this run
            // got five repair iterations must be able to read the answer in the
            // run's own output rather than reconstruct it from the learning
            // store by hand.
            if let Err(error) = ui_sink.emit(WorkflowUiEvent::Text(report)).await {
                tracing::debug!(%error, "tuning report delivery failed");
            }
        }
    }
    let generated_config = generated_config;
    let tuning_decisions = tuning_decisions;
    let runner = PipelineWorkflowRunner {
        llm: llm.clone(),
        ui_sink: ui_sink.clone(),
        agent_names: AgentRegistry::load(cwd)
            .available_agent_names()
            .into_iter()
            .map(str::to_string)
            .collect(),
        workspace_boundary_supported,
    };
    let capped_live_plan = |task: &str| {
        let llm = llm.clone();
        let ui_sink = ui_sink.clone();
        let task = task.to_string();
        let store = &store;
        let runner = &runner;
        let policy = &policy;
        let generated_config = &generated_config;
        let learning = &learning;
        let check_policy = &check_policy;
        let tuning_decisions = &tuning_decisions;
        async move {
            let mut plan = plan_live(
                store,
                &task,
                llm,
                ui_sink.clone(),
                generated_config,
                learning,
            )
            .await?;
            cap_live_plan_parallelism(&mut plan, runner, policy);
            plan.check_policy = check_policy.clone();
            // Attached here rather than inside the planner because the planner
            // never sees the baseline it was tuned away from, and a decision
            // record without its baseline explains nothing.
            plan.tuning_decisions = tuning_decisions.clone();
            // Shape comes after the plan, not before it like the budgets: the
            // knob is scored against the plan's own stage families and the
            // declared task graph, and neither exists until the planner has
            // run. Budgets have no such dependency, which is why they are
            // resolved earlier and reach the planner itself.
            apply_generated_shape(cwd, task_class, learning, &mut plan, &ui_sink).await;
            Ok::<_, anyhow::Error>(plan)
        }
    };
    match action {
        CommandAction::Plan { task } => {
            let plan = capped_live_plan(&task).await?;
            render_live_plan(&plan)
        }
        CommandAction::PlanSpec { path } => Ok(load_spec_file(cwd, &path)?.to_yaml()?),
        CommandAction::Run { task, decomposed } => {
            let plan = capped_live_plan(&task).await?;
            return workflow_live_v2::run_generated_v2_workflow(
                cwd,
                &store,
                plan,
                task,
                llm,
                ui_sink,
                runner.agent_names.clone(),
                approval_mode,
                workspace_boundary_supported,
                if decomposed {
                    false
                } else {
                    workflow_live_v2::script_lifecycle_from_env()
                },
                &learning,
            )
            .await;
        }
        CommandAction::RunSpec { .. } => Err(anyhow!(
            "legacy imported-spec execution was removed by the workflow runtime rescue;                  run work through the V2 runtime with /workflow run <task> or a saved V2 workflow"
        )),
        CommandAction::RunTemplate { name, args } => {
            let template = load_template(cwd, &name)?;
            let Some(harness) = template.harness_source else {
                return Err(anyhow!(
                    "saved workflow '{name}' has no V2 harness; legacy template execution                      was removed by the workflow runtime rescue"
                ));
            };
            // QuickJS dry-run is the single grammar; failure is a hard error.
            let calls = workflow_live_v2::dry_run_workflow_plan(&harness, args.as_ref())
                .await
                .map_err(|err| anyhow!("saved workflow '{name}' failed validation: {err}"))?;
            let task = template.spec.task.clone();
            let mut plan = WorkflowScriptPlan::from_template(template.spec, &harness, calls);
            plan.script_args = args;
            plan.check_policy = check_policy.clone();
            cap_live_plan_parallelism(&mut plan, &runner, &policy);
            return workflow_live_v2::run_saved_v2_workflow(
                cwd,
                &store,
                plan,
                task,
                llm,
                ui_sink,
                runner.agent_names.clone(),
                approval_mode,
                workspace_boundary_supported,
                &learning,
            )
            .await;
        }
        CommandAction::Resume { run_id } | CommandAction::Continue { run_id } => {
            if let Some(output) = workflow_live_v2::resume_generated_v2_workflow(
                cwd,
                &store,
                &run_id,
                llm.clone(),
                ui_sink.clone(),
                runner.agent_names.clone(),
                approval_mode,
                workspace_boundary_supported,
                &learning,
            )
            .await?
            {
                return Ok(output);
            }
            let run = store.load_state(&run_id)?;
            if let Some(message) = terminal_resume_message(&run) {
                return Ok(message);
            }
            Err(anyhow!(
                "workflow {run_id} is not a resumable V2 run; legacy stage execution was                  removed by the workflow runtime rescue"
            ))
        }
        other => run_action(cwd, other),
    }
}
