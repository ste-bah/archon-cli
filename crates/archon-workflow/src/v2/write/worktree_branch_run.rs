//! One worktree branch: dispatch the agent, validate, capture the manifest.
use super::*;

pub(crate) async fn run_one_worktree_branch(
    ctx: WorktreeWaveRunContext<'_>,
    prepared: PreparedWorktreeBranch,
) -> crate::WorkflowResult<CompletedWorktreeBranch> {
    let delivery = super::super::delivery::ArtifactDelivery::capture(&prepared, ctx.v2_store);
    let mut branch = prepare_worktree_branch_execution(
        ctx.execution,
        ctx.store_for_control,
        ctx.run_id,
        &prepared,
    )?;
    // The request builder renders `call.options.task` when the call carries
    // one, which every fanout branch does, so the host preamble (budget,
    // write-first rule, resumed partial) must go there, not on the fallback.
    let task = ctx.task.to_string();
    let mut rendered = branch
        .execution
        .call
        .options
        .task
        .clone()
        .unwrap_or_else(|| task.clone());
    rendered.push_str(&super::super::audit_gate::preamble(ctx.v2_store, &prepared.assignment.owned_targets)?);
    let source = prepared.branch.input.get("item").unwrap_or(&prepared.branch.input);
    let task_ids = crate::generated_contract::canonical_task_ids_from_generated_value(source, ctx.task_universe);
    // Kept so a session restarted mid-attempt (transport drop, host timeout)
    // can be told what its worktree holds by then, not what it held here.
    branch.refresh = Some(super::partial_work::BranchTaskRefresh {
        base_task: rendered.clone(),
        task_ids: task_ids.clone(),
        run_root: ctx.run_root.to_path_buf(),
        stage_id: ctx.execution.call.id.clone(),
        item_id: branch.id.clone(),
    });
    let rendered = crate::v2::write_read_set::with_retry_preamble(&rendered, ctx.v2_store, &task_ids);
    // The budget the agent is told is the one that will actually end its
    // session: the host's per-dispatch timeout when that is the smaller.
    branch.execution.call.options.task = Some(super::partial_work::with_host_preamble(
        &rendered,
        super::partial_work::effective_call_budget(
            ctx.dispatch.dispatch_timeout(),
            ctx.dispatch.call_time_budget(),
            std::time::Duration::ZERO,
        ),
        prepared.resumed_partial.as_ref(),
    ));
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
    let (mut manifest, pre_hashes) = capture_worktree_branch_manifest(
        ctx.run_root,
        ctx.run_id,
        ctx.execution,
        ctx.cfg,
        ctx.v2_store,
        &mut result,
        &prepared,
    )?;
    mark_patch_landed(&mut result, &prepared, landed, schema_repair_failed);
    delivery.stamp(&mut result, landed);
    super::super::audit_gate::enforce(ctx.v2_store, &prepared.assignment.owned_targets, &mut result, &mut manifest)?;
    // The dependency gate reads landed tasks from saved outcomes (TD-058).
    super::dependency_gate::stamp_canonical_task_ids(
        &mut result,
        &prepared.branch.input,
        ctx.task_universe,
    );
    Ok(completed_worktree_branch(
        branch, result, manifest, pre_hashes,
    ))
}
