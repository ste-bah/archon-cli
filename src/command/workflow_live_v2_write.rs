use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use archon_workflow::write_coordinator::patch_apply::apply_wave;
use archon_workflow::write_coordinator::patch_manifest::{capture_patch, persist_manifest};
use archon_workflow::write_coordinator::worktree_isolation::{
    capture_canonical_baseline, cleanup_workspace, create_item_workspace,
};
use archon_workflow::write_coordinator::write_plan::{
    NormalizedPath, normalize_target, resource_keys_for_targets,
};
use archon_workflow::write_coordinator::{
    CanonicalBaseline, CapturedPatch, ItemWorkspace, ManifestStatus, PatchManifest,
    WorkspaceStatus, with_repo_lock,
};
use archon_workflow::{
    BranchFailureKind, TargetFilesSource, WorkflowError, WorkflowV2AgentAdapter,
    WorkflowV2BranchOutcome, WorkflowV2CallExecution, WorkflowV2CommandStatus, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2HostCall, WorkflowV2ResidualGap, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2Status, WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus,
    WorkflowV2WriteAssignment, WorkflowV2WriteItem, WorkflowV2WriteMode, WorkflowV2WritePlan,
    WorkflowV2WritePlanner, WorkflowV2WriteWave, WriteCoordinatorConfig, WritePlan,
    validate_changed_files_for_repository,
};
use tokio::sync::Semaphore;

use super::super::workflow_live_generated_contract::{
    canonical_task_ids_from_generated_value, evidence_refs_from_generated_value,
};
use super::workflow_live_v2_aggregate::attach_branch_evidence;
use super::workflow_live_v2_data::{attach_completion_evidence_for_call, fanout_items_for_call};
use super::workflow_live_v2_state::poll_v2_run_control;
use super::workflow_live_v2_target_expansion::expand_declared_rust_module_targets;
use super::{
    LiveV2AgentClient, run_single_v2_agent_call, run_single_v2_agent_call_in_repository,
    split_reusable_branch_outcomes,
};

pub(super) async fn run_write_capable_v2_fanout(
    task: &str,
    target_repository_root: Option<&str>,
    execution: WorkflowV2CallExecution,
    adapter: WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: &WorkflowV2ResultStore,
    store_for_control: &archon_workflow::WorkflowStore,
    run_id: &str,
    workspace_boundary_supported: bool,
    source_task_graph: Option<&archon_workflow::WorkflowV2SourceTaskGraph>,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let branches = fanout_items_for_call(&execution, v2_store)?;
    let mut branches = stamp_project_artifact_policy(branches, v2_store);
    apply_source_graph_targets_to_branches(&mut branches, source_task_graph);
    let all_write_items =
        write_items_for_branches(target_repository_root, &execution.call, &branches)?;
    let planner = WorkflowV2WritePlanner::new(
        v2_store
            .root()
            .join("worktrees")
            .join(sanitize_v2_path_segment(&execution.call.id)),
    );
    let all_plan = planner
        .plan(&all_write_items)
        .map_err(|err| WorkflowError::SpecInvalid(err.to_string()))?;
    let (reused_outcomes, branches) =
        split_reusable_branch_outcomes(v2_store, &execution.call.id, branches)?;
    let reused_results = branch_results_from_outcomes(&reused_outcomes);
    if branches.is_empty() {
        return Ok(result_from_write_fanout(
            &execution.call,
            reused_results,
            &all_plan,
            0,
            None,
        ));
    }
    let write_items = write_items_for_branches(target_repository_root, &execution.call, &branches)?;
    let plan = planner
        .plan(&write_items)
        .map_err(|err| WorkflowError::SpecInvalid(err.to_string()))?;
    if let Some(result) = preflight_write_fanout_source_contract(
        &execution.call,
        &branches,
        &write_items,
        &plan,
        target_repository_root,
    ) {
        return Ok(result);
    }
    match (execution.call.write_mode, workspace_boundary_supported) {
        (Some(WorkflowV2WriteMode::Coordinated), true) => {
            run_coordinated_v2_write_fanout(
                task,
                target_repository_root,
                &execution,
                adapter,
                client,
                v2_store,
                store_for_control,
                run_id,
                branches,
                plan,
                reused_results,
            )
            .await
        }
        (Some(WorkflowV2WriteMode::Worktree), true) => {
            run_worktree_v2_write_fanout(
                task,
                target_repository_root,
                &execution,
                adapter,
                client,
                v2_store,
                store_for_control,
                run_id,
                branches,
                plan,
                reused_results,
            )
            .await
        }
        (Some(WorkflowV2WriteMode::Serial), _) => {
            run_serial_v2_write_fanout(
                task,
                target_repository_root,
                &execution,
                adapter,
                client,
                v2_store,
                store_for_control,
                run_id,
                branches,
                write_items,
                plan,
                None,
                reused_results,
            )
            .await
        }
        (_, false) => Err(WorkflowError::SpecInvalid(
            "write-capable fanout requested coordinated/worktree isolation, but workspace boundary support is unavailable; workflow.js must choose an explicit safe mode or ask the user"
                .to_string(),
        )),
        _ => Err(WorkflowError::SpecInvalid(format!(
            "write-capable fanout '{}' requires explicit write mode serial, coordinated, or worktree",
            execution.call.id
        ))),
    }
}

fn stamp_project_artifact_policy(
    mut branches: Vec<archon_workflow::WorkflowV2FanoutItem>,
    v2_store: &WorkflowV2ResultStore,
) -> Vec<archon_workflow::WorkflowV2FanoutItem> {
    let context = archon_workflow::project_artifact_context_from_v2_root(v2_store.root());
    if context.is_empty() {
        return branches;
    }
    let stamp = serde_json::json!({
        "version": context.policy_version,
        "project_root": context.project_root,
        "artifact_roots": context.artifact_roots,
    });
    for branch in &mut branches {
        if let Some(object) = branch.input.as_object_mut() {
            object.insert(
                "_workflow_project_artifact_policy".to_string(),
                stamp.clone(),
            );
        }
    }
    branches
}

fn apply_source_graph_targets_to_branches(
    branches: &mut [archon_workflow::WorkflowV2FanoutItem],
    source_task_graph: Option<&archon_workflow::WorkflowV2SourceTaskGraph>,
) {
    let Some(graph) = source_task_graph else {
        return;
    };
    for branch in branches {
        let Some(item_id) = branch_source_item_id(branch) else {
            continue;
        };
        let Some(item) = graph.items.iter().find(|item| item.item_id == item_id) else {
            continue;
        };
        if !item.target_files.is_empty() {
            branch.call.options.target_files = item.target_files.clone();
        }
    }
}

fn branch_source_item_id(branch: &archon_workflow::WorkflowV2FanoutItem) -> Option<&str> {
    branch
        .input
        .get("item")
        .and_then(|item| item.get("item_id").or_else(|| item.get("id")))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

include!("workflow_live_v2_write_coordinated.rs");

include!("workflow_live_v2_write_worktree.rs");

include!("workflow_live_v2_write_worktree_wave.rs");

include!("workflow_live_v2_write_worktree_branch.rs");

include!("workflow_live_v2_write_serial.rs");

include!("workflow_live_v2_write_contract.rs");

include!("workflow_live_v2_write_preflight.rs");

include!("workflow_live_v2_write_ownership.rs");

include!("workflow_live_v2_write_result.rs");

include!("workflow_live_v2_write_errors.rs");

#[cfg(test)]
#[path = "workflow_live_v2_write_tests.rs"]
mod tests;
