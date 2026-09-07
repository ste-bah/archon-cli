//! Prepare one wave: a sealed worktree per assignment, partial work resumed.
use super::*;

pub(super) fn prepare_worktree_wave(
    wave: &WorkflowV2WriteWave,
    branches: &[crate::WorkflowV2FanoutItem],
    run_id: &str,
    call_id: &str,
    canonical_root: &Path,
    cfg: &WriteCoordinatorConfig,
    store_for_control: &crate::WorkflowStore,
    v2_store: &WorkflowV2ResultStore,
    task_universe: Option<&crate::task_universe::WorkflowV2TaskUniverse>,
) -> crate::WorkflowResult<Vec<PreparedWorktreeBranch>> {
    // One list per wave, shared by every branch in it: ownership is a property
    // of the wave, and recomputing it per branch would let two branches
    // disagree about who owns what.
    let wave_claims = crate::v2::write_scope_extension::wave_claims_for(wave);
    let mut prepared = Vec::new();
    for assignment in &wave.assignments {
        let branch = branch_for_assignment(branches, assignment)?;
        poll_v2_run_control(store_for_control, run_id, &branch.id)?;
        let coordinator_plan =
            coordinator_plan_for_assignment(run_id, call_id, assignment, canonical_root)?;
        let baseline = capture_canonical_baseline(
            canonical_root,
            &coordinator_plan,
            &coordinator_plan.verify_inputs,
            cfg,
        )
        .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
        let workspace = create_item_workspace(canonical_root, &coordinator_plan, &baseline)
            .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
        let resumed_partial = super::partial_work::resume_into_workspace(
            v2_store,
            task_universe,
            &branch.input,
            &workspace.plan.isolated_root,
        );
        prepared.push(PreparedWorktreeBranch {
            branch,
            assignment: assignment.clone(),
            wave_claims: wave_claims.clone(),
            coordinator_plan,
            baseline,
            workspace,
            resumed_partial,
        });
    }
    Ok(prepared)
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
