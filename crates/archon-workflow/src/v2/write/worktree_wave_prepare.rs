//! Prepare one wave: a sealed worktree per assignment, its test baseline
//! established (Obs-31), partial work resumed.
use super::*;
use crate::write_coordinator::worktree_isolation::{
    capture_sealed_source, create_item_workspace_from_sealed, extend_baseline_with_granted_targets,
};

/// A branch with its worktree created and nothing in it yet but the base
/// commit: the state the baseline must be run in.
struct StagedBranch {
    branch: crate::WorkflowV2FanoutItem,
    assignment: WorkflowV2WriteAssignment,
    coordinator_plan: WritePlan,
    baseline: CanonicalBaseline,
    workspace: ItemWorkspace,
}

pub(super) async fn prepare_worktree_wave(
    ctx: &WorktreePlanRunContext<'_>,
    wave: &WorkflowV2WriteWave,
    branches: &[crate::WorkflowV2FanoutItem],
) -> crate::WorkflowResult<Vec<PreparedWorktreeBranch>> {
    let run_id = ctx.run_id;
    let call_id = ctx.execution.call.id.as_str();
    let canonical_root = ctx.setup.canonical_root.as_path();
    let cfg = &ctx.setup.cfg;
    let v2_store = ctx.v2_store;
    let task_universe = ctx.task_universe;
    let dispatch = ctx.dispatch;
    // One list per wave, shared by every branch in it: ownership is a property
    // of the wave, and recomputing it per branch would let two branches
    // disagree about who owns what.
    let mut wave_claims = crate::v2::write_scope_extension::wave_claims_for(wave);
    let plans = wave
        .assignments
        .iter()
        .map(|assignment| {
            coordinator_plan_for_assignment(run_id, call_id, assignment, canonical_root)
        })
        .collect::<crate::WorkflowResult<Vec<_>>>()?;
    let Some(mut union) = plans.first().cloned() else {
        return Ok(Vec::new());
    };
    for plan in &plans[1..] {
        for target in &plan.target_files {
            if !union.target_files.contains(target) {
                union.target_files.push(target.clone());
            }
        }
        for input in &plan.verify_inputs {
            if !union.verify_inputs.contains(input) {
                union.verify_inputs.push(input.clone());
            }
        }
    }
    let source = capture_sealed_source(canonical_root, &union, cfg)
        .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
    if let Some(audit) = dispatch.repository_audit() {
        let snapshot = crate::repository_audit::runtime::Snapshot::from_sealed(
            canonical_root,
            &source,
            &union,
            v2_store,
        )?;
        let paths = union
            .target_files
            .iter()
            .map(|p| p.as_str().to_string())
            .collect::<Vec<_>>();
        // Issue-25: a tree that an apply receipt explains is the post-apply
        // audit a pause interrupted, not a foreign edit against the allowance.
        let receipts =
            crate::repository_audit::receipts::read_apply_receipts(&audit.store, &audit.run_id)?;
        let run_root = v2_store
            .root()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| v2_store.root().to_path_buf());
        let refresh = super::audit_refresh::refresh_trigger(
            audit.state()?.snapshot.as_ref(),
            &snapshot,
            &receipts,
            &run_root,
            canonical_root,
        )?;
        audit
            .assess_with(
                &snapshot,
                &paths,
                refresh.trigger,
                refresh.event_detail(),
                dispatch,
            )
            .await?;
    }
    let mut staged = Vec::new();
    for (assignment, coordinator_plan) in wave.assignments.iter().zip(plans) {
        let branch = branch_for_assignment(branches, assignment)?;
        poll_v2_run_control(ctx.store_for_control, run_id, &branch.id)?;
        let baseline = source.baseline_for(&coordinator_plan);
        let workspace =
            create_item_workspace_from_sealed(canonical_root, &coordinator_plan, &source)
                .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
        staged.push(StagedBranch {
            branch,
            assignment: assignment.clone(),
            coordinator_plan,
            baseline,
            workspace,
        });
    }
    // Obs-31: the declared focused tests run on the base commit NOW, in the
    // pristine worktrees, before partial work is resumed into them and
    // before any coder is dispatched.
    let requests: Vec<_> = staged
        .iter()
        .map(|s| baseline_request(s, task_universe))
        .collect();
    let baselines = super::test_baseline_wave::establish_wave(
        &super::test_baseline_wave::WaveBaselineContext {
            store: v2_store,
            dispatch,
            universe: task_universe,
            stage_id: call_id,
            base_commit: &source.base_commit,
            parallelism: dispatch.fanout_parallelism(ctx.execution.call.options.max_parallelism),
        },
        &requests,
    )
    .await;
    let mut prepared = Vec::new();
    for (mut staged, test_baseline) in staged.into_iter().zip(baselines) {
        widen_to_obligations(
            &mut staged,
            &test_baseline,
            &mut wave_claims,
            canonical_root,
        );
        let focused_test_targets =
            widen_to_focused_tests(&mut staged, task_universe, &mut wave_claims, canonical_root);
        let resumed_partial = super::partial_work::resume_into_workspace(
            v2_store,
            task_universe,
            &staged.branch.input,
            &staged.workspace.plan.isolated_root,
        );
        prepared.push(PreparedWorktreeBranch {
            branch: staged.branch,
            assignment: staged.assignment,
            wave_claims: Vec::new(),
            coordinator_plan: staged.coordinator_plan,
            baseline: staged.baseline,
            workspace: staged.workspace,
            resumed_partial,
            test_baseline: Some(test_baseline),
            focused_test_targets,
        });
    }
    // Stamped last: every branch's obligations have widened the claims by now.
    for branch in &mut prepared {
        branch.wave_claims = wave_claims.clone();
    }
    Ok(prepared)
}

