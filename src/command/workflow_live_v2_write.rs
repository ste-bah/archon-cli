use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use archon_workflow::write_coordinator::patch_apply::apply_wave;
use archon_workflow::write_coordinator::patch_manifest::{
    capture_patch, persist_manifest, validate_patch,
};
use archon_workflow::write_coordinator::worktree_isolation::{
    capture_canonical_baseline, cleanup_workspace, create_item_workspace,
};
use archon_workflow::write_coordinator::write_plan::{normalize_target, resource_keys_for_targets};
use archon_workflow::write_coordinator::{
    CanonicalBaseline, CapturedPatch, ItemWorkspace, ManifestStatus, PatchManifest,
    WorkspaceStatus, with_repo_lock,
};
use archon_workflow::{
    TargetFilesSource, WorkflowError, WorkflowSpec, WorkflowV2AgentAdapter,
    WorkflowV2BranchOutcome, WorkflowV2CallExecution, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2HostCall, WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2ResultStore,
    WorkflowV2Status, WorkflowV2WriteAssignment, WorkflowV2WriteItem, WorkflowV2WriteMode,
    WorkflowV2WritePlan, WorkflowV2WritePlanner, WriteCoordinatorConfig, WritePlan,
    validate_changed_files,
};
use tokio::sync::Semaphore;

use super::workflow_live_v2_aggregate::attach_branch_evidence;
use super::workflow_live_v2_data::fanout_items_for_call;
use super::workflow_live_v2_state::poll_v2_run_control;
use super::{
    LiveV2AgentClient, run_single_v2_agent_call, run_single_v2_agent_call_in_repository,
    split_reusable_branch_outcomes,
};

