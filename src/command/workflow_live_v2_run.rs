pub(super) async fn run_generated_v2_workflow(
    cwd: &Path,
    store: &WorkflowStore,
    plan: WorkflowScriptPlan,
    task: String,
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
    agent_names: Vec<String>,
    approval_mode: LiveApprovalMode,
    workspace_boundary_supported: bool,
) -> Result<String> {
    run_v2_workflow_with_origin(
        cwd,
        store,
        plan,
        task,
        llm,
        tui_tx,
        agent_names,
        approval_mode,
        workspace_boundary_supported,
        WorkflowBundleOrigin::GeneratedHarness,
    )
    .await
}

pub(super) async fn run_saved_v2_workflow(
    cwd: &Path,
    store: &WorkflowStore,
    plan: WorkflowScriptPlan,
    task: String,
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
    agent_names: Vec<String>,
    approval_mode: LiveApprovalMode,
    workspace_boundary_supported: bool,
) -> Result<String> {
    run_v2_workflow_with_origin(
        cwd,
        store,
        plan,
        task,
        llm,
        tui_tx,
        agent_names,
        approval_mode,
        workspace_boundary_supported,
        WorkflowBundleOrigin::SavedCommand,
    )
    .await
}

async fn run_v2_workflow_with_origin(
    cwd: &Path,
    store: &WorkflowStore,
    plan: WorkflowScriptPlan,
    task: String,
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
    agent_names: Vec<String>,
    approval_mode: LiveApprovalMode,
    workspace_boundary_supported: bool,
    origin: WorkflowBundleOrigin,
) -> Result<String> {
    let run = store.create_run(plan.approval_metadata_spec())?;
    WorkflowBundle::create_for_run(store, &run, &plan.harness_source, origin)?;
    save_generated_v2_metadata(store, &run.id, &plan)?;
    let run = match gate_live_approval(cwd, store, run, approval_mode, &tui_tx)? {
        LiveApprovalOutcome::Proceed { run, note } => {
            let _ = tui_tx.send(TuiEvent::TextDelta(note.clone()));
            *run
        }
        LiveApprovalOutcome::Pending(message) | LiveApprovalOutcome::Denied(message) => {
            return Ok(message);
        }
    };
    execute_generated_v2_run(
        store,
        run,
        plan,
        task,
        llm,
        tui_tx,
        agent_names,
        workspace_boundary_supported,
        false,
    )
    .await
}

pub(super) async fn resume_generated_v2_workflow(
    cwd: &Path,
    store: &WorkflowStore,
    run_id: &str,
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
    agent_names: Vec<String>,
    approval_mode: LiveApprovalMode,
    workspace_boundary_supported: bool,
) -> Result<Option<String>> {
    let run = store.load_state(run_id)?;
    let Some(plan) = live_plan_from_generated_bundle(store, &run).await? else {
        return Ok(None);
    };
    match run.status {
        RunStatus::Completed => {
            return Ok(Some(format!(
                "Workflow {} is already completed; start a new workflow run for new work.\n",
                run.id
            )));
        }
        RunStatus::Cancelled => {
            return Ok(Some(format!(
                "Workflow {} is cancelled and cannot be resumed; start a new workflow run.\n",
                run.id
            )));
        }
        _ => {}
    }
    let run = match gate_live_approval(cwd, store, run, approval_mode, &tui_tx)? {
        LiveApprovalOutcome::Proceed { run, note } => {
            let _ = tui_tx.send(TuiEvent::TextDelta(note.clone()));
            if matches!(run.status, RunStatus::Paused) {
                LifecycleController::new(store.clone()).apply(&run.id, LifecycleAction::Resume)?
            } else {
                *run
            }
        }
        LiveApprovalOutcome::Pending(message) | LiveApprovalOutcome::Denied(message) => {
            return Ok(Some(message));
        }
    };
    let task = run.spec.task.clone();
    execute_generated_v2_run(
        store,
        run,
        plan,
        task,
        llm,
        tui_tx,
        agent_names,
        workspace_boundary_supported,
        true,
    )
    .await
    .map(Some)
}

