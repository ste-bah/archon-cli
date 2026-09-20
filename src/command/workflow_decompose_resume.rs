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
    resume_fixed_decomposition_at_binary_revision(
        cwd,
        run_id,
        yes,
        config,
        env_vars,
        factory,
        ui_sink,
        interactive_owner,
        cancellation_requested,
        env!("ARCHON_GIT_HASH"),
    )
    .await
}

/// The resume proper, with the running build's revision as a parameter so a
/// test can resume a run launched by this binary as if by an upgraded one.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn resume_fixed_decomposition_at_binary_revision(
    cwd: &Path,
    run_id: &str,
    yes: bool,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    factory: &dyn WorkflowLlmClientFactory,
    ui_sink: archon_workflow::SharedWorkflowUiSink,
    interactive_owner: Option<&str>,
    cancellation_requested: Option<&std::sync::atomic::AtomicBool>,
    current_binary_revision: &str,
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
    let _execution_lease =
        crate::command::workflow_task_root_reclaim::begin_execution(&store, run_id)?;
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
    // The catalog is rebuilt under the launch revision, not the running one:
    // its digest hashes `starting_binary_revision`, and per-call ids are keyed
    // on it, so building it from the current build would fail the digest
    // comparison and orphan every persisted per-call result on an upgraded
    // binary. What the comparison must detect is a changed capability set,
    // and that still shows up (Issue-59).
    let catalog = fixed_decomposition_catalog(&state.identity.starting_binary_revision)?;
    let current_identity = FixedRunIdentityV1 {
        template_version: FIXED_DECOMPOSITION_TEMPLATE_VERSION.to_string(),
        starting_binary_revision: current_binary_revision.to_string(),
        script_digest: workflow_scaffold_hash(FIXED_SCRIPT_SOURCE),
        catalog_digest: catalog.digest.clone(),
        project_root_identity: path_text(&canonical_persisted_project),
        prd_identity: path_text(&prd_path),
        task_root_identity: path_text(&task_root),
    };
    let binary_drift =
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
    let (_, prd_digest, acceptance_criteria) =
        super::super::workflow_task_set::validate_prd_input(&prd_path)?;
    let arguments: serde_json::Value = read_run_json(&store, run_id, FIXED_ARGUMENTS_PATH)?;
    // The frozen chain is the launch-time reading of the task root, bound
    // into the run like every other argument: the script skipped stages on
    // its word, so a resume replays against the same word, not a fresh read
    // of a root the run has since written to.
    let frozen_chain = arguments
        .get("frozenChain")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| {
            anyhow!("fixed decomposition persisted arguments carry no frozenChain object")
        })?;
    // The repository the launch grounded the authors in is read back from the
    // task root's record, never from a flag or config: a resume replays the
    // launch's word, and the record is that word.
    let repository_root = archon_workflow::repository_record::read_repository_record(&task_root)?
        .map(|record| PathBuf::from(record.repository_root))
        .ok_or_else(|| {
            anyhow!(
                "fixed decomposition {run_id} task root {} carries no {}; the launch that created the run recorded one, so the task root changed underneath the run",
                task_root.display(),
                archon_workflow::repository_record::REPOSITORY_LOCK_FILE
            )
        })?;
    let expected_arguments = super::fixed_script_arguments(
        &project_root,
        &prd_path,
        &prd_digest,
        acceptance_criteria,
        config,
        &task_root,
        &repository_root,
        frozen_chain,
    );
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
    let log_path = crate::command::workflow_decompose_log::validated_fixed_log_path(
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
    // Every launch artifact has verified intact by here; the running build's
    // revision is the one tolerated deviation, and it is recorded rather than
    // refused. The persisted identity is left as the launch record.
    if let Some(drift) = &binary_drift {
        crate::command::workflow_decompose_events::emit_binary_revision_drift(
            &store, run_id, &log_path, drift,
        )?;
        ui_sink
            .emit(WorkflowUiEvent::Text(format!(
                "Binary revision drift: persisted={} current={}\n",
                drift.persisted, drift.current
            )))
            .await
            .map_err(|error| anyhow!("reporting binary revision drift: {error}"))?;
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
            read_roots: crate::command::workflow_read_scope::read_roots(
                &project_root,
                run.spec.target_repository_root.as_deref(),
            ),
        })
        .await
        .context("building the fixed decomposition resume provider client")?;
    if let Some(root) = run.spec.target_repository_root.as_deref() {
        crate::command::workflow_read_scope::require_agent_read(
            client.as_ref(),
            Path::new(root),
            "the decomposition authors",
        )?;
    }
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