pub(super) async fn run_write_capable_v2_fanout(
    task: &str,
    spec: &WorkflowSpec,
    execution: WorkflowV2CallExecution,
    adapter: WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: &WorkflowV2ResultStore,
    store_for_control: &archon_workflow::WorkflowStore,
    run_id: &str,
    workspace_boundary_supported: bool,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let branches = fanout_items_for_call(&execution, v2_store)?;
    let all_write_items = write_items_for_branches(spec, &execution.call, &branches)?;
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
    let write_items = write_items_for_branches(spec, &execution.call, &branches)?;
    let plan = planner
        .plan(&write_items)
        .map_err(|err| WorkflowError::SpecInvalid(err.to_string()))?;
    match (execution.call.write_mode, workspace_boundary_supported) {
        (Some(WorkflowV2WriteMode::Coordinated), true) => {
            run_coordinated_v2_write_fanout(
                task,
                spec,
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
                spec,
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
        (_, false) => {
            run_serial_v2_write_fanout(
                task,
                spec,
                &execution,
                adapter,
                client,
                v2_store,
                store_for_control,
                run_id,
                branches,
                write_items,
                plan,
                Some(
                    "workspace boundary support is unavailable; serialized fallback used"
                        .to_string(),
                ),
                reused_results,
            )
            .await
        }
        _ => {
            run_serial_v2_write_fanout(
                task,
                spec,
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
    }
}

async fn run_coordinated_v2_write_fanout(
    task: &str,
    spec: &WorkflowSpec,
    execution: &WorkflowV2CallExecution,
    adapter: WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: &WorkflowV2ResultStore,
    store_for_control: &archon_workflow::WorkflowStore,
    run_id: &str,
    branches: Vec<archon_workflow::WorkflowV2FanoutItem>,
    plan: WorkflowV2WritePlan,
    reused_results: Vec<WorkflowV2Result>,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let write_items = write_items_for_branches(spec, &execution.call, &branches)?;
    let mut results = Vec::new();
    let mut peak_parallelism = 0usize;
    let max_parallelism = client.fanout_parallelism(execution.call.options.max_parallelism);
    for wave in &plan.waves {
        let semaphore = Arc::new(Semaphore::new(max_parallelism));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let jobs = wave.assignments.iter().map(|assignment| {
            let assignment = assignment.clone();
            let branch = branches
                .iter()
                .find(|branch| branch.id == assignment.item_id)
                .cloned();
            let adapter = adapter.clone();
            let control_store = store_for_control.clone();
            let run_id = run_id.to_string();
            let semaphore = semaphore.clone();
            let active = active.clone();
            let peak = peak.clone();
            async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
                let branch = branch.ok_or_else(|| {
                    WorkflowError::SpecInvalid(format!(
                        "write plan referenced missing fanout item '{}'",
                        assignment.item_id
                    ))
                })?;
                poll_v2_run_control(&control_store, &run_id, &branch.id)?;
                let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
                record_write_peak(&peak, now_active);
                let branch_execution = WorkflowV2CallExecution {
                    call: branch.call,
                    input: branch.input,
                    depends_on: vec![execution.call.id.clone()],
                };
                let result =
                    run_single_v2_agent_call(task, spec, &branch_execution, &adapter, client, None)
                        .await;
                active.fetch_sub(1, Ordering::SeqCst);
                let result = result?;
                poll_v2_run_control(&control_store, &run_id, &assignment.item_id)?;
                Ok::<WorkflowV2Result, WorkflowError>(result)
            }
        });
        let wave_results = futures_util::future::join_all(jobs).await;
        peak_parallelism = peak_parallelism.max(peak.load(Ordering::SeqCst));
        for (assignment, result) in wave.assignments.iter().zip(wave_results) {
            let mut result = match result {
                Ok(result) => result,
                Err(err) if is_recoverable_write_branch_error(&err.to_string()) => {
                    recoverable_write_branch_error_result(&assignment.item_id, &err.to_string())
                }
                Err(err) => return Err(err),
            };
            let write_item = write_items
                .iter()
                .find(|item| item.id == assignment.item_id)
                .ok_or_else(|| {
                    WorkflowError::SpecInvalid(format!(
                        "write item '{}' disappeared during validation",
                        assignment.item_id
                    ))
                })?;
            if let Err(err) = validate_changed_files(write_item, &result) {
                if is_recoverable_write_branch_error(&err.to_string()) {
                    result = recoverable_write_branch_error_result(
                        &assignment.item_id,
                        &err.to_string(),
                    );
                } else {
                    return Err(WorkflowError::SpecInvalid(err.to_string()));
                }
            }
            let role = branches
                .iter()
                .find(|branch| branch.id == assignment.item_id)
                .map(|branch| branch.role.as_str())
                .unwrap_or("coder");
            save_write_branch_outcome(
                v2_store,
                &execution.call.id,
                &assignment.item_id,
                role,
                &result,
            )?;
            results.push(result);
        }
    }
    let mut all_results = reused_results;
    all_results.extend(results);
    Ok(result_from_write_fanout(
        &execution.call,
        all_results,
        &plan,
        peak_parallelism,
        None,
    ))
}

struct PreparedWorktreeBranch {
    branch: archon_workflow::WorkflowV2FanoutItem,
    assignment: WorkflowV2WriteAssignment,
    coordinator_plan: WritePlan,
    baseline: CanonicalBaseline,
    workspace: ItemWorkspace,
}

struct CompletedWorktreeBranch {
    item_id: String,
    role: String,
    result: WorkflowV2Result,
    manifest: Option<PatchManifest>,
    pre_hashes: Option<BTreeMap<String, String>>,
    workspace_root: PathBuf,
}

async fn run_worktree_v2_write_fanout(
    task: &str,
    spec: &WorkflowSpec,
    execution: &WorkflowV2CallExecution,
    adapter: WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: &WorkflowV2ResultStore,
    store_for_control: &archon_workflow::WorkflowStore,
    run_id: &str,
    branches: Vec<archon_workflow::WorkflowV2FanoutItem>,
    plan: WorkflowV2WritePlan,
    reused_results: Vec<WorkflowV2Result>,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let canonical_root = spec
        .target_repository_root
        .as_deref()
        .filter(|root| !root.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            WorkflowError::SpecInvalid(
                "worktree write mode requires target_repository_root".to_string(),
            )
        })?;
    let cfg = WriteCoordinatorConfig::default();
    let run_root = v2_store
        .root()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| v2_store.root().to_path_buf());
    let mut prepared = Vec::new();
    for wave in &plan.waves {
        for assignment in &wave.assignments {
            let branch = branches
                .iter()
                .find(|branch| branch.id == assignment.item_id)
                .cloned()
                .ok_or_else(|| {
                    WorkflowError::SpecInvalid(format!(
                        "write plan referenced missing fanout item '{}'",
                        assignment.item_id
                    ))
                })?;
            poll_v2_run_control(store_for_control, run_id, &branch.id)?;
            let coordinator_plan = coordinator_plan_for_assignment(
                run_id,
                &execution.call.id,
                assignment,
                &canonical_root,
            )?;
            let baseline = capture_canonical_baseline(
                &canonical_root,
                &coordinator_plan,
                &coordinator_plan.verify_inputs,
                &cfg,
            )
            .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
            let workspace = create_item_workspace(&canonical_root, &coordinator_plan, &baseline)
                .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
            prepared.push(PreparedWorktreeBranch {
                branch,
                assignment: assignment.clone(),
                coordinator_plan,
                baseline,
                workspace,
            });
        }
    }

    let max_parallelism = client.fanout_parallelism(execution.call.options.max_parallelism);
    let semaphore = Arc::new(Semaphore::new(max_parallelism));
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let jobs = prepared.into_iter().map(|prepared| {
        let adapter = adapter.clone();
        let control_store = store_for_control.clone();
        let run_id = run_id.to_string();
        let task = task.to_string();
        let spec = spec.clone();
        let run_root = run_root.clone();
        let canonical_root = canonical_root.clone();
        let cfg = cfg.clone();
        let semaphore = semaphore.clone();
        let active = active.clone();
        let peak = peak.clone();
        async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
            poll_v2_run_control(&control_store, &run_id, &prepared.branch.id)?;
            let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
            record_write_peak(&peak, now_active);
            let result = run_one_worktree_branch(
                &task,
                &spec,
                execution,
                adapter,
                client,
                &control_store,
                &run_id,
                &run_root,
                &canonical_root,
                &cfg,
                prepared,
            )
            .await;
            active.fetch_sub(1, Ordering::SeqCst);
            result
        }
    });

    let mut completed = Vec::new();
    for item in futures_util::future::join_all(jobs).await {
        completed.push(item?);
    }
    let peak_parallelism = peak.load(Ordering::SeqCst);

    let mut branch_results = Vec::new();
    let mut manifests = Vec::new();
    let mut pre_hashes_by_item = BTreeMap::new();
    for completed_branch in &completed {
        save_write_branch_outcome(
            v2_store,
            &execution.call.id,
            &completed_branch.item_id,
            &completed_branch.role,
            &completed_branch.result,
        )?;
        branch_results.push(completed_branch.result.clone());
        if let Some(manifest) = &completed_branch.manifest {
            manifests.push(manifest.clone());
        }
        if let Some(pre_hashes) = &completed_branch.pre_hashes {
            pre_hashes_by_item.insert(completed_branch.item_id.clone(), pre_hashes.clone());
        }
    }

    let mut apply_gap = None::<String>;
    if !manifests.is_empty() {
        if !plan.conflicts.is_empty() {
            apply_gap = Some(
                "worktree patches were captured but overlapping targets require operator review before canonical apply"
                    .to_string(),
            );
        } else {
            let apply_result = with_repo_lock(&canonical_root, || {
                apply_wave(
                    &canonical_root,
                    &manifests,
                    &pre_hashes_by_item,
                    0,
                    &run_root,
                    run_id,
                    &execution.call.id,
                )
            });
            match apply_result {
                Ok(record) if !record.items_failed.is_empty() => {
                    apply_gap = Some(format!(
                        "worktree patch apply left {} item(s) unapplied: {}",
                        record.items_failed.len(),
                        record
                            .items_failed
                            .iter()
                            .map(|(item, reason)| format!("{item}: {reason}"))
                            .collect::<Vec<_>>()
                            .join("; ")
                    ));
                }
                Ok(_) => {}
                Err(err) => {
                    apply_gap = Some(format!("worktree patch apply failed: {err}"));
                }
            }
        }
    }

    for completed_branch in &completed {
        let status = if apply_gap.is_none() {
            WorkspaceStatus::Succeeded
        } else {
            WorkspaceStatus::Failed
        };
        let _ = cleanup_workspace(
            &canonical_root,
            &completed_branch.workspace_root,
            status,
            &cfg,
        );
    }

    let mut all_results = reused_results;
    all_results.extend(branch_results);
    let mut result =
        result_from_write_fanout(&execution.call, all_results, &plan, peak_parallelism, None);
    if let Some(reason) = apply_gap {
        result.status = WorkflowV2Status::NeedsReview;
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Review,
            reason.clone(),
        ));
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id: format!(
                "worktree_apply_review_{}",
                sanitize_v2_path_segment(&execution.call.id)
            ),
            description: reason,
            severity: Some("review".to_string()),
        });
    }
    if let Some(object) = result.data.as_object_mut() {
        object.insert(
            "worktree_apply_manifests".to_string(),
            serde_json::Value::Array(
                manifests
                    .iter()
                    .map(|manifest| {
                        serde_json::Value::String(manifest_path_for(
                            &run_root,
                            &execution.call.id,
                            &manifest.item_id,
                        ))
                    })
                    .collect(),
            ),
        );
    }
    Ok(result)
}

