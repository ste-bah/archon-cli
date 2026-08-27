//! First-class launcher for the immutable engine-native decomposition run.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_core::config::{ArchonConfig, GateMode};
use archon_workflow::{
    DecompositionPhase, FIXED_DECOMPOSITION_STATE_SCHEMA_VERSION,
    FIXED_DECOMPOSITION_TEMPLATE_VERSION, FixedDecompositionStateV1, FixedRunIdentityV1,
    WorkflowBundle, WorkflowBundleOrigin, WorkflowLlmClientFactory, WorkflowLlmClientRequest,
    WorkflowRunKind, WorkflowSpec, WorkflowStore, workflow_scaffold_hash,
};

use super::workflow_host_command_catalog::fixed_decomposition_catalog;

pub(crate) const FIXED_SCRIPT_SOURCE: &str = include_str!("workflow_decompose_v1.js");
pub(crate) const FIXED_DECOMPOSITION_STATE_PATH: &str = "decomposition/state.json";
pub(crate) const FIXED_CATALOG_PATH: &str = "decomposition/command-catalog.json";
pub(crate) const FIXED_ARGUMENTS_PATH: &str = "decomposition/arguments.json";
pub(crate) const FIXED_PROVIDER_ROUTE_PATH: &str = "decomposition/provider-route.json";
pub(crate) const DECOMPOSE_GATE_OFF_REMEDY: &str = "workflow decompose requires workflow.gate_mode=observe or enforce; set [workflow] gate_mode = \"observe\" and retry";

pub(crate) async fn run_fixed_decomposition_with_factory(
    cwd: &Path,
    prd: &Path,
    tasks: &Path,
    yes: bool,
    config: &ArchonConfig,
    factory: &dyn WorkflowLlmClientFactory,
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
    let starting_binary_revision = env!("ARCHON_GIT_HASH").to_string();
    let catalog = fixed_decomposition_catalog(&starting_binary_revision)?;
    let script_digest = workflow_scaffold_hash(FIXED_SCRIPT_SOURCE);
    let arguments = serde_json::json!({
        "projectRoot": path_text(&project_root),
        "prdPath": path_text(&prd_path),
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
    let run = store.create_run(plan.approval_metadata_spec())?;
    WorkflowBundle::create_for_run(
        &store,
        &run,
        FIXED_SCRIPT_SOURCE,
        WorkflowBundleOrigin::GeneratedHarness,
    )?;
    super::workflow_live::save_fixed_decomposition_metadata(
        &store,
        &run.id,
        &plan,
        &state.identity,
    )?;
    store.write_run_json(&run.id, FIXED_DECOMPOSITION_STATE_PATH, &state)?;
    store.write_run_json(&run.id, FIXED_CATALOG_PATH, &catalog)?;
    store.write_run_json(&run.id, FIXED_ARGUMENTS_PATH, &arguments)?;
    store.write_run_json(&run.id, FIXED_PROVIDER_ROUTE_PATH, &route)?;

    let _client = factory
        .build_client(WorkflowLlmClientRequest {
            cwd: project_root,
            origin: "workflow_decompose_v1".to_string(),
            session_id: run.id.clone(),
        })
        .await
        .context("building the fixed decomposition provider client")?;

    Err(anyhow!(
        "fixed decomposition run {} was persisted but execution is not connected",
        run.id
    ))
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