fn baseline_request(
    staged: &StagedBranch,
    task_universe: Option<&crate::task_universe::WorkflowV2TaskUniverse>,
) -> super::test_baseline_wave::BranchBaselineRequest {
    let source = staged
        .branch
        .input
        .get("item")
        .unwrap_or(&staged.branch.input);
    let task_ids = canonical_task_ids_from_generated_value(source, task_universe);
    let forbidden = task_universe
        .map(|universe| {
            super::forbidden_paths::forbidden_paths_for_item(universe, &task_ids, source)
        })
        .unwrap_or_default();
    super::test_baseline_wave::BranchBaselineRequest {
        branch_id: staged.branch.id.clone(),
        task_ids,
        commands: crate::agent_dispatch_port::declared_focused_tests(&staged.branch.input),
        worktree: staged.workspace.plan.isolated_root.clone(),
        targets: staged
            .coordinator_plan
            .target_files
            .iter()
            .map(|p| p.as_str().to_string())
            .collect(),
        forbidden,
    }
}

/// The files of this branch's baseline obligations become declared targets:
/// on the coordinator plan (gates 2 and 3, the scope roots), on the
/// assignment (gate 1 and the adapter's claims), on the wave claims (so a
/// sibling cannot be granted the same file), and in the sealed baseline (so
/// the apply-time stale recheck still covers them).
fn widen_to_obligations(
    staged: &mut StagedBranch,
    test_baseline: &super::test_baseline::BranchBaseline,
    wave_claims: &mut [crate::v2::write_scope_extension::WaveClaim],
    canonical_root: &Path,
) {
    let files = test_baseline.obligation_files();
    if files.is_empty() {
        return;
    }
    let mut added = Vec::new();
    for file in &files {
        let Ok(normalized) = normalize_target(file, canonical_root) else {
            continue;
        };
        if staged.coordinator_plan.target_files.contains(&normalized) {
            continue;
        }
        staged.coordinator_plan.target_files.push(normalized);
        added.push(file.clone());
    }
    if added.is_empty() {
        return;
    }
    staged.coordinator_plan.target_files.sort();
    staged.coordinator_plan.target_files.dedup();
    for file in &added {
        if !staged.assignment.owned_targets.contains(file) {
            staged.assignment.owned_targets.push(file.clone());
        }
    }
    if let Some(claim) = wave_claims
        .iter_mut()
        .find(|c| c.item_id == staged.assignment.item_id)
    {
        claim.owned.extend(added.iter().cloned());
    }
    staged.baseline =
        extend_baseline_with_granted_targets(&staged.baseline, canonical_root, &added);
}