async fn run_one_worktree_branch(
    task: &str,
    spec: &WorkflowSpec,
    execution: &WorkflowV2CallExecution,
    adapter: WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    store_for_control: &archon_workflow::WorkflowStore,
    run_id: &str,
    run_root: &Path,
    canonical_root: &Path,
    cfg: &WriteCoordinatorConfig,
    prepared: PreparedWorktreeBranch,
) -> archon_workflow::WorkflowResult<CompletedWorktreeBranch> {
    let branch_id = prepared.branch.id.clone();
    let branch_role = prepared.branch.role.clone();
    let workspace_root = prepared.workspace.plan.isolated_root.clone();
    poll_v2_run_control(store_for_control, run_id, &branch_id)?;
    let branch_execution = WorkflowV2CallExecution {
        call: prepared.branch.call,
        input: prepared.branch.input,
        depends_on: vec![execution.call.id.clone()],
    };
    let mut result = match run_single_v2_agent_call_in_repository(
        task,
        spec,
        &branch_execution,
        &adapter,
        client,
        None,
        Some(workspace_root.display().to_string()),
    )
    .await
    {
        Ok(result) => result,
        Err(err) if is_recoverable_write_branch_error(&err.to_string()) => {
            recoverable_write_branch_error_result(&branch_id, &err.to_string())
        }
        Err(err) => return Err(err),
    };
    poll_v2_run_control(store_for_control, run_id, &branch_id)?;
    let write_item = WorkflowV2WriteItem::new(
        branch_execution.call.id.clone(),
        WorkflowV2WriteMode::Worktree,
        prepared.assignment.owned_targets.clone(),
    );
    if let Err(err) = validate_changed_files(&write_item, &result) {
        if is_recoverable_write_branch_error(&err.to_string()) {
            result = recoverable_write_branch_error_result(&branch_id, &err.to_string());
        } else {
            return Err(WorkflowError::SpecInvalid(err.to_string()));
        }
    }
    let mut manifest = None;
    let mut pre_hashes = None;
    if matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        let captured = match capture_and_validate_worktree_patch(
            &prepared.workspace,
            &prepared.coordinator_plan,
            &prepared.baseline,
            cfg,
            &result,
        ) {
            Ok(captured) => captured,
            Err(err) if is_recoverable_write_branch_error(&err.to_string()) => {
                result = recoverable_write_branch_error_result(&branch_id, &err.to_string());
                return Ok(CompletedWorktreeBranch {
                    item_id: branch_id,
                    role: branch_role,
                    result,
                    manifest: None,
                    pre_hashes: None,
                    workspace_root,
                });
            }
            Err(err) => return Err(err),
        };
        let status = if captured.patch_bytes.is_empty() {
            ManifestStatus::IdempotentNoop
        } else {
            ManifestStatus::PendingApply
        };
        let manifest_path = persist_manifest(
            run_root,
            run_id,
            &execution.call.id,
            &branch_id,
            &captured,
            status,
        )
        .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
        let manifest_body =
            std::fs::read_to_string(&manifest_path).map_err(|err| WorkflowError::Io {
                path: manifest_path.clone(),
                source: err,
            })?;
        let parsed_manifest: PatchManifest = serde_json::from_str(&manifest_body)?;
        result.artifacts.push(archon_workflow::WorkflowV2Artifact {
            id: format!("patch_manifest_{branch_id}"),
            path: manifest_path.display().to_string(),
            description: Some("worktree patch manifest".to_string()),
        });
        pre_hashes = Some(captured.pre_hashes.clone());
        manifest = Some(parsed_manifest);
    }
    let _ = canonical_root;
    Ok(CompletedWorktreeBranch {
        item_id: branch_id,
        role: branch_role,
        result,
        manifest,
        pre_hashes,
        workspace_root,
    })
}

