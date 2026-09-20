//! First-class launcher for the immutable engine-native decomposition run.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use archon_core::agents::AgentRegistry;
use archon_core::config::{ArchonConfig, GateMode};
use archon_core::env_vars::ArchonEnvVars;
use archon_workflow::{
    DecompositionPhase, FIXED_DECOMPOSITION_STATE_SCHEMA_VERSION,
    FIXED_DECOMPOSITION_TEMPLATE_VERSION, FixedDecompositionStateV1, FixedRunIdentityV1,
    SharedWorkflowUiSink, WorkflowBundle, WorkflowBundleOrigin, WorkflowLlmClientFactory,
    WorkflowLlmClientRequest, WorkflowRunKind, WorkflowSpec, WorkflowStore, WorkflowUiEvent,
    workflow_scaffold_hash,
};

use super::workflow_host_command_catalog::fixed_decomposition_catalog;

/// The embedded script, one source: the main file first, then the helpers
/// hoisted into its scope. Every `.js` beside it is part of the identity the
/// script digest pins, so a change to any of them is a change to the runtime.
pub(crate) const FIXED_SCRIPT_SOURCE: &str = concat!(
    include_str!("workflow_decompose_v1.js"),
    "\n",
    include_str!("workflow_decompose_v1_acceptance.js"),
    "\n",
    include_str!("workflow_decompose_v1_set_gate.js"),
);
pub(crate) const FIXED_DECOMPOSITION_STATE_PATH: &str = "decomposition/state.json";
pub(crate) const FIXED_CATALOG_PATH: &str = "decomposition/command-catalog.json";
pub(crate) const FIXED_ARGUMENTS_PATH: &str = "decomposition/arguments.json";
pub(crate) const FIXED_PROVIDER_ROUTE_PATH: &str = "decomposition/provider-route.json";
pub(crate) const FIXED_GENERATED_METADATA_PATH: &str = "v2/generated-metadata.json";
pub(crate) const FIXED_LAUNCH_DIGEST_PERMISSION: &str = "archon.fixed_decomposition_launch_digest";
pub(crate) const DECOMPOSE_GATE_OFF_REMEDY: &str = "workflow decompose requires workflow.gate_mode=observe or enforce; set [workflow] gate_mode = \"observe\" and retry";