/// Issue-71: the files the branch's declared focused-test commands resolve
/// to (and their module directories) become declared targets the same way
/// the obligation files do — plan, assignment, wave claim, sealed baseline
/// — unless another task declares them. Returns what was widened, files
/// and directories (trailing `/`), sorted, and which filters were
/// ambiguous (Issue-72), for the preamble and the result.
fn widen_to_focused_tests(
    staged: &mut StagedBranch,
    task_universe: Option<&crate::task_universe::WorkflowV2TaskUniverse>,
    wave_claims: &mut [crate::v2::write_scope_extension::WaveClaim],
    canonical_root: &Path,
) -> super::focused_test_targets::FocusedTestTargets {
    let commands = crate::agent_dispatch_port::declared_focused_tests(&staged.branch.input);
    if commands.is_empty() {
        return Default::default();
    }
    let source = staged
        .branch
        .input
        .get("item")
        .unwrap_or(&staged.branch.input);
    let task_ids = canonical_task_ids_from_generated_value(source, task_universe);
    let own_targets: Vec<String> = staged
        .coordinator_plan
        .target_files
        .iter()
        .map(|p| p.as_str().to_string())
        .collect();
    let widenable = super::focused_test_targets::widenable(
        canonical_root,
        task_universe,
        &task_ids,
        &own_targets,
        &commands,
    );
    let mut recorded = Vec::new();
    let mut added_files = Vec::new();
    for file in widenable.files {
        let Ok(normalized) = normalize_target(&file, canonical_root) else {
            continue;
        };
        if staged.coordinator_plan.target_files.contains(&normalized) {
            continue;
        }
        staged.coordinator_plan.target_files.push(normalized);
        added_files.push(file.clone());
        recorded.push(file);
    }
    let mut added_dirs = Vec::new();
    for dir in widenable.dirs {
        let Ok(normalized) = normalize_target(&dir, canonical_root) else {
            continue;
        };
        if staged
            .coordinator_plan
            .target_dir_scopes
            .contains(&normalized)
        {
            continue;
        }
        staged.coordinator_plan.target_dir_scopes.push(normalized);
        added_dirs.push(dir.clone());
        recorded.push(format!("{dir}/"));
    }
    let ambiguous = widenable.ambiguous;
    if recorded.is_empty() {
        return super::focused_test_targets::FocusedTestTargets {
            widened: recorded,
            ambiguous,
        };
    }
    staged.coordinator_plan.target_files.sort();
    staged.coordinator_plan.target_files.dedup();
    staged.coordinator_plan.target_dir_scopes.sort();
    staged.coordinator_plan.target_dir_scopes.dedup();
    for file in &added_files {
        if !staged.assignment.owned_targets.contains(file) {
            staged.assignment.owned_targets.push(file.clone());
        }
    }
    for dir in &added_dirs {
        if !staged.assignment.owned_scopes.contains(dir) {
            staged.assignment.owned_scopes.push(dir.clone());
        }
    }
    if let Some(claim) = wave_claims
        .iter_mut()
        .find(|c| c.item_id == staged.assignment.item_id)
    {
        claim.owned.extend(added_files.iter().cloned());
        claim.owned.extend(added_dirs.iter().cloned());
    }
    staged.baseline =
        extend_baseline_with_granted_targets(&staged.baseline, canonical_root, &added_files);
    recorded.sort();
    super::focused_test_targets::FocusedTestTargets {
        widened: recorded,
        ambiguous,
    }
}

pub(super) fn branch_for_assignment(
    branches: &[crate::WorkflowV2FanoutItem],
    assignment: &WorkflowV2WriteAssignment,
) -> crate::WorkflowResult<crate::WorkflowV2FanoutItem> {
    branches
        .iter()
        .find(|branch| branch.id == assignment.item_id)
        .cloned()
        .ok_or_else(|| {
            WorkflowError::SpecInvalid(format!(
                "write plan referenced missing fanout item '{}'",
                assignment.item_id
            ))
        })
}

#[cfg(test)]
#[path = "worktree_wave_prepare_tests.rs"]
mod tests;