fn capture_and_validate_worktree_patch(
    workspace: &ItemWorkspace,
    coordinator_plan: &WritePlan,
    baseline: &CanonicalBaseline,
    cfg: &WriteCoordinatorConfig,
    result: &WorkflowV2Result,
) -> archon_workflow::WorkflowResult<CapturedPatch> {
    let captured = capture_patch(workspace, &coordinator_plan.target_files, baseline)
        .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
    let agent_body = serde_json::to_string(result)?;
    validate_patch(&captured, coordinator_plan, cfg, &agent_body)
        .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
    Ok(captured)
}

fn coordinator_plan_for_assignment(
    run_id: &str,
    stage_id: &str,
    assignment: &WorkflowV2WriteAssignment,
    canonical_root: &Path,
) -> archon_workflow::WorkflowResult<WritePlan> {
    let isolated_root = assignment
        .worktree_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| {
            WorkflowError::SpecInvalid(format!(
                "worktree assignment '{}' has no isolated root",
                assignment.item_id
            ))
        })?;
    let mut targets = Vec::new();
    for target in &assignment.owned_targets {
        targets.push(
            normalize_target(target, canonical_root)
                .map_err(|err| WorkflowError::SpecInvalid(err.to_string()))?,
        );
    }
    let resource_keys = resource_keys_for_targets(&targets, canonical_root, &[])
        .map_err(|err| WorkflowError::SpecInvalid(err.to_string()))?;
    Ok(WritePlan {
        run_id: run_id.to_string(),
        stage_id: stage_id.to_string(),
        item_id: assignment.item_id.clone(),
        canonical_root: canonical_root.to_path_buf(),
        isolated_root,
        target_files: targets,
        target_files_source: TargetFilesSource::Item,
        read_context_files: Vec::new(),
        verify_inputs: Vec::new(),
        baseline_id: "git:HEAD".to_string(),
        workspace_boundary_required: true,
        resource_keys,
    })
}

fn manifest_path_for(run_root: &Path, stage_id: &str, item_id: &str) -> String {
    run_root
        .join("write-coordination")
        .join("stages")
        .join(stage_id)
        .join("manifests")
        .join(format!("{item_id}.json"))
        .display()
        .to_string()
}

async fn run_serial_v2_write_fanout(
    task: &str,
    spec: &WorkflowSpec,
    execution: &WorkflowV2CallExecution,
    adapter: WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: &WorkflowV2ResultStore,
    store_for_control: &archon_workflow::WorkflowStore,
    run_id: &str,
    branches: Vec<archon_workflow::WorkflowV2FanoutItem>,
    write_items: Vec<WorkflowV2WriteItem>,
    plan: WorkflowV2WritePlan,
    fallback_reason: Option<String>,
    reused_results: Vec<WorkflowV2Result>,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let mut branch_results = Vec::new();
    for branch in branches {
        let branch_id = branch.id.clone();
        let branch_role = branch.role.clone();
        poll_v2_run_control(store_for_control, run_id, &branch_id)?;
        let branch_execution = WorkflowV2CallExecution {
            call: branch.call,
            input: branch.input,
            depends_on: vec![execution.call.id.clone()],
        };
        let mut result =
            match run_single_v2_agent_call(task, spec, &branch_execution, &adapter, client, None)
                .await
            {
                Ok(result) => result,
                Err(err) if is_recoverable_write_branch_error(&err.to_string()) => {
                    recoverable_write_branch_error_result(&branch_id, &err.to_string())
                }
                Err(err) => return Err(err),
            };
        poll_v2_run_control(store_for_control, run_id, &branch_id)?;
        let write_item = write_items
            .iter()
            .find(|item| item.id == branch_execution.call.id)
            .ok_or_else(|| {
                WorkflowError::SpecInvalid(format!(
                    "write item '{}' disappeared during validation",
                    branch_execution.call.id
                ))
            })?;
        if let Err(err) = validate_changed_files(write_item, &result) {
            if is_recoverable_write_branch_error(&err.to_string()) {
                result = recoverable_write_branch_error_result(&branch_id, &err.to_string());
            } else {
                return Err(WorkflowError::SpecInvalid(err.to_string()));
            }
        }
        save_write_branch_outcome(
            v2_store,
            &execution.call.id,
            &branch_id,
            &branch_role,
            &result,
        )?;
        branch_results.push(result);
    }
    let mut all_results = reused_results;
    all_results.extend(branch_results);
    Ok(result_from_write_fanout(
        &execution.call,
        all_results,
        &plan,
        1,
        fallback_reason,
    ))
}

