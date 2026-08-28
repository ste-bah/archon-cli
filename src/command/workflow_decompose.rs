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

pub(crate) const FIXED_SCRIPT_SOURCE: &str = include_str!("workflow_decompose_v1.js");
pub(crate) const FIXED_DECOMPOSITION_STATE_PATH: &str = "decomposition/state.json";
pub(crate) const FIXED_CATALOG_PATH: &str = "decomposition/command-catalog.json";
pub(crate) const FIXED_ARGUMENTS_PATH: &str = "decomposition/arguments.json";
pub(crate) const FIXED_PROVIDER_ROUTE_PATH: &str = "decomposition/provider-route.json";
pub(crate) const FIXED_GENERATED_METADATA_PATH: &str = "v2/generated-metadata.json";
pub(crate) const FIXED_LAUNCH_DIGEST_PERMISSION: &str = "archon.fixed_decomposition_launch_digest";
pub(crate) const DECOMPOSE_GATE_OFF_REMEDY: &str = "workflow decompose requires workflow.gate_mode=observe or enforce; set [workflow] gate_mode = \"observe\" and retry";

pub(crate) async fn run_fixed_decomposition_with_factory(
    cwd: &Path,
    prd: &Path,
    tasks: &Path,
    yes: bool,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    factory: &dyn WorkflowLlmClientFactory,
) -> Result<String> {
    run_fixed_decomposition_with_factory_and_sink(
        cwd,
        prd,
        tasks,
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

pub(crate) async fn run_fixed_decomposition_with_factory_and_sink(
    cwd: &Path,
    prd: &Path,
    tasks: &Path,
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
    let (_, prd_digest, _) = super::workflow_task_set::validate_prd_input(&prd_path)?;
    let starting_binary_revision = env!("ARCHON_GIT_HASH").to_string();
    let catalog = fixed_decomposition_catalog(&starting_binary_revision)?;
    let script_digest = workflow_scaffold_hash(FIXED_SCRIPT_SOURCE);
    let arguments = serde_json::json!({
        "projectRoot": path_text(&project_root),
        "prdPath": path_text(&prd_path),
        "prdDigest": prd_digest.clone(),
        "taskRoot": path_text(&task_root),
        "gateMode": gate_mode_text(config.workflow.gate_mode),
    });
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
    let spec = WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: FIXED_DECOMPOSITION_TEMPLATE_VERSION.to_string(),
        task: format!(
            "Decompose PRD {} into {}",
            prd_path.display(),
            task_root.display()
        ),
        target_repository_root: None,
        max_parallelism: 1,
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
            })
            .await
            .context("building the fixed decomposition provider client")?;
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

pub(crate) fn create_claimed_run(
    store: &WorkflowStore,
    task_root: &Path,
    spec: WorkflowSpec,
    state: &FixedDecompositionStateV1,
) -> Result<archon_workflow::WorkflowRun> {
    store.with_store_lock(|locked| {
        refuse_active_task_root(locked, task_root)
            .map_err(|error| archon_workflow::WorkflowError::PolicyDenied(error.to_string()))?;
        let run = locked.create_run(spec)?;
        if let Err(error) = locked.write_run_json(&run.id, FIXED_DECOMPOSITION_STATE_PATH, state) {
            let run_dir = locked.run_dir(&run.id);
            if let Err(cleanup) = std::fs::remove_dir_all(&run_dir) {
                return Err(archon_workflow::WorkflowError::StateCorrupt(format!(
                    "fixed task-root claim failed ({error}); incomplete run {} could not be removed ({cleanup})",
                    run.id
                )));
            }
            return Err(error);
        }
        Ok(run)
    })
    .map_err(Into::into)
}

fn refuse_active_task_root(store: &WorkflowStore, task_root: &Path) -> Result<()> {
    let identity = path_text(task_root);
    for run in store.list_runs()? {
        if matches!(
            run.status,
            archon_workflow::RunStatus::Completed | archon_workflow::RunStatus::Failed
        ) {
            continue;
        }
        let Ok(state) = read_fixed_state(store, &run.id) else {
            continue;
        };
        if state.run_kind == WorkflowRunKind::FixedDecompositionV1
            && state.identity.task_root_identity == identity
        {
            return Err(anyhow!(
                "active fixed decomposition {} already owns task root {}; resume or complete that run before launching another; cancelled fixed runs remain resumable and retain ownership",
                run.id,
                task_root.display()
            ));
        }
    }
    Ok(())
}

fn read_fixed_state(store: &WorkflowStore, run_id: &str) -> Result<FixedDecompositionStateV1> {
    read_run_json(store, run_id, FIXED_DECOMPOSITION_STATE_PATH)
}

fn read_run_json<T: serde::de::DeserializeOwned>(
    store: &WorkflowStore,
    run_id: &str,
    relative: &str,
) -> Result<T> {
    let path = store.run_dir(run_id).join(relative);
    serde_json::from_slice(
        &std::fs::read(&path)
            .with_context(|| format!("reading fixed decomposition record {}", path.display()))?,
    )
    .with_context(|| format!("parsing fixed decomposition record {}", path.display()))
}
