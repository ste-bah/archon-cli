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
    script_lifecycle: bool,
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
        script_lifecycle,
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
        script_lifecycle_from_env(),
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
    script_lifecycle: bool,
) -> Result<String> {
    let run = store.create_run(plan.approval_metadata_spec())?;
    WorkflowBundle::create_for_run(store, &run, &plan.harness_source, origin)?;
    save_generated_v2_metadata(store, &run.id, &plan, script_lifecycle)?;
    let run = match gate_live_approval(cwd, store, run, approval_mode, &tui_tx).await? {
        LiveApprovalOutcome::Proceed(run) => *run,
        LiveApprovalOutcome::Pending(message) | LiveApprovalOutcome::Denied(message) => {
            return Ok(message);
        }
    };
    let run_id = run.id.clone();
    let result = execute_generated_v2_run(
        store,
        run,
        plan,
        task.clone(),
        llm,
        tui_tx,
        agent_names,
        workspace_boundary_supported,
        false,
    )
    .await;
    fold_run_topology(cwd, store, &run_id, &task).await;
    result
}

/// Project a finished workflow run into the topology corpus and the learning
/// stack.
///
/// Graph completion is the trigger the design names, and this is it for
/// `/workflow`: the run's `events.jsonl` becomes a topology trace and a single
/// batched fold writes `.archon/topology.db` plus one `learning_events`
/// summary row; then the learning bridge writes the run's record stream and
/// routes it by the spec's `learning_hooks` into `LearningIntegration`.
///
/// Runs on `spawn_blocking` because every part is synchronous and the Cozo
/// write guard's retry loop sleeps on `thread::sleep` — roughly 19 seconds
/// worst case, which on a tokio worker is a runtime stall.
///
/// Entirely best-effort: a failure to record must never change what the user's
/// run reports.
async fn fold_run_topology(cwd: &Path, store: &WorkflowStore, run_id: &str, task: &str) {
    let cwd = cwd.to_path_buf();
    let store = store.clone();
    let run_id = run_id.to_string();
    let task = task.to_string();
    let _ = tokio::task::spawn_blocking(move || {
        crate::command::topology_trace::project_workflow_run(&cwd, &store, &run_id);
        crate::command::topology_fold::fold_project_pending_blocking(
            &cwd, &run_id, &task, "default",
        );
        crate::command::topology_fold::bridge_workflow_learning(&cwd, &store, &run_id);
    })
    .await;
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
    // A cancelled run IS resumable: its accepted call results are persisted
    // in the result-store frontier, so resuming re-runs only the work that
    // did not complete (Resume resets cancelled stages/items to Pending).
    // Only a genuinely finished run (Completed) refuses resume.
    if run.status == RunStatus::Completed {
        return Ok(Some(format!(
            "Workflow {} is already completed; start a new workflow run for new work.\n",
            run.id
        )));
    }
    let run = match gate_live_approval(cwd, store, run, approval_mode, &tui_tx).await? {
        LiveApprovalOutcome::Proceed(run) => {
            if matches!(run.status, RunStatus::Paused | RunStatus::Cancelled) {
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
    let run_id = run.id.clone();
    let result = execute_generated_v2_run(
        store,
        run,
        plan,
        task.clone(),
        llm,
        tui_tx,
        agent_names,
        workspace_boundary_supported,
        true,
    )
    .await;
    fold_run_topology(cwd, store, &run_id, &task).await;
    result.map(Some)
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
    script_lifecycle: bool,
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
        // Only task-universe runs can enter the authored-script lifecycle.
        script_lifecycle: Some(script_lifecycle && plan.task_universe.is_some()),
    };
    store.write_run_json(run_id, GENERATED_V2_METADATA_PATH, &metadata)
}

/// The ARCHON_SCRIPT_LIFECYCLE env decision, in one place so creation and the
/// fallback on continue agree.
pub(super) fn script_lifecycle_from_env() -> bool {
    // v3 authored-script lifecycle is the DEFAULT. The decomposed (v1) engine is
    // opt-in only via ARCHON_SCRIPT_LIFECYCLE=0/false — otherwise a run silently
    // fell back to decomposed (old monolithic review) whenever the flag wasn't
    // read at creation, which is a footgun. Absent var => v3.
    std::env::var("ARCHON_SCRIPT_LIFECYCLE")
        .map(|value| !(value == "0" || value.eq_ignore_ascii_case("false")))
        .unwrap_or(true)
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
                    &v2_store, universe,
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
    // A CONTINUED run MUST use the lifecycle it was created with (persisted in
    // metadata): re-reading the env var here silently switches a v3 run to
    // decomposed when the flag is absent, and the decomposed engine cannot
    // reuse the v3 run's records — it re-does everything under a different
    // engine. Persisted choice wins; the env var is only the fallback for a
    // run that predates this field.
    let script_lifecycle = load_generated_v2_metadata(store, &run.id)
        .ok()
        .flatten()
        .and_then(|metadata| metadata.script_lifecycle)
        // Legacy runs created before the persisted field: a v3 run leaves an
        // authored-workflow.js in its run dir — detect it so those continue as
        // v3 too, rather than falling back to the env var and switching engine.
        .or_else(|| {
            store
                .run_dir(&run.id)
                .join("authored-workflow.js")
                .exists()
                .then_some(true)
        })
        .unwrap_or_else(script_lifecycle_from_env);
    if let Some(root) = plan.target_repository_root.as_deref() {
        let trimmed = root.trim();
        if !trimmed.is_empty() && !std::path::Path::new(trimmed).join(".git").exists() {
            return Err(WorkflowError::SpecInvalid(format!(
                "write repository root '{trimmed}' is not a git repository (no .git); every write branch would fail 'Not a git repository'. Point the repository/write root at the git working tree, not the artifact/--target project"
            )).into());
        }
    }
    if script_lifecycle && plan.task_universe.is_none() {
        return Err(WorkflowError::SpecInvalid(format!(
            "generated V2 run '{}' was created for the v3 authored-script lifecycle but has no persisted task universe; refusing to execute scaffold workflow.js",
            run.id
        ))
        .into());
    }
    let run_result = if plan.task_universe.is_some() && script_lifecycle {
        runner
            .run_authored_script_lifecycle(
                store.run_dir(&run.id).join("authored-workflow.js"),
                serde_json::to_value(&plan.governed_learning_context)
                    .unwrap_or(serde_json::Value::Array(Vec::new())),
            )
            .await
    } else if plan.task_universe.is_some() {
        // TODO(remove-decomposed): delete the legacy native lifecycle after v3 live proof.
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
        // Every early return below must reconcile state.json first: these paths
        // skip sync_v2_summary_to_run, so without this the run's final status
        // exists only in events.jsonl while state.json still says Running.
        Err(WorkflowError::ControlPaused(message)) => {
            persist_terminal_run_status(store, &run.id, RunStatus::Paused)?;
            return Ok(format!(
                "Workflow paused: {}\n{}\nResume with: /workflow resume --live {}\n",
                run.id, message, run.id
            ));
        }
        Err(WorkflowError::ControlCancelled(message)) => {
            persist_terminal_run_status(store, &run.id, RunStatus::Cancelled)?;
            return Ok(format!("Workflow cancelled: {}\n{}\n", run.id, message));
        }
        Err(err) => {
            // Best-effort: the original error is what the caller must see, so a
            // failure to persist here is logged rather than masking it.
            if let Err(state_err) =
                persist_terminal_run_status(store, &run.id, RunStatus::Failed)
            {
                tracing::warn!(
                    run_id = %run.id,
                    error = %state_err,
                    "failed to persist terminal run status after run error"
                );
            }
            return Err(err.into());
        }
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