fn branch_results_from_outcomes(outcomes: &[WorkflowV2BranchOutcome]) -> Vec<WorkflowV2Result> {
    outcomes
        .iter()
        .filter_map(|outcome| outcome.result.clone())
        .collect()
}

fn save_write_branch_outcome(
    v2_store: &WorkflowV2ResultStore,
    call_id: &str,
    item_id: &str,
    role: &str,
    result: &WorkflowV2Result,
) -> archon_workflow::WorkflowResult<()> {
    let outcome = WorkflowV2BranchOutcome {
        item_id: item_id.to_string(),
        role: role.to_string(),
        status: result.status,
        result: Some(result.clone()),
        error: None,
    };
    v2_store.save_branch_outcome(call_id, &outcome)?;
    Ok(())
}

fn write_items_for_branches(
    spec: &WorkflowSpec,
    call: &WorkflowV2HostCall,
    branches: &[archon_workflow::WorkflowV2FanoutItem],
) -> archon_workflow::WorkflowResult<Vec<WorkflowV2WriteItem>> {
    let mode = call.write_mode.unwrap_or(WorkflowV2WriteMode::Serial);
    branches
        .iter()
        .map(|branch| {
            let targets = target_files_for_branch(spec, call, branch)?;
            Ok(WorkflowV2WriteItem::new(branch.id.clone(), mode, targets))
        })
        .collect()
}

fn target_files_for_branch(
    spec: &WorkflowSpec,
    call: &WorkflowV2HostCall,
    branch: &archon_workflow::WorkflowV2FanoutItem,
) -> archon_workflow::WorkflowResult<Vec<String>> {
    if call.options.target_files_from_item {
        let targets = branch
            .input
            .get("item")
            .and_then(|item| {
                item.get("target_files")
                    .or_else(|| item.get("expected_target_files"))
            })
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !targets.is_empty() {
            return normalize_declared_targets(
                &branch.id,
                &targets,
                spec.target_repository_root.as_deref(),
            );
        }
    }
    if !call.options.target_files.is_empty() {
        return normalize_declared_targets(
            &branch.id,
            &call.options.target_files,
            spec.target_repository_root.as_deref(),
        );
    }
    Err(WorkflowError::SpecInvalid(format!(
        "write-capable fanout '{}' item '{}' has no target file ownership",
        call.id, branch.id
    )))
}

fn normalize_declared_targets(
    item_id: &str,
    targets: &[String],
    target_repository_root: Option<&str>,
) -> archon_workflow::WorkflowResult<Vec<String>> {
    targets
        .iter()
        .map(|target| normalize_declared_target(item_id, target, target_repository_root))
        .collect()
}

fn normalize_declared_target(
    item_id: &str,
    target: &str,
    target_repository_root: Option<&str>,
) -> archon_workflow::WorkflowResult<String> {
    let trimmed = target.trim();
    let path = Path::new(trimmed);
    if !path.is_absolute() {
        return Ok(trimmed.to_string());
    }
    let root = target_repository_root
        .map(str::trim)
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            WorkflowError::SpecInvalid(format!(
                "absolute write target '{}' for item '{}' requires target_repository_root",
                target, item_id
            ))
        })?;
    let clean_root = clean_absolute_path(&root)?;
    let clean_target = clean_absolute_path(path)?;
    let relative = clean_target.strip_prefix(&clean_root).map_err(|_| {
        WorkflowError::SpecInvalid(format!(
            "absolute write target '{}' for item '{}' is outside target_repository_root '{}'",
            target,
            item_id,
            clean_root.display()
        ))
    })?;
    if relative.as_os_str().is_empty() {
        return Err(WorkflowError::SpecInvalid(format!(
            "absolute write target '{}' for item '{}' resolves to repository root",
            target, item_id
        )));
    }
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn clean_absolute_path(path: &Path) -> archon_workflow::WorkflowResult<PathBuf> {
    let mut cleaned = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => cleaned.push(prefix.as_os_str()),
            std::path::Component::RootDir => cleaned.push(Path::new("/")),
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => cleaned.push(part),
            std::path::Component::ParentDir => {
                return Err(WorkflowError::SpecInvalid(format!(
                    "write target path contains '..': {}",
                    path.display()
                )));
            }
        }
    }
    Ok(cleaned)
}

