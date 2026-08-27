//! Persisted generated-run metadata and launch-time observer snapshot.

use std::fs;

use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::{
    FixedRunIdentityV1, GeneratedWorkflowKind, GeneratedWorkflowLearningContext,
    RunEndAcceptanceObserverSnapshotV1, WorkflowError, WorkflowGeneratedScaffold, WorkflowRunKind,
    WorkflowStore,
};

use super::WorkflowScriptPlan;

pub(super) const GENERATED_V2_METADATA_PATH: &str = "v2/generated-metadata.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct GeneratedV2Metadata {
    pub(super) schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) run_kind: Option<WorkflowRunKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) fixed_identity: Option<FixedRunIdentityV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) observer_snapshot: Option<RunEndAcceptanceObserverSnapshotV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) generated_kind: Option<GeneratedWorkflowKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) scaffold_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) generated_scaffold: Option<WorkflowGeneratedScaffold>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) task_universe: Option<WorkflowV2TaskUniverse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) script_args: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) governed_learning_context: Vec<GeneratedWorkflowLearningContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) generated_config: Option<archon_core::config::GeneratedWorkflowConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) tuning_decisions: Vec<archon_core::config::GeneratedTuningDecision>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) shape_decisions: Vec<archon_core::config::ShapeDecision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) script_lifecycle: Option<bool>,
}

pub(super) fn save_generated_v2_metadata(
    store: &WorkflowStore,
    run_id: &str,
    plan: &WorkflowScriptPlan,
    script_lifecycle: bool,
) -> archon_workflow::WorkflowResult<()> {
    let generated_scaffold = plan.generated_scaffold();
    let observer_snapshot = (script_lifecycle && plan.task_universe.is_some())
        .then(|| {
            plan.task_universe.as_ref().and_then(|universe| {
                super::workflow_run_end_snapshot::collect_run_end_observer_snapshot(store, universe)
            })
        })
        .flatten();
    let metadata = GeneratedV2Metadata {
        schema_version: "workflow-generated-v2-metadata-v1".to_string(),
        run_kind: Some(if plan.task_universe.is_some() {
            if script_lifecycle {
                WorkflowRunKind::AuthoredTaskWorkflow
            } else {
                WorkflowRunKind::LegacyDecomposed
            }
        } else {
            WorkflowRunKind::FixedOrSavedScript
        }),
        fixed_identity: None,
        observer_snapshot,
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
        tuning_decisions: plan.tuning_decisions.clone(),
        shape_decisions: plan.shape_decisions.clone(),
        // Only task-universe runs can enter the authored-script lifecycle.
        script_lifecycle: Some(script_lifecycle && plan.task_universe.is_some()),
    };
    store.write_run_json(run_id, GENERATED_V2_METADATA_PATH, &metadata)
}

pub(crate) fn save_fixed_decomposition_metadata(
    store: &WorkflowStore,
    run_id: &str,
    plan: &WorkflowScriptPlan,
    identity: &FixedRunIdentityV1,
) -> archon_workflow::WorkflowResult<()> {
    let metadata = GeneratedV2Metadata {
        schema_version: "workflow-generated-v2-metadata-v1".to_string(),
        run_kind: Some(WorkflowRunKind::FixedDecompositionV1),
        fixed_identity: Some(identity.clone()),
        observer_snapshot: None,
        generated_kind: None,
        scaffold_hash: Some(plan.scaffold_hash()),
        generated_scaffold: None,
        task_universe: None,
        script_args: plan.script_args.clone(),
        governed_learning_context: Vec::new(),
        generated_config: None,
        tuning_decisions: Vec::new(),
        shape_decisions: Vec::new(),
        script_lifecycle: Some(true),
    };
    store.write_run_json(run_id, GENERATED_V2_METADATA_PATH, &metadata)
}

pub(super) fn load_generated_v2_metadata(
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
