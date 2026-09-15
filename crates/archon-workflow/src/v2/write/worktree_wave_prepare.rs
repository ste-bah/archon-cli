//! Prepare one wave: a sealed worktree per assignment, partial work resumed.
use super::*;
use crate::write_coordinator::worktree_isolation::{capture_sealed_source, create_item_workspace_from_sealed};

pub(super) async fn prepare_worktree_wave(
    wave: &WorkflowV2WriteWave,
    branches: &[crate::WorkflowV2FanoutItem],
    run_id: &str,
    call_id: &str,
    canonical_root: &Path,
    cfg: &WriteCoordinatorConfig,
    store_for_control: &crate::WorkflowStore,
    v2_store: &WorkflowV2ResultStore,
    task_universe: Option<&crate::task_universe::WorkflowV2TaskUniverse>,
    dispatch: &dyn WorkflowAgentDispatch,
) -> crate::WorkflowResult<Vec<PreparedWorktreeBranch>> {
    // One list per wave, shared by every branch in it: ownership is a property
    // of the wave, and recomputing it per branch would let two branches
    // disagree about who owns what.
    let wave_claims = crate::v2::write_scope_extension::wave_claims_for(wave);
    let plans = wave.assignments.iter().map(|assignment| {
        coordinator_plan_for_assignment(run_id, call_id, assignment, canonical_root)
    }).collect::<crate::WorkflowResult<Vec<_>>>()?;
    let Some(mut union) = plans.first().cloned() else { return Ok(Vec::new()); };
    for plan in &plans[1..] {
        for target in &plan.target_files {
            if !union.target_files.contains(target) { union.target_files.push(target.clone()); }
        }
        for input in &plan.verify_inputs {
            if !union.verify_inputs.contains(input) { union.verify_inputs.push(input.clone()); }
        }
    }
    let source = capture_sealed_source(canonical_root, &union, cfg)
        .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
    if let Some(audit) = dispatch.repository_audit() {
        let snapshot = crate::repository_audit::runtime::Snapshot::from_sealed(canonical_root, &source, &union, v2_store)?;
        let paths = union.target_files.iter().map(|p|p.as_str().to_string()).collect::<Vec<_>>();
        // Issue-25: a tree that an apply receipt explains is the post-apply
        // audit a pause interrupted, not a foreign edit against the allowance.
        let receipts = crate::repository_audit::receipts::read_apply_receipts(&audit.store, &audit.run_id)?;
        let run_root = v2_store.root().parent().map(Path::to_path_buf).unwrap_or_else(|| v2_store.root().to_path_buf());
        let refresh = super::audit_refresh::refresh_trigger(audit.state()?.snapshot.as_ref(), &snapshot, &receipts, &run_root, canonical_root)?;
        audit.assess_with(&snapshot, &paths, refresh.trigger, refresh.event_detail(), dispatch).await?;
    }
    let mut prepared = Vec::new();
    for (assignment, coordinator_plan) in wave.assignments.iter().zip(plans) {
        let branch = branch_for_assignment(branches, assignment)?;
        poll_v2_run_control(store_for_control, run_id, &branch.id)?;
        let baseline = source.baseline_for(&coordinator_plan);
        let workspace = create_item_workspace_from_sealed(canonical_root, &coordinator_plan, &source)
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