/// `repository` is the `--repository` flag; `None` falls through to the
/// configured sources (see `workflow_decompose_repository`), never to `cwd`.
pub(crate) async fn run_fixed_decomposition_with_factory(
    cwd: &Path,
    prd: &Path,
    tasks: &Path,
    repository: Option<&Path>,
    yes: bool,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    factory: &dyn WorkflowLlmClientFactory,
) -> Result<String> {
    run_fixed_decomposition_with_factory_and_sink(
        cwd,
        prd,
        tasks,
        repository,
        yes,
        config,
        env_vars,
        factory,
        crate::command::workflow_decompose_progress::DecompositionCliUiSink::shared(),
        None,
        None,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_fixed_decomposition_with_factory_and_sink(
    cwd: &Path,
    prd: &Path,
    tasks: &Path,
    repository: Option<&Path>,
    yes: bool,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    factory: &dyn WorkflowLlmClientFactory,
    ui_sink: SharedWorkflowUiSink,
    persisted_run_id: Option<&std::sync::Mutex<Option<String>>>,
    interactive_owner: Option<&str>,
    cancellation_requested: Option<&std::sync::atomic::AtomicBool>,
) -> Result<String> {
    if !yes {
        return Err(anyhow!(
            "workflow decompose is a live operation and requires --yes for CLI execution"
        ));
    }
    if config.workflow.gate_mode == GateMode::Off {
        return Err(anyhow!(DECOMPOSE_GATE_OFF_REMEDY));
    }

    let project_root = canonical_existing(cwd, "project root")?;
    let prd_path = canonical_project_path(&project_root, prd, "PRD path")?;
    let task_root = canonical_project_path(&project_root, tasks, "task root")?;
    // The repository the authors read (Issue-55): flag, config, or refusal.
    // Resolved before anything is written so a missing or wrong repository
    // stops the launch with no run and no task-root claim.
    let repository = super::workflow_decompose_repository::resolve_repository(
        &project_root,
        repository,
        config,
    )?;
    let (_, prd_digest, acceptance_criteria) = super::workflow_task_set::validate_prd_input(&prd_path)?;
    let starting_binary_revision = env!("ARCHON_GIT_HASH").to_string();
    let catalog = fixed_decomposition_catalog(&starting_binary_revision)?;
    let script_digest = workflow_scaffold_hash(FIXED_SCRIPT_SOURCE);
    // What the task root already holds of the frozen chain, verified now. A
    // lock that does not verify stops the launch here, before a run exists.
    let frozen_chain = crate::command::workflow_decompose_frozen_chain::frozen_chain_snapshot(
        &project_root,
        &prd_path,
        &task_root,
    )?;
    // A task root that already records its repository must name this one; a
    // moved base commit is reported below, once the run exists to log it.
    let existing_record = super::workflow_decompose_repository::verify_existing_record(
        &task_root,
        &repository,
    )?;
    let arguments = fixed_script_arguments(
        &project_root,
        &prd_path,
        &prd_digest,
        acceptance_criteria,
        config,
        &task_root,
        &repository.root,
        frozen_chain.to_argument(),
    );
    let log_path = task_root.join(".decompose.log");
    let identity = FixedRunIdentityV1 {
        template_version: FIXED_DECOMPOSITION_TEMPLATE_VERSION.to_string(),
        starting_binary_revision: starting_binary_revision.clone(),
        script_digest,
        catalog_digest: catalog.digest.clone(),
        project_root_identity: path_text(&project_root),
        prd_identity: path_text(&prd_path),
        task_root_identity: path_text(&task_root),
    };
    let state = FixedDecompositionStateV1 {
        schema_version: FIXED_DECOMPOSITION_STATE_SCHEMA_VERSION,
        run_kind: WorkflowRunKind::FixedDecompositionV1,
        identity,
        phase: DecompositionPhase::Identity,
        attempts: Default::default(),
        dispositions: Default::default(),
        log_path: path_text(&log_path),
    };
    let route = super::workflow_provider_route::resolve_anthropic_route(
        config.api.base_url.as_deref(),
        super::workflow_provider_route::ProviderEndpointPolicy::ConfiguredOnly,
    );
    let launch_digest = fixed_launch_digest(&state.identity, &arguments, &catalog, &route)?;
    let calls =
        archon_workflow::v2::script::dry_run_workflow_plan(FIXED_SCRIPT_SOURCE, Some(&arguments))
            .await?;
    let target_repository_root = path_text(&repository.root);
    // What the spec's repository root adds to every author's readable
    // directories (Issue-56); the authors keep the project root as their
    // working directory.
    let read_roots =
        super::workflow_read_scope::read_roots(&project_root, Some(&target_repository_root));
    let spec = WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: FIXED_DECOMPOSITION_TEMPLATE_VERSION.to_string(),
        task: format!(
            "Decompose PRD {} into {}",
            prd_path.display(),
            task_root.display()
        ),
        target_repository_root: Some(target_repository_root.clone()),
        max_parallelism: u32::try_from(config.subagent.max_concurrent.max(1))
            .context("subagent concurrency exceeds workflow limit")?,
        max_agents: 64,
        stages: Vec::new(),
        permissions: Default::default(),
        learning_hooks: Vec::new(),
    };
    let plan = super::workflow_live::workflow_live_planner::WorkflowScriptPlan::fixed(
        spec,
        FIXED_SCRIPT_SOURCE,
        calls,
        arguments.clone(),
    );

    let store = WorkflowStore::project(&project_root);
    let mut approval_spec = plan.approval_metadata_spec();
    approval_spec.permissions.insert(
        FIXED_LAUNCH_DIGEST_PERMISSION.to_string(),
        serde_json::Value::String(launch_digest),
    );
    let run = create_claimed_run(&store, &task_root, approval_spec, &state)?;
    let run_id = run.id.clone();
    let _execution_lease = crate::command::workflow_task_root_reclaim::begin_execution(&store, &run_id)?;
    let launch_generation = run.generation;
    let launch = async {
        if let Some(slot) = persisted_run_id {
            *slot
                .lock()
                .map_err(|_| anyhow!("fixed decomposition run-id owner lock is poisoned"))? =
                Some(run_id.clone());
        }
        WorkflowBundle::create_for_run(
            &store,
            &run,
            FIXED_SCRIPT_SOURCE,
            WorkflowBundleOrigin::GeneratedHarness,
        )?;
        super::workflow_live::save_fixed_decomposition_metadata(
            &store,
            &run_id,
            &plan,
            &state.identity,
        )?;
        if let Some(owner_identity) = interactive_owner {
            crate::command::workflow_decompose_owner::initialize(&store, &run_id, owner_identity)?;
        }
        store.write_run_json(&run_id, FIXED_CATALOG_PATH, &catalog)?;
        store.write_run_json(&run_id, FIXED_ARGUMENTS_PATH, &arguments)?;
        store.write_run_json(&run_id, FIXED_PROVIDER_ROUTE_PATH, &route)?;
        crate::command::workflow_decompose_log::append_fixed_log_marker(
            &log_path,
            "run_started",
            &run_id,
            &state.identity,
        )?;
        let record = match existing_record {
            Some(record) => record,
            None => super::workflow_decompose_repository::record_launch(
                &task_root,
                &repository,
                &run_id,
            )?,
        };
        crate::command::workflow_decompose_log::append_nofollow_line(
            &log_path,
            &super::workflow_decompose_repository::log_line(&run_id, &repository, &record),
        )?;
        if let Some(drift) = super::workflow_decompose_repository::drift_text(&record, &repository) {
            ui_sink
                .emit(WorkflowUiEvent::Text(format!("Repository drift: {drift}\n")))
                .await
                .map_err(|error| anyhow!("reporting repository drift: {error}"))?;
        }
        if cancellation_requested
            .is_some_and(|requested| requested.load(std::sync::atomic::Ordering::SeqCst))
        {
            archon_workflow::LifecycleController::new(store.clone())
                .apply(&run_id, archon_workflow::LifecycleAction::Cancel)?;
            return Err(anyhow!(
                "fixed decomposition cancelled after persistence and before provider construction"
            ));
        }
        ui_sink
            .emit(WorkflowUiEvent::Text(format!(
                "Fixed decomposition started: {run_id}\n"
            )))
            .await
            .map_err(|error| anyhow!("reporting persisted fixed decomposition run id: {error}"))?;

        let client = factory
            .build_client(WorkflowLlmClientRequest {
                cwd: project_root.clone(),
                origin: "workflow_decompose_v1".to_string(),
                session_id: run_id.clone(),
                // The authors work in the project directory and read the
                // repository (Issue-56): without this every Read of it was
                // refused and the bodies were written around the refusal.
                read_roots: read_roots.clone(),
            })
            .await
            .context("building the fixed decomposition provider client")?;
        // Proof, not trust: the same guard the authors' tools consult, asked
        // now for the repository root. A refusal ends the launch here with
        // the guard's text, before the first author spends an hour on it.
        super::workflow_read_scope::require_agent_read(
            client.as_ref(),
            &repository.root,
            "the decomposition authors",
        )?;
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
                    run_staging_root: store.run_dir(&run_id).join("host-command-staging"),
                    frozen_task_id: None,
                    frozen_task_file: None,
                    freeze_provider_environment: freeze_provider_environment(env_vars),
                    gate_mode: config.workflow.gate_mode,
                },
                store.run_dir(&run_id),
            ),
        );
        let agent_names = AgentRegistry::load(&project_root)
            .available_agent_names()
            .into_iter()
            .map(str::to_string)
            .collect();
        super::workflow_live::execute_fixed_decomposition_v2_run(
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
    .await;
    match launch {
        Ok(output) => Ok(output),
        Err(error) => {
            let cleanup = cancel_active_launch_failure(&store, &run_id, launch_generation);
            match cleanup {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(anyhow!(
                    "fixed decomposition launch failed ({error:#}); terminal cleanup also failed ({cleanup_error})"
                )),
            }
        }
    }
}