fn result_from_write_fanout(
    call: &WorkflowV2HostCall,
    branch_results: Vec<WorkflowV2Result>,
    plan: &WorkflowV2WritePlan,
    peak_parallelism: usize,
    fallback_reason: Option<String>,
) -> WorkflowV2Result {
    let cancelled = count_results_with_status(&branch_results, WorkflowV2Status::Cancelled);
    let blocked = count_results_with_status(&branch_results, WorkflowV2Status::Blocked);
    let failed = count_results_with_status(&branch_results, WorkflowV2Status::Failed);
    let needs_review = count_results_with_status(&branch_results, WorkflowV2Status::NeedsReview);
    let mut result = if cancelled > 0 {
        WorkflowV2Result {
            status: WorkflowV2Status::Cancelled,
            summary: format!(
                "write-capable fanout '{}' cancelled with {} cancelled branch(es)",
                call.id, cancelled
            ),
            ..WorkflowV2Result::default()
        }
    } else if blocked > 0 {
        WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: format!(
                "write-capable fanout '{}' completed with {} blocked branch(es) retained for remediation",
                call.id, blocked
            ),
            residual_gaps: vec![WorkflowV2ResidualGap {
                id: format!(
                    "blocked_write_fanout_{}",
                    sanitize_v2_path_segment(&call.id)
                ),
                description: format!(
                    "write-capable fanout '{}' had {} blocked branch(es)",
                    call.id, blocked
                ),
                severity: Some("remediation".to_string()),
            }],
            ..WorkflowV2Result::default()
        }
    } else if failed > 0 {
        WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: format!(
                "write-capable fanout '{}' completed with {} failed branch(es) needing remediation",
                call.id, failed
            ),
            ..WorkflowV2Result::default()
        }
    } else if needs_review > 0 {
        WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: format!(
                "write-capable fanout '{}' completed with {} branch(es) needing review",
                call.id, needs_review
            ),
            ..WorkflowV2Result::default()
        }
    } else {
        WorkflowV2Result::accepted(format!(
            "write-capable fanout '{}' completed {} branch(es)",
            call.id,
            branch_results.len()
        ))
    };
    add_write_fanout_evidence(&mut result, plan, fallback_reason.clone());
    attach_branch_evidence(&mut result, &branch_results);
    result.data = serde_json::json!({
        "items": branch_results,
        "write_mode": plan.mode,
        "waves": plan.waves.iter().map(|wave| {
            serde_json::json!({
                "assignments": wave.assignments.iter().map(|assignment| {
                    serde_json::json!({
                        "item_id": assignment.item_id,
                        "owned_targets": assignment.owned_targets,
                        "worktree_path": assignment.worktree_path,
                    })
                }).collect::<Vec<_>>()
            })
        }).collect::<Vec<_>>(),
        "conflicts": plan.conflicts.iter().map(|conflict| {
            serde_json::json!({
                "left_item": conflict.left_item,
                "right_item": conflict.right_item,
                "target": conflict.target,
                "isolated_by_worktree": conflict.isolated_by_worktree,
            })
        }).collect::<Vec<_>>(),
        "peak_parallelism": peak_parallelism,
        "serial_fallback_reason": fallback_reason,
    });
    result
}

fn add_write_fanout_evidence(
    result: &mut WorkflowV2Result,
    plan: &WorkflowV2WritePlan,
    fallback_reason: Option<String>,
) {
    let status_evidence = match result.status {
        WorkflowV2Status::Blocked => Some((
            archon_workflow::WorkflowV2EvidenceKind::Blocker,
            "one or more write fanout branches returned a typed blocked result",
        )),
        WorkflowV2Status::NeedsReview => Some((
            archon_workflow::WorkflowV2EvidenceKind::Review,
            "write fanout branch findings were retained as typed review data for downstream workflow steps",
        )),
        WorkflowV2Status::Failed => Some((
            archon_workflow::WorkflowV2EvidenceKind::Blocker,
            "write fanout returned a terminal failed status",
        )),
        _ => None,
    };
    if let Some((kind, summary)) = status_evidence {
        result
            .evidence
            .push(archon_workflow::WorkflowV2Evidence::new(kind, summary));
    }
    let detail = fallback_reason.unwrap_or_else(|| {
        format!(
            "write-capable fanout used {:?} planning across {} wave(s)",
            plan.mode,
            plan.waves.len()
        )
    });
    result
        .evidence
        .push(archon_workflow::WorkflowV2Evidence::new(
            archon_workflow::WorkflowV2EvidenceKind::Implementation,
            detail,
        ));
}

fn count_results_with_status(results: &[WorkflowV2Result], status: WorkflowV2Status) -> usize {
    results
        .iter()
        .filter(|result| result.status == status)
        .count()
}

fn record_write_peak(peak: &AtomicUsize, observed: usize) {
    let mut current = peak.load(Ordering::SeqCst);
    while observed > current {
        match peak.compare_exchange(current, observed, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return,
            Err(next) => current = next,
        }
    }
}

fn recoverable_write_branch_error_result(item_id: &str, error: &str) -> WorkflowV2Result {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: format!(
            "write branch '{item_id}' produced recoverable implementation evidence requiring remediation"
        ),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "write branch validation failure was retained as remediation input instead of terminating the workflow",
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!("write_branch_repair_{}", sanitize_v2_path_segment(item_id)),
        description: truncate_for_result(error, 500),
        severity: Some("remediation".to_string()),
    });
    result.data = serde_json::json!({
        "branch_id": item_id,
        "normalized_from_error": true,
        "error": truncate_for_result(error, 2_000),
    });
    result
}

fn is_recoverable_write_branch_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    !lower.contains("agent transport failed")
        && (lower.contains("schema repair failed")
            || lower.contains("agent output contains a confirmation question")
            || lower.contains("workflowv2result object")
            || lower.contains("agent result failed validation")
            || lower.contains("implementation agent changed files outside declared target_files")
            || lower.contains("implementation noop requires typed task_coverage evidence")
            || lower
                .contains("implementation agent returned accepted status without changed files")
            || lower.contains("changed files outside declared ownership")
            || lower.contains("declares no target ownership")
            || lower.contains("verification blocked after patch")
            || lower.contains("output not usable")
            || lower.contains("malformedoutput")
            || lower.contains("invalid branch result"))
}