async fn live_plan_from_generated_bundle(
    store: &WorkflowStore,
    run: &WorkflowRun,
) -> Result<Option<WorkflowScriptPlan>> {
    let manifest = match WorkflowBundle::verify(store, &run.id) {
        Ok(manifest) => manifest,
        Err(err) => {
            return Err(WorkflowError::SpecInvalid(format!(
                "workflow bundle verification failed for generated V2 run '{}': {err}",
                run.id
            ))
            .into());
        }
    };
    if !matches!(
        manifest.origin,
        WorkflowBundleOrigin::GeneratedHarness | WorkflowBundleOrigin::SavedCommand
    ) {
        return Ok(None);
    }
    let harness_path = store.run_dir(&run.id).join("workflow.js");
    let harness_source = fs::read_to_string(&harness_path).map_err(|err| WorkflowError::Io {
        path: harness_path.clone(),
        source: err,
    })?;
    // One source of truth for the plan: the host-call manifest persisted at
    // approval time. Scripts without one (older saved runs) are re-planned by
    // the QuickJS dry-run; a script that fails it is a hard error.
    let metadata = load_generated_v2_metadata(store, &run.id)?;
    let calls = match metadata
        .as_ref()
        .and_then(|metadata| metadata.generated_scaffold.as_ref())
        .map(|scaffold| scaffold.host_call_manifest.clone())
        .filter(|calls| !calls.is_empty())
    {
        Some(calls) => calls,
        None => dry_run_workflow_plan(&harness_source, None).await?,
    };
    let mut script_plan =
        WorkflowScriptPlan::from_template(run.spec.clone(), &harness_source, calls);
    if let Some(metadata) = metadata {
        let current_hash = workflow_scaffold_hash(&harness_source);
        if let Some(scaffold_hash) = metadata.scaffold_hash.as_deref()
            && scaffold_hash != current_hash
        {
            return Err(WorkflowError::ArtifactInvalid(format!(
                "generated V2 scaffold hash mismatch for run '{}': metadata {}, workflow.js {}",
                run.id, scaffold_hash, current_hash
            ))
            .into());
        }
        if let Some(scaffold) = metadata.generated_scaffold.as_ref()
            && scaffold.scaffold_hash != current_hash
        {
            return Err(WorkflowError::ArtifactInvalid(format!(
                "generated V2 scaffold record hash mismatch for run '{}': metadata {}, workflow.js {}",
                run.id, scaffold.scaffold_hash, current_hash
            ))
            .into());
        }
        script_plan.task_universe = metadata.task_universe;
        script_plan.script_args = metadata.script_args;
        script_plan.governed_learning_context = metadata.governed_learning_context;
        if let Some(generated_config) = metadata.generated_config {
            script_plan.generated_config = generated_config;
        }
    }
    Ok(Some(script_plan))
}

fn save_generated_v2_metadata(
    store: &WorkflowStore,
    run_id: &str,
    plan: &WorkflowScriptPlan,
) -> archon_workflow::WorkflowResult<()> {
    let generated_scaffold = plan.generated_scaffold();
    let metadata = GeneratedV2Metadata {
        schema_version: "workflow-generated-v2-metadata-v1".to_string(),
        generated_kind: generated_scaffold.as_ref().map(|scaffold| scaffold.kind),
        scaffold_hash: Some(plan.scaffold_hash()),
        generated_scaffold,
        task_universe: plan.task_universe.clone(),
        script_args: plan.script_args.clone(),
        governed_learning_context: plan.governed_learning_context.clone(),
        generated_config: plan
            .task_universe
            .as_ref()
            .map(|_| plan.generated_config.clone()),
    };
    store.write_run_json(run_id, GENERATED_V2_METADATA_PATH, &metadata)
}

fn load_generated_v2_metadata(
    store: &WorkflowStore,
    run_id: &str,
) -> archon_workflow::WorkflowResult<Option<GeneratedV2Metadata>> {
    let path = store.run_dir(run_id).join(GENERATED_V2_METADATA_PATH);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).map_err(|err| WorkflowError::Io {
        path: path.clone(),
        source: err,
    })?;
    serde_json::from_str(&raw).map(Some).map_err(Into::into)
}