pub(crate) fn cancel_active_launch_failure(
    store: &WorkflowStore,
    run_id: &str,
    expected_generation: u64,
) -> Result<()> {
    let run = store.load_state(run_id)?;
    if run.generation != expected_generation {
        return Ok(());
    }
    if matches!(
        run.status,
        archon_workflow::RunStatus::Completed
            | archon_workflow::RunStatus::Paused
            | archon_workflow::RunStatus::Cancelled
            | archon_workflow::RunStatus::Failed
            | archon_workflow::RunStatus::Blocked
    ) {
        return Ok(());
    }
    archon_workflow::LifecycleController::new(store.clone())
        .apply(run_id, archon_workflow::LifecycleAction::Cancel)?;
    Ok(())
}

/// The launch-bound script arguments. Resume rebuilds them from the same
/// inputs and compares them to the persisted copy, so every key here is part
/// of the run's identity; `frozen_chain` is the launch-time reading of the
/// task root and is carried forward verbatim on resume rather than re-read,
/// because the script's call sequence depends on it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fixed_script_arguments(
    project_root: &Path,
    prd_path: &Path,
    prd_digest: &str,
    acceptance_criteria: BTreeMap<String, String>,
    config: &ArchonConfig,
    task_root: &Path,
    repository_root: &Path,
    frozen_chain: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "projectRoot": path_text(project_root),
        // The code repository (Issue-55): the only place the script's authors
        // and critics verify source paths, test names, module layout and
        // "exists / does not exist" claims. projectRoot keeps the PRD, the
        // task root and .mcp.json and nothing else.
        "repositoryRoot": path_text(repository_root),
        "prdPath": path_text(prd_path),
        "prdDigest": prd_digest,
        "acceptanceCriteria": acceptance_criteria,
        "authorMaxParallelism": config.subagent.max_concurrent.max(1),
        "taskRoot": path_text(task_root),
        "gateMode": gate_mode_text(config.workflow.gate_mode),
        // Directory NAMES the authors must not descend into, from the engine's own
        // canonical list rather than a literal in a prompt string. project-1 holds
        // 249,451 files, 230,606 of them under .archon; an author told to read
        // "relevant repository files" walks all of it (run wf-4815f89a,
        // acceptance-author-3, 69 tool calls and no artifact in 7200s).
        "excludedDirs": archon_leann::language::default_exclude_patterns(),
        "frozenChain": frozen_chain,
    })
}

