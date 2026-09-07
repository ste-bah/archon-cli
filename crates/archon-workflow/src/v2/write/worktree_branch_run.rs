//! One worktree branch: dispatch the agent, validate, capture the manifest.
use super::*;

pub(crate) async fn run_one_worktree_branch(
    ctx: WorktreeWaveRunContext<'_>,
    prepared: PreparedWorktreeBranch,
) -> crate::WorkflowResult<CompletedWorktreeBranch> {
    let branch = prepare_worktree_branch_execution(
        ctx.execution,
        ctx.store_for_control,
        ctx.run_id,
        &prepared,
    )?;
    // An earlier branch may have left partial work for this task; it is on
    // disk already, and the agent is told to continue rather than start over.
    let task =
        super::partial_work::with_resume_preamble(ctx.task, prepared.resumed_partial.as_ref());
    // Wrapped at the branch, not at the dispatch inside it, and deliberately:
    // this covers the whole re-ask loop, so a cancelled run stops re-asking
    // rather than working through its remaining size and transport budgets
    // first. The checkpoint below still runs for a stop observed between calls.
    let mut result = crate::control_race::until_run_stops(
        ctx.store_for_control,
        ctx.run_id,
        &branch.id,
        run_worktree_branch_agent(
            &task,
            ctx.target_repository_root.map(str::to_string),
            ctx.dispatch,
            ctx.v2_store,
            ctx.adapter,
            &branch,
            ctx.task_universe,
        ),
    )
    .await?;
    poll_v2_run_control(ctx.store_for_control, ctx.run_id, &branch.id)?;
    // Answered against the declared baseline BEFORE validation, because both
    // `validate_worktree_branch_result` and `capture_worktree_branch_manifest`
    // replace `*result` wholesale on rejection — an ownership or size-policy
    // rejection would otherwise discard the very marker that records it landed
    // nothing. The verdict is captured here and stamped last, so it survives
    // whichever result object comes out the far end.
    let landed = worktree_patch_landed(&prepared);
    let schema_repair_failed = is_schema_repair_failure_result(&result);
    validate_worktree_branch_result(
        &mut result,
        &branch,
        &prepared.assignment,
        ctx.v2_store,
        ctx.canonical_root.to_str(),
    )?;
    let (manifest, pre_hashes) = capture_worktree_branch_manifest(
        ctx.run_root,
        ctx.run_id,
        ctx.execution,
        ctx.cfg,
        ctx.v2_store,
        &mut result,
        &prepared,
    )?;
    mark_patch_landed(&mut result, &prepared, landed, schema_repair_failed);
    Ok(completed_worktree_branch(
        branch, result, manifest, pre_hashes,
    ))
}
