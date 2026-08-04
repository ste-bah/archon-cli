//! Write-capable fan-out: the layer that turns one host call into isolated,
//! ownership-bounded branches that may change the repository.
//!
//! It plans which branches may touch which files, runs them in waves under one
//! of three isolation modes (serial, coordinated, worktree), validates what
//! each branch actually changed against what it was allowed to change, and
//! merges the surviving work back. Everything it decides is expressed in types
//! this crate already owns.
//!
//! The one thing it does not own is the moment a branch hands its execution to
//! an agent; that goes through [`crate::agent_dispatch_port`], which the
//! binary supplies. The fan-out ITEMS are likewise built by the caller and
//! passed in — the builder is shared with read-only fan-out and resolves stored
//! source, which is host territory, not write-layer territory.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::Semaphore;

use crate::agent_dispatch_port::WorkflowAgentDispatch;
use crate::control::poll_v2_run_control;
use crate::error::{WorkflowError, WorkflowResult};
use crate::generated_contract::{
    canonical_task_ids_from_generated_value, evidence_refs_from_generated_value,
};
use crate::store::WorkflowStore;
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::branch_cache::split_reusable_branch_outcomes;
use crate::v2::branch_evidence::attach_branch_evidence;
use crate::v2::completion_evidence::attach_completion_evidence_for_call;
use crate::v2::target_expansion::{ExpandedTargetFiles, expand_declared_rust_module_targets};
use crate::v2::{
    BranchFailureKind, WorkflowV2AgentAdapter, WorkflowV2BranchOutcome, WorkflowV2CallExecution,
    WorkflowV2CommandStatus, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FanoutItem,
    WorkflowV2HostCall, WorkflowV2RejectedOutput, WorkflowV2ResidualGap, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2SourceTaskGraph, WorkflowV2Status, WorkflowV2TaskCoverage,
    WorkflowV2TaskCoverageStatus, WorkflowV2WriteAssignment, WorkflowV2WriteItem,
    WorkflowV2WriteMode, WorkflowV2WritePlan, WorkflowV2WritePlanner, WorkflowV2WriteWave,
    validate_changed_files_for_repository,
};
use crate::write_coordinator::patch_apply::apply_wave;
use crate::write_coordinator::patch_manifest::{capture_patch, persist_manifest};
use crate::write_coordinator::worktree_isolation::{
    capture_canonical_baseline, cleanup_workspace, create_item_workspace,
};
use crate::write_coordinator::write_plan::{
    NormalizedPath, TargetFilesSource, WritePlan, normalize_target, resource_keys_for_targets,
};
use crate::write_coordinator::{
    CanonicalBaseline, CapturedPatch, ItemWorkspace, ManifestStatus, PatchManifest,
    WorkspaceStatus, WriteCoordinatorConfig, with_repo_lock,
};

