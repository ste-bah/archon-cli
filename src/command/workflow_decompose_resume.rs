use super::*;

pub(crate) async fn resume_fixed_decomposition_with_factory(
    cwd: &Path,
    run_id: &str,
    yes: bool,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    factory: &dyn WorkflowLlmClientFactory,
) -> Result<String> {
    resume_fixed_decomposition_with_factory_and_sink(
        cwd,
        run_id,
        yes,
        config,
        env_vars,
        factory,
        crate::command::workflow_decompose_progress::DecompositionCliUiSink::shared(),
        None,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn resume_fixed_decomposition_with_factory_and_sink(
    cwd: &Path,
    run_id: &str,
    yes: bool,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    factory: &dyn WorkflowLlmClientFactory,
    ui_sink: archon_workflow::SharedWorkflowUiSink,
    interactive_owner: Option<&str>,
    cancellation_requested: Option<&std::sync::atomic::AtomicBool>,
) -> Result<String> {
    if !yes {
        return Err(anyhow!(
            "fixed workflow resume is a live operation and requires --yes"
        ));
    }
    if config.workflow.gate_mode == GateMode::Off {
        return Err(anyhow!(DECOMPOSE_GATE_OFF_REMEDY));
    }
    let project_root = canonical_existing(cwd, "project root")?;
    let store = WorkflowStore::project(&project_root);
    let run = store.load_state(run_id)?;
    if run.status == archon_workflow::RunStatus::Completed {
        return Err(anyhow!(
            "fixed decomposition {run_id} is already completed; start a new decomposition in a fresh task root"
        ));
    }
    if !matches!(
        run.status,
        archon_workflow::RunStatus::Paused | archon_workflow::RunStatus::Cancelled
    ) {
        return Err(anyhow!(
            "fixed decomposition {run_id} can resume only from paused or cancelled status, found {:?}",
            run.status
        ));
    }
    crate::command::workflow_decompose_owner::require_owner(&store, run_id, interactive_owner)?;
    let state = read_fixed_state(&store, run_id)?;
    if state.run_kind != WorkflowRunKind::FixedDecompositionV1 {
        return Err(anyhow!(
            "workflow {run_id} is not a FixedDecompositionV1 run"
        ));
    }
    archon_workflow::WorkflowBundle::verify(&store, run_id)?;
    let compiled_path = store
        .run_dir(run_id)
        .join(archon_workflow::bundle::COMPILED_SPEC_FILE);
    let compiled_spec: archon_workflow::WorkflowSpec =
        serde_yaml_ng::from_str(&std::fs::read_to_string(&compiled_path).with_context(|| {
            format!(
                "reading verified fixed workflow spec {}",
                compiled_path.display()
            )
        })?)?;
    if run.spec != compiled_spec {
        return Err(anyhow!(
            "fixed decomposition mutable run spec differs from the verified workflow bundle"
        ));
    }
    let prd_path = canonical_existing(Path::new(&state.identity.prd_identity), "persisted PRD")?;
    let task_root = canonical_existing(
        Path::new(&state.identity.task_root_identity),
        "persisted task root",
    )?;
    let canonical_persisted_project = canonical_existing(
        Path::new(&state.identity.project_root_identity),
        "persisted project root",
    )?;
    let catalog = fixed_decomposition_catalog(env!("ARCHON_GIT_HASH"))?;
    let current_identity = FixedRunIdentityV1 {
        template_version: FIXED_DECOMPOSITION_TEMPLATE_VERSION.to_string(),
        starting_binary_revision: env!("ARCHON_GIT_HASH").to_string(),
        script_digest: workflow_scaffold_hash(FIXED_SCRIPT_SOURCE),
        catalog_digest: catalog.digest.clone(),
        project_root_identity: path_text(&canonical_persisted_project),
        prd_identity: path_text(&prd_path),
        task_root_identity: path_text(&task_root),
    };
    archon_workflow::verify_fixed_resume_identity(&state.identity, &current_identity)?;
    if canonical_persisted_project != project_root {
        return Err(anyhow!(
            "fixed decomposition resume project root differs from the invoking project; run the command from {}",
            canonical_persisted_project.display()
        ));
    }
    let recorded_source =
        std::fs::read_to_string(archon_workflow::bundle::record_path(&store.run_dir(run_id)))?;
    if recorded_source != FIXED_SCRIPT_SOURCE {
        return Err(anyhow!(
            "fixed decomposition recorded source differs from the embedded script; do not deploy or replace the binary while a decomposition is active"
        ));
    }
    let (_, prd_digest, acceptance_criteria) = super::super::workflow_task_set::validate_prd_input(&prd_path)?;
    let expected_arguments = serde_json::json!({
        "projectRoot": path_text(&project_root),
        "prdPath": path_text(&prd_path),
        "prdDigest": prd_digest.clone(),
        "acceptanceCriteria": acceptance_criteria,
        "excludedDirs": archon_leann::language::default_exclude_patterns(),
        "taskRoot": path_text(&task_root),
        "gateMode": gate_mode_text(config.workflow.gate_mode),
    });
    let arguments: serde_json::Value = read_run_json(&store, run_id, FIXED_ARGUMENTS_PATH)?;
    if arguments != expected_arguments {
        return Err(anyhow!(
            "fixed decomposition persisted arguments differ from the launch-bound canonical arguments"
        ));
    }
    let persisted_catalog: archon_workflow::CommandCapabilityCatalog =
        read_run_json(&store, run_id, FIXED_CATALOG_PATH)?;
    if persisted_catalog != catalog {
        return Err(anyhow!(
            "fixed decomposition persisted command catalog differs from the current embedded catalog"
        ));
    }
    let current_route = super::super::workflow_provider_route::resolve_anthropic_route(
        config.api.base_url.as_deref(),
        super::super::workflow_provider_route::ProviderEndpointPolicy::ConfiguredOnly,
    );
    let persisted_route: super::super::workflow_provider_route::TrustedProviderRouteSnapshot =
        read_run_json(&store, run_id, FIXED_PROVIDER_ROUTE_PATH)?;
    if persisted_route != current_route {
        return Err(anyhow!(
            "fixed decomposition provider route differs from its launch snapshot; restore the original trusted configuration before resume"
        ));
    }
    crate::command::workflow_decompose_log::validated_fixed_log_path(
        Path::new(&state.log_path),
        &state.identity,
    )?;
    let expected_metadata = serde_json::json!({
        "schema_version": "workflow-generated-v2-metadata-v1",
        "run_kind": "fixed_decomposition_v1",
        "fixed_identity": state.identity,
        "scaffold_hash": workflow_scaffold_hash(FIXED_SCRIPT_SOURCE),
        "script_args": expected_arguments,
        "script_lifecycle": true,
    });
    let metadata: serde_json::Value = read_run_json(&store, run_id, FIXED_GENERATED_METADATA_PATH)?;
    if metadata != expected_metadata {
        return Err(anyhow!(
            "fixed decomposition generated metadata differs from its canonical launch snapshot"
        ));
    }
    let launch_digest = super::fixed_launch_digest(
        &state.identity,
        &arguments,
        &persisted_catalog,
        &persisted_route,
    )?;
    let anchored_digest = compiled_spec
        .permissions
        .get(FIXED_LAUNCH_DIGEST_PERMISSION)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("verified fixed workflow spec has no launch digest anchor"))?;
    if launch_digest != anchored_digest {
        return Err(anyhow!(
            "fixed decomposition launch snapshot differs from the verified workflow bundle anchor"
        ));
    }
    let calls = archon_workflow::v2::script::dry_run_workflow_plan(
        FIXED_SCRIPT_SOURCE,
        Some(&expected_arguments),
    )
    .await?;
    let plan = super::super::workflow_live::workflow_live_planner::WorkflowScriptPlan::fixed(
        compiled_spec,
        FIXED_SCRIPT_SOURCE,
        calls,
        expected_arguments,
    );
    if cancellation_requested
        .is_some_and(|requested| requested.load(std::sync::atomic::Ordering::SeqCst))
    {
        let current = store.load_state(run_id)?;
        if !matches!(
            current.status,
            archon_workflow::RunStatus::Completed
                | archon_workflow::RunStatus::Cancelled
                | archon_workflow::RunStatus::Failed
                | archon_workflow::RunStatus::Blocked
        ) {
            archon_workflow::LifecycleController::new(store.clone())
                .apply(run_id, archon_workflow::LifecycleAction::Cancel)?;
        }
        return Err(anyhow!(
            "fixed decomposition resume cancelled before provider construction"
        ));
    }

    let program = std::env::current_exe()
        .context("resolving the fixed decomposition binary")?
        .canonicalize()
        .context("canonicalizing the fixed decomposition binary")?;
    let executor = Arc::new(
        crate::command::workflow_host_command_exec::FixedHostCommandExecutor::new(
            catalog,
            crate::command::workflow_host_command_catalog::HostCommandResolutionContext {
                program,
                project_root: project_root.clone(),
                prd_path,
                prd_digest,
                task_root,
                run_staging_root: store.run_dir(run_id).join("host-command-staging"),
                frozen_task_id: None,
                frozen_task_file: None,
                freeze_provider_environment: freeze_provider_environment(env_vars),
                gate_mode: config.workflow.gate_mode,
            },
            store.run_dir(run_id),
        ),
    );
    let agent_names = AgentRegistry::load(&project_root)
        .available_agent_names()
        .into_iter()
        .map(str::to_string)
        .collect();
    let client = factory
        .build_client(WorkflowLlmClientRequest {
            cwd: project_root.clone(),
            origin: "workflow_decompose_v1".to_string(),
            session_id: run.id.clone(),
        })
        .await
        .context("building the fixed decomposition resume provider client")?;
    let lifecycle = archon_workflow::LifecycleController::new(store.clone());
    let run = lifecycle
        .apply(run_id, archon_workflow::LifecycleAction::Resume)
        .context("applying fixed decomposition resume lifecycle")?;
    if let Err(log_error) = crate::command::workflow_decompose_log::append_fixed_log_marker(
        Path::new(&state.log_path),
        "resume",
        run_id,
        &state.identity,
    ) {
        let cancellation = lifecycle.apply(run_id, archon_workflow::LifecycleAction::Cancel);
        return match cancellation {
            Ok(_) => Err(log_error.into()),
            Err(cancel_error) => Err(anyhow!(
                "fixed decomposition resume log failed ({log_error}); terminal cancellation also failed ({cancel_error})"
            )),
        };
    }
    if let Err(owner_error) = crate::command::workflow_decompose_owner::record_action(
        &store,
        run_id,
        interactive_owner,
        "resume",
    ) {
        let cancellation = lifecycle.apply(run_id, archon_workflow::LifecycleAction::Cancel);
        return match cancellation {
            Ok(_) => Err(owner_error.into()),
            Err(cancel_error) => Err(anyhow!(
                "fixed decomposition resume ownership evidence failed ({owner_error}); terminal cancellation also failed ({cancel_error})"
            )),
        };
    }
    super::super::workflow_live::execute_fixed_decomposition_v2_run(
        &store,
        run,
        plan,
        client,
        ui_sink,
        agent_names,
        executor,
    )
    .await
}
