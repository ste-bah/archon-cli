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
    let execution_lease =
        crate::command::workflow_task_root_reclaim::begin_execution(&store, run_id)?;
    let mut run = store.load_state(run_id)?;
    if run.status == archon_workflow::RunStatus::Completed {
        return Err(anyhow!(
            "fixed decomposition {run_id} is already completed; start a new decomposition in a fresh task root"
        ));
    }
    crate::command::workflow_decompose_owner::require_owner(&store, run_id, interactive_owner)?;
    let state = match read_fixed_state(&store, run_id) {
        Ok(state) => state,
        Err(error) => {
            // The lease proves the old executor is gone. An unreadable
            // decomposition frontier still pauses, never fails, and through
            // the dead-owner recovery: its event and the generation bump
            // that fences the dead executor's work. The state names no log.
            if run.status == archon_workflow::RunStatus::Running {
                let ended_groups =
                    crate::command::workflow_host_command_groups::require_no_running_groups(
                        &store.run_dir(run_id),
                        run_id,
                    )
                    .map_err(|groups| {
                        groups.context(format!(
                            "the decomposition state is also unreadable: {error:#}"
                        ))
                    })?;
                if let Some(recovery) =
                    crate::command::workflow_decompose_stale_owner::recover_dead_generic_owner(
                        &store,
                        run_id,
                        &execution_lease,
                        &ended_groups,
                    )?
                {
                    ui_sink
                        .emit(WorkflowUiEvent::Text(recovery.summary(run_id)))
                        .await
                        .map_err(|error| anyhow!("reporting stale owner recovery: {error}"))?;
                }
            }
            return Err(error);
        }
    };
    if state.run_kind != WorkflowRunKind::FixedDecompositionV1 {
        return Err(anyhow!(
            "workflow {run_id} is not a FixedDecompositionV1 run"
        ));
    }
    // Any group record left here is from a dead executor: this process holds
    // the lease, and a live executor's groups end before it releases it.
    let ended_groups = crate::command::workflow_host_command_groups::require_no_running_groups(
        &store.run_dir(run_id),
        run_id,
    )?;
    if run.status == archon_workflow::RunStatus::Running {
        // The lease is held, so the kernel says no live process executes
        // this run: its owner died without a pause (Issue 251). Recorded,
        // then resumed through the paused path.
        let log_path = crate::command::workflow_decompose_log::validated_fixed_log_path(
            Path::new(&state.log_path),
            &state.identity,
        )?;
        if let Some(recovery) = crate::command::workflow_decompose_stale_owner::recover_dead_owner(
            &store,
            run_id,
            &execution_lease,
            &log_path,
            &ended_groups,
        )? {
            ui_sink
                .emit(WorkflowUiEvent::Text(recovery.summary(run_id)))
                .await
                .map_err(|error| anyhow!("reporting stale owner recovery: {error}"))?;
        }
        run = store.load_state(run_id)?;
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
    // Keep the launch revision as the call-key namespace. The capabilities
    // come from this build; unchanged ones keep their legacy result keys.
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
    archon_workflow::verify_fixed_resume_identity(&state.identity, &current_identity)?;
    if canonical_persisted_project != project_root {
        return Err(anyhow!(
            "fixed decomposition resume project root differs from the invoking project; run the command from {}",
            canonical_persisted_project.display()
        ));
    }
    let recorded_source =
        std::fs::read_to_string(archon_workflow::bundle::record_path(&store.run_dir(run_id)))?;
    if workflow_scaffold_hash(&recorded_source) != state.identity.script_digest {
        return Err(anyhow!(
            "fixed decomposition resume paused: identity.script_digest does not match workflow.js; restore the intact launch bundle before resume"
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
            upgrade::unmapped(
                "decomposition/arguments.json.frozenChain",
                "missing or not an object",
            )
        })?;
    let _: crate::command::workflow_decompose_frozen_chain::FrozenChainSnapshot = upgrade::decode(
        frozen_chain.clone(),
        "decomposition/arguments.json.frozenChain",
    )?;
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
    upgrade::require_equal(
        &arguments,
        &expected_arguments,
        "decomposition/arguments.json",
    )?;
    let persisted_catalog: archon_workflow::CommandCapabilityCatalog =
        read_run_json(&store, run_id, FIXED_CATALOG_PATH)?;
    // The schemas this build reads, from the build's own catalog schema.
    if !crate::command::workflow_host_command_exec::identity::catalog_schema_readable(
        persisted_catalog.schema_version,
        catalog.schema_version,
    ) {
        return Err(upgrade::unmapped(
            "command-catalog.schema_version",
            &format!(
                "found {}; this binary reads schemas 1..={}; install a compatible binary or migrate this catalog",
                persisted_catalog.schema_version, catalog.schema_version
            ),
        ));
    }
    let mut verified_catalog = persisted_catalog.clone();
    verified_catalog.recompute_digest()?;
    if verified_catalog.digest != persisted_catalog.digest
        || persisted_catalog.digest != state.identity.catalog_digest
        || persisted_catalog.starting_binary_revision != state.identity.starting_binary_revision
    {
        return Err(anyhow!(
            "fixed decomposition resume paused: command-catalog.digest or starting_binary_revision differs from identity; restore the intact launch catalog before resume"
        ));
    }
    let current_route = super::super::workflow_provider_route::resolve_anthropic_route(
        config.api.base_url.as_deref(),
        super::super::workflow_provider_route::ProviderEndpointPolicy::ConfiguredOnly,
    );
    let persisted_route: super::super::workflow_provider_route::TrustedProviderRouteSnapshot =
        read_run_json(&store, run_id, FIXED_PROVIDER_ROUTE_PATH)?;
    upgrade::require_equal(
        &serde_json::to_value(&persisted_route)?,
        &serde_json::to_value(&current_route)?,
        "provider route",
    )?;
    let log_path = crate::command::workflow_decompose_log::validated_fixed_log_path(
        Path::new(&state.log_path),
        &state.identity,
    )?;
    let expected_metadata = serde_json::json!({
        "schema_version": "workflow-generated-v2-metadata-v1",
        "run_kind": "fixed_decomposition_v1",
        "fixed_identity": state.identity,
        "scaffold_hash": state.identity.script_digest,
        "script_args": expected_arguments,
        "script_lifecycle": true,
    });
    let metadata: serde_json::Value = read_run_json(&store, run_id, FIXED_GENERATED_METADATA_PATH)?;
    upgrade::require_equal(&metadata, &expected_metadata, "generated metadata")?;
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
    // Verify readable execution state before any provider or lifecycle change.
    upgrade::validate_result_state(&store, run_id)?;
    let transitions = upgrade::record_upgrade(
        &store,
        run_id,
        &log_path,
        &state.identity,
        &current_identity,
    )?;
    for line in transitions.iter().flat_map(|t| t.summary()) {
        ui_sink
            .emit(WorkflowUiEvent::Text(line))
            .await
            .map_err(|error| anyhow!("reporting the runtime transition: {error}"))?;
    }
    // Issue 360: after an upgrade the script starts from the phase seed. The
    // criteria texts are the host-owned criterion the script stamps (#357).
    let criteria = expected_arguments["acceptanceCriteria"]
        .as_object()
        .map(|criteria| {
            criteria
                .iter()
                .filter_map(|(id, text)| Some((id.clone(), text.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let seed = crate::command::workflow_decompose_seed::current_seed(
        &store, run_id, &log_path, &criteria,
    )?;
    let script_arguments = crate::command::workflow_decompose_seed::seeded_arguments(
        &expected_arguments,
        seed.as_ref(),
    );
    let calls = archon_workflow::v2::script::dry_run_workflow_plan(
        FIXED_SCRIPT_SOURCE,
        Some(&script_arguments),
    )
    .await?;
    let plan = super::super::workflow_live::workflow_live_planner::WorkflowScriptPlan::fixed(
        compiled_spec,
        FIXED_SCRIPT_SOURCE,
        calls,
        script_arguments,
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
        .map(archon_shell::paths::plain)
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
                acceptance_environment_allowlist: config
                    .workflow
                    .acceptance_execution
                    .as_ref()
                    .map(|policy| policy.environment_allowlist.clone())
                    .unwrap_or_default(),
                gate_mode: config.workflow.gate_mode,
            },
            store.run_dir(run_id),
        )
        .with_launch_catalog(persisted_catalog),
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
    // A run whose launch failed before any work gave its task root back;
    // it takes the root again here, unless another run has claimed it since.
    super::claim::reclaim_released_task_root(&store, run_id)?;
    // Every check has passed: from here this process executes the run.
    execution_lease.record_executor()?;
    upgrade::mark_started(&store, run_id)?;
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

#[path = "workflow_decompose_upgrade.rs"]
pub(super) mod upgrade;