/// Run one write-capable fan-out call to completion.
///
/// `branches` are the fan-out items the caller built. They arrive already
/// derived because the builder is shared with read-only fan-out and resolves
/// stored source expressions against the result store — host policy about
/// where items come from, not a decision this layer makes.
pub async fn run_write_capable_v2_fanout(
    task: &str,
    target_repository_root: Option<&str>,
    execution: WorkflowV2CallExecution,
    adapter: WorkflowV2AgentAdapter,
    dispatch: &dyn WorkflowAgentDispatch,
    v2_store: &WorkflowV2ResultStore,
    store_for_control: &WorkflowStore,
    run_id: &str,
    workspace_boundary_supported: bool,
    branches: Vec<WorkflowV2FanoutItem>,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    source_task_graph: Option<&WorkflowV2SourceTaskGraph>,
) -> WorkflowResult<WorkflowV2Result> {
    let mut branches = stamp_project_artifact_policy(branches, v2_store);
    apply_source_graph_targets_to_branches(&mut branches, source_task_graph);
    // Authoritative tool binding does NOT depend on the source graph: v3
    // authored write call ids (`implement-task-...`) are not recognized by
    // dynamic_source_kind, so no graph exists for them and the graph-based
    // stamp never runs. Stamp straight from the task universe instead.
    stamp_required_tools_from_universe(&mut branches, task_universe);
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
    let all_branches = branches.clone();
    let (reused_outcomes, branches) =
        split_reusable_branch_outcomes(v2_store, &execution.call.id, branches)?;
    let mut reused_results = branch_results_from_outcomes(&reused_outcomes);
    revalidate_reused_artifact_results(&all_branches, &mut reused_results, target_repository_root);
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
                dispatch,
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
                dispatch,
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
                dispatch,
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

fn revalidate_reused_artifact_results(
    branches: &[crate::WorkflowV2FanoutItem],
    results: &mut [WorkflowV2Result],
    target_repository_root: Option<&str>,
) {
    let Some(root) = target_repository_root else {
        return;
    };
    for result in results {
        let Some(item_id) = result
            .data
            .get("branch_id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        let Some(branch) = branches.iter().find(|branch| branch.id == item_id) else {
            continue;
        };
        if let Err(error) =
            verify_declared_artifacts_for_result(&branch.input, result, Path::new(root))
        {
            *result = write_branch_validation_error_result(&item_id, Some(&branch.input), &error);
        }
    }
}

/// Stamp the project's artifact-root policy onto every branch item.
///
/// Read-only verification branches need this as much as write branches: without
/// the project root a verifier falls back to repo-relative paths and cannot
/// resolve a declared artifact the reference told it to check absolutely.
pub fn stamp_project_artifact_policy(
    mut branches: Vec<crate::WorkflowV2FanoutItem>,
    v2_store: &WorkflowV2ResultStore,
) -> Vec<crate::WorkflowV2FanoutItem> {
    let context = crate::project_artifact_context_from_v2_root(v2_store.root());
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

/// Stamp each branch item's `required_tools` from the AUTHORITATIVE task
/// universe, matched by the item's canonical task ids. Runs for every write
/// branch regardless of whether a source graph was built, so tool binding
/// works for authored (v3) and generated (v2) call ids alike. Agent-authored
/// tool declarations were already stripped at the shared builder, so this is
/// the only writer of the field.
fn stamp_required_tools_from_universe(
    branches: &mut [crate::WorkflowV2FanoutItem],
    task_universe: Option<&crate::task_universe::WorkflowV2TaskUniverse>,
) {
    let Some(universe) = task_universe else {
        return;
    };
    for branch in branches {
        let Some(item) = branch
            .input
            .get_mut("item")
            .and_then(serde_json::Value::as_object_mut)
        else {
            continue;
        };
        let claimed: Vec<String> = item
            .get("canonical_task_ids")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect();
        if claimed.is_empty() {
            continue;
        }
        let mut tools: std::collections::BTreeSet<String> = Default::default();
        for task in &universe.tasks {
            if claimed.iter().any(|id| id == &task.canonical_task_id) {
                tools.extend(task.required_tools.iter().cloned());
            }
        }
        if !tools.is_empty() {
            item.insert(
                "required_tools".to_string(),
                serde_json::json!(tools.into_iter().collect::<Vec<_>>()),
            );
        }
    }
}

fn apply_source_graph_targets_to_branches(
    branches: &mut [crate::WorkflowV2FanoutItem],
    source_task_graph: Option<&crate::WorkflowV2SourceTaskGraph>,
) {
    // Defense in depth: the shared builder (fanout_items_for_call) already
    // recursively strips agent-authored tool declarations from every item;
    // strip again here so this pass is self-contained and the authoritative
    // stamp below is the only tool source, regardless of caller.
    for branch in branches.iter_mut() {
        if let Some(item_value) = branch.input.get_mut("item") {
            crate::tool_declarations::strip_tool_declarations(item_value);
        }
    }
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
        // Stamp ONLY the authoritative required_tools (task-universe derived in
        // source_graph_build) into the fanout item payload, which becomes the
        // write agent's stage input: allowed_mcp_tools binds them and the no-op
        // guard reads them to tell a tool-declaring task from a plain one.
        if !item.required_tools.is_empty()
            && let Some(item_value) = branch
                .input
                .get_mut("item")
                .and_then(serde_json::Value::as_object_mut)
        {
            item_value.insert(
                "required_tools".to_string(),
                serde_json::json!(item.required_tools),
            );
        }
    }
}

fn branch_source_item_id(branch: &crate::WorkflowV2FanoutItem) -> Option<&str> {
    branch
        .input
        .get("item")
        .and_then(|item| item.get("item_id").or_else(|| item.get("id")))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

mod contract;
mod coordinated;
mod errors;
mod ownership;
mod preflight;
mod result;
mod serial;
mod worktree;
mod worktree_branch;
mod worktree_wave;

use contract::*;
use coordinated::*;
use errors::*;
use ownership::*;
use preflight::*;
use result::*;
use serial::*;
use worktree::*;
use worktree_branch::*;
use worktree_wave::*;

// `write_tests*`, not `tests*`: the runtime-genericity gate identifies test
// sources by a `_tests` infix and would otherwise scan these as runtime code —
// they carry fixture-domain vocabulary by design.
#[cfg(test)]
#[path = "write_tests.rs"]
mod tests;