pub(crate) fn fixed_launch_digest(
    identity: &FixedRunIdentityV1,
    arguments: &serde_json::Value,
    catalog: &archon_workflow::CommandCapabilityCatalog,
    route: &super::workflow_provider_route::TrustedProviderRouteSnapshot,
) -> Result<String> {
    let bytes = serde_json::to_vec(&(identity, arguments, catalog, route))?;
    Ok(archon_workflow::task_set_contract::content_digest(&bytes))
}

fn freeze_provider_environment(env_vars: &ArchonEnvVars) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    for (name, value) in [
        ("ANTHROPIC_API_KEY", env_vars.anthropic_api_key.as_ref()),
        ("ARCHON_API_KEY", env_vars.archon_api_key.as_ref()),
        ("ARCHON_OAUTH_TOKEN", env_vars.archon_oauth_token.as_ref()),
        ("ARCHON_MODEL", env_vars.model.as_ref()),
        ("ARCHON_EFFORT", env_vars.effort.as_ref()),
    ] {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            environment.insert(name.to_string(), value.clone());
        }
    }
    if let Some(path) = &env_vars.config_dir {
        environment.insert(
            "ARCHON_CONFIG_DIR".to_string(),
            path.to_string_lossy().into_owned(),
        );
    }
    environment
}

fn canonical_project_path(project_root: &Path, path: &Path, label: &str) -> Result<PathBuf> {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    };
    let canonical = canonical_existing(&candidate, label)?;
    canonical.strip_prefix(project_root).map_err(|_| {
        anyhow!(
            "{label} {} escapes project root {}",
            canonical.display(),
            project_root.display()
        )
    })?;
    Ok(canonical)
}

fn canonical_existing(path: &Path, label: &str) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("canonicalizing {label} {}", path.display()))
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn gate_mode_text(mode: GateMode) -> &'static str {
    match mode {
        GateMode::Off => "off",
        GateMode::Observe => "observe",
        GateMode::Enforce => "enforce",
    }
}

pub(crate) fn is_fixed_decomposition_run(cwd: &Path, run_id: &str) -> Result<bool> {
    let project_root = canonical_existing(cwd, "project root")?;
    let store = WorkflowStore::project(project_root);
    let path = store.run_dir(run_id).join(FIXED_DECOMPOSITION_STATE_PATH);
    if !path.exists() {
        return Ok(false);
    }
    Ok(read_fixed_state(&store, run_id)?.run_kind == WorkflowRunKind::FixedDecompositionV1)
}

#[path = "workflow_decompose_resume.rs"]
mod resume;
pub(crate) use resume::{
    resume_fixed_decomposition_with_factory, resume_fixed_decomposition_with_factory_and_sink,
};

#[path = "workflow_decompose_claim.rs"]
mod claim;
pub(crate) use claim::create_claimed_run;
use claim::{read_fixed_state, read_run_json};

#[cfg(test)]
#[path = "workflow_decompose_repair_tests.rs"]
mod repair_tests;