async fn execute_generated_v2_run(
    store: &WorkflowStore,
    run: WorkflowRun,
    plan: WorkflowScriptPlan,
    task: String,
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
    agent_names: Vec<String>,
    workspace_boundary_supported: bool,
    adopt_accepted_cache: bool,
) -> Result<String> {
    let adapter = WorkflowV2AgentAdapter::new();
    let runtime = WorkflowV2ScriptRuntime {
        target_repository_root: plan.target_repository_root.clone(),
        generated_config: plan.generated_config.clone(),
    };
    let provider_env_resolution =
        workflow_live_provider_env::resolve_generated_workflow_provider_env(
            plan.task_universe.as_ref(),
        )
        .await;
    let client = LiveV2AgentClient::new(
        llm,
        tui_tx.clone(),
        agent_names,
        run.id.clone(),
        runtime.target_repository_root.clone(),
        Some(u64::from(runtime.generated_config.host_call_timeout_secs)),
    )
    .with_provider_env_resolution(provider_env_resolution);
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let resume_completed_ids = if adopt_accepted_cache {
        plan.task_universe
            .as_ref()
            .map(|universe| {
                super::workflow_live_v2_completion_credit::prepare_resume_credit(
                    &v2_store,
                    universe,
                )
            })
            .transpose()?
            .unwrap_or_default()
    } else {
        Default::default()
    };
    let runner = WorkflowV2ScriptRunner::new(
        task,
        runtime,
        adapter,
        client,
        v2_store.clone(),
        store.clone(),
        run.id.clone(),
        workspace_boundary_supported,
        plan.task_universe.clone(),
        plan.script_args.clone(),
    )
    .with_frontier_resume(adopt_accepted_cache)
    .with_resume_completed_ids(resume_completed_ids);
    // Decomposed-PRD runs default to the Rust lifecycle. v3 script mode
    // (ARCHON_SCRIPT_LIFECYCLE=1) instead AUTHORS a workflow.js from the
    // task universe and executes it — composition as code, no reducer relay.
    let script_lifecycle = std::env::var("ARCHON_SCRIPT_LIFECYCLE")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let run_result = if plan.task_universe.is_some() && script_lifecycle {
        runner
            .run_authored_script_lifecycle(store.run_dir(&run.id).join("authored-workflow.js"))
            .await
    } else if plan.task_universe.is_some() {
        runner
            .run_decomposed_lifecycle(
                &plan.harness_source,
                serde_json::to_value(&plan.governed_learning_context)
                    .unwrap_or(serde_json::Value::Array(Vec::new())),
            )
            .await
    } else {
        runner.run(&plan.harness_source).await
    };
    let summary = match run_result {
        Ok(summary) => summary,
        Err(WorkflowError::ControlPaused(message)) => {
            return Ok(format!(
                "Workflow paused: {}\n{}\nResume with: /workflow resume --live {}\n",
                run.id, message, run.id
            ));
        }
        Err(WorkflowError::ControlCancelled(message)) => {
            return Ok(format!("Workflow cancelled: {}\n{}\n", run.id, message));
        }
        Err(err) => return Err(err.into()),
    };

    sync_v2_summary_to_run(store, &run.id, &summary.calls, &v2_store, summary.status)?;
    let learning_note = record_generated_learning_event(store, &run.id, &plan, &summary, &v2_store)
        .map(|path| format!("generated_learning: {}\n", path.display()))
        .unwrap_or_else(|err| format!("generated_learning: degraded ({err})\n"));
    let status_label = match summary.status {
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop => "complete",
        WorkflowV2Status::NeedsReview => "needs review",
        WorkflowV2Status::Failed | WorkflowV2Status::Blocked | WorkflowV2Status::Cancelled => {
            "failed"
        }
        WorkflowV2Status::Pending | WorkflowV2Status::Running => "stopped",
    };
    let mut output = format!(
        "Workflow V2 {status_label}: {} (status {:?}, completed {}, executed {}, reused {})\n",
        run.id, summary.status, summary.completed, summary.executed, summary.reused
    );
    if let Some(call_id) = &summary.failed_call {
        output.push_str(&format!("failed_call: {call_id}\n"));
    }
    if let Some(path) = &summary.failed_result_path {
        output.push_str(&format!("failed_result: {path}\n"));
    }
    if let Some(next_action) = &summary.next_action {
        output.push_str(&format!("next_action: {next_action}\n"));
    }
    output.push_str(&format!(
        "harness: {}\nv2_results: {}\n",
        store.run_dir(&run.id).join("workflow.js").display(),
        v2_store.root().display()
    ));
    output.push_str(&learning_note);
    Ok(output)
}