fn truncate_for_result(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for ch in value.chars().take(max_chars) {
        output.push(ch);
    }
    if value.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

fn sanitize_v2_path_segment(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use archon_workflow::spec::WORKFLOW_SCHEMA;
    use archon_workflow::{
        ArtifactPolicy, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FanoutItem,
        WorkflowV2FileRecord, WorkflowV2HostMethod, WorkflowV2HostOptions,
    };

    use super::*;

    fn spec_with_root(root: &Path) -> WorkflowSpec {
        WorkflowSpec {
            schema: WORKFLOW_SCHEMA.to_string(),
            name: "test".to_string(),
            task: "test".to_string(),
            target_repository_root: Some(root.display().to_string()),
            max_parallelism: 4,
            max_agents: 16,
            provider_tiers: BTreeMap::new(),
            stages: Vec::new(),
            artifact_policy: ArtifactPolicy::default(),
            permissions: BTreeMap::new(),
            quality_gates: BTreeMap::new(),
            learning_hooks: Vec::new(),
        }
    }

    #[test]
    fn target_files_from_fanout_item_are_required_for_write_branches() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec = spec_with_root(temp.path());
        let call = WorkflowV2HostCall {
            id: "impl".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions {
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        };
        let branch = WorkflowV2FanoutItem::read_only(
            "impl-T001",
            "coder",
            call.clone(),
            serde_json::json!({
                "item": {
                    "task_id": "T001",
                    "target_files": ["src/lib.rs"]
                }
            }),
        );

        let targets = target_files_for_branch(&spec, &call, &branch).expect("target files");

        assert_eq!(targets, vec!["src/lib.rs"]);
    }

    #[test]
    fn item_target_files_override_static_fallback_targets_for_write_branches() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let spec = spec_with_root(&repo);
        let call = WorkflowV2HostCall {
            id: "impl".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: WorkflowV2HostOptions {
                target_files: vec![repo.display().to_string()],
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        };
        let branch = WorkflowV2FanoutItem::read_only(
            "impl-T001",
            "coder",
            call.clone(),
            serde_json::json!({
                "item": {
                    "task_id": "T001",
                    "target_files": ["crates/archon-trading/src/data_lake.rs"]
                }
            }),
        );

        let targets = target_files_for_branch(&spec, &call, &branch).expect("target files");

        assert_eq!(targets, vec!["crates/archon-trading/src/data_lake.rs"]);
    }

    #[test]
    fn repo_root_fallback_without_item_targets_is_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let spec = spec_with_root(&repo);
        let call = WorkflowV2HostCall {
            id: "impl".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: WorkflowV2HostOptions {
                target_files: vec![repo.display().to_string()],
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        };
        let branch = WorkflowV2FanoutItem::read_only(
            "impl-T001",
            "coder",
            call.clone(),
            serde_json::json!({
                "item": {
                    "task_id": "T001"
                }
            }),
        );

        let error = target_files_for_branch(&spec, &call, &branch).expect_err("repo root target");

        assert!(error.to_string().contains("resolves to repository root"));
    }

    #[test]
    fn absolute_item_target_inside_repository_is_made_relative() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("crates/example/src")).expect("repo");
        let spec = spec_with_root(&repo);
        let call = WorkflowV2HostCall {
            id: "impl".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Serial),
            options: WorkflowV2HostOptions {
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        };
        let target = repo.join("crates/example/src/lib.rs");
        let branch = WorkflowV2FanoutItem::read_only(
            "impl-T001",
            "coder",
            call.clone(),
            serde_json::json!({
                "item": {
                    "id": "T001",
                    "target_files": [target.display().to_string()]
                }
            }),
        );

        let targets = target_files_for_branch(&spec, &call, &branch).expect("target files");

        assert_eq!(targets, vec!["crates/example/src/lib.rs"]);
    }

    #[test]
    fn absolute_item_target_outside_repository_is_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let spec = spec_with_root(&repo);
        let call = WorkflowV2HostCall {
            id: "impl".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Serial),
            options: WorkflowV2HostOptions {
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        };
        let branch = WorkflowV2FanoutItem::read_only(
            "impl-T001",
            "coder",
            call.clone(),
            serde_json::json!({
                "item": {
                    "id": "T001",
                    "target_files": [temp.path().join("other/src/lib.rs").display().to_string()]
                }
            }),
        );

        let error = target_files_for_branch(&spec, &call, &branch).expect_err("outside repo");

        assert!(error.to_string().contains("outside target_repository_root"));
    }

    #[test]
    fn write_fanout_result_records_serial_fallback_reason() {
        let temp = tempfile::tempdir().expect("tempdir");
        let call = WorkflowV2HostCall {
            id: "impl".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: WorkflowV2HostOptions::default(),
        };
        let plan = WorkflowV2WritePlanner::new(temp.path())
            .plan(&[WorkflowV2WriteItem::new(
                "impl-T001",
                WorkflowV2WriteMode::Worktree,
                vec!["src/lib.rs".to_string()],
            )])
            .expect("write plan");
        let mut branch_result = WorkflowV2Result::accepted("changed file");
        branch_result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Implementation,
            "changed src/lib.rs",
        ));
        branch_result
            .files_changed
            .push(WorkflowV2FileRecord::new("src/lib.rs"));

        let result = result_from_write_fanout(
            &call,
            vec![branch_result],
            &plan,
            1,
            Some("workspace boundary support is unavailable; serialized fallback used".to_string()),
        );

        assert_eq!(result.status, WorkflowV2Status::Accepted);
        assert_eq!(
            result
                .data
                .get("serial_fallback_reason")
                .and_then(serde_json::Value::as_str),
            Some("workspace boundary support is unavailable; serialized fallback used")
        );
    }

    #[test]
    fn worktree_assignment_builds_coordinator_plan_with_isolated_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).expect("repo");
        std::fs::write(repo.join("src/lib.rs"), "pub fn existing() {}\n").expect("file");
        let assignment = WorkflowV2WriteAssignment {
            item_id: "impl-T001".to_string(),
            owned_targets: vec!["src/lib.rs".to_string()],
            worktree_path: Some(temp.path().join("wt").display().to_string()),
        };

        let plan = coordinator_plan_for_assignment("wf-test", "impl", &assignment, &repo)
            .expect("coordinator plan");

        assert_eq!(plan.item_id, "impl-T001");
        assert_eq!(plan.stage_id, "impl");
        assert_eq!(plan.isolated_root, temp.path().join("wt"));
        assert_eq!(plan.target_files[0].as_str(), "src/lib.rs");
        assert!(plan.workspace_boundary_required);
    }

    #[test]
    fn worktree_write_result_does_not_record_serial_fallback_when_active() {
        let temp = tempfile::tempdir().expect("tempdir");
        let call = WorkflowV2HostCall {
            id: "impl".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: WorkflowV2HostOptions::default(),
        };
        let plan = WorkflowV2WritePlanner::new(temp.path())
            .plan(&[WorkflowV2WriteItem::new(
                "impl-T001",
                WorkflowV2WriteMode::Worktree,
                vec!["src/lib.rs".to_string()],
            )])
            .expect("write plan");
        let mut branch_result = WorkflowV2Result::accepted("changed file");
        branch_result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Implementation,
            "changed src/lib.rs in isolated worktree",
        ));
        branch_result
            .files_changed
            .push(WorkflowV2FileRecord::new("src/lib.rs"));

        let result = result_from_write_fanout(&call, vec![branch_result], &plan, 1, None);

        assert_eq!(result.status, WorkflowV2Status::Accepted);
        assert_eq!(
            result
                .data
                .get("serial_fallback_reason")
                .and_then(serde_json::Value::as_str),
            None
        );
        assert!(result.evidence.iter().any(|evidence| {
            evidence
                .summary
                .contains("write-capable fanout used Worktree")
        }));
    }

    #[test]
    fn write_fanout_review_branch_stays_needs_review_not_failed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let call = WorkflowV2HostCall {
            id: "impl".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        };
        let plan = WorkflowV2WritePlanner::new(temp.path())
            .plan(&[WorkflowV2WriteItem::new(
                "impl-T001",
                WorkflowV2WriteMode::Coordinated,
                vec!["src/lib.rs".to_string()],
            )])
            .expect("write plan");
        let mut branch_result = WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: "implementation needs remediation".to_string(),
            ..WorkflowV2Result::default()
        };
        branch_result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Review,
            "reviewed implementation branch",
        ));

        let result = result_from_write_fanout(&call, vec![branch_result], &plan, 1, None);

        assert_eq!(result.status, WorkflowV2Status::NeedsReview);
        assert!(result.summary.contains("needing review"));
    }

    #[test]
    fn write_fanout_failed_branch_feeds_remediation_not_terminal_failure() {
        let temp = tempfile::tempdir().expect("tempdir");
        let call = WorkflowV2HostCall {
            id: "impl".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        };
        let plan = WorkflowV2WritePlanner::new(temp.path())
            .plan(&[WorkflowV2WriteItem::new(
                "impl-T001",
                WorkflowV2WriteMode::Coordinated,
                vec!["src/lib.rs".to_string()],
            )])
            .expect("write plan");
        let mut branch_result = WorkflowV2Result {
            status: WorkflowV2Status::Failed,
            summary: "focused implementation failed and needs another pass".to_string(),
            ..WorkflowV2Result::default()
        };
        branch_result.residual_gaps.push(WorkflowV2ResidualGap {
            id: "gap".to_string(),
            description: "implementation branch did not satisfy acceptance criteria".to_string(),
            severity: Some("high".to_string()),
        });

        let result = result_from_write_fanout(&call, vec![branch_result], &plan, 1, None);

        assert_eq!(result.status, WorkflowV2Status::NeedsReview);
        assert!(result.summary.contains("needing remediation"));
        assert_eq!(result.residual_gaps.len(), 1);
    }

    #[test]
    fn recoverable_write_branch_validation_error_becomes_remediation_input() {
        let result = recoverable_write_branch_error_result(
            "impl-T001",
            "schema repair failed after one retry: first=implementation agent changed files outside declared target_files; repair=implementation noop requires typed task_coverage evidence",
        );

        assert_eq!(result.status, WorkflowV2Status::NeedsReview);
        assert_eq!(
            result.residual_gaps[0].severity.as_deref(),
            Some("remediation")
        );
        assert_eq!(result.data["normalized_from_error"], true);
    }

    #[test]
    fn transport_error_is_not_reclassified_as_recoverable_write_branch_gap() {
        assert!(!is_recoverable_write_branch_error(
            "agent transport failed: rate limit"
        ));
    }
}
