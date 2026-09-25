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
    rendered.push_str(&super::super::audit_gate::preamble(
        ctx.v2_store,
        &prepared.assignment.owned_targets,
    )?);
    // Issue-27: the ceiling the grant will apply, told to the agent in the
    // same words and from the same plan, so prompt and gate agree.
    rendered.push_str(&super::scope_roots::scope_roots(&prepared.coordinator_plan).preamble());
    let source = prepared
        .branch
        .input
        .get("item")
        .unwrap_or(&prepared.branch.input);
    let task_ids = crate::generated_contract::canonical_task_ids_from_generated_value(
        source,
        ctx.task_universe,
    );
    // Issue-30: the paths the branch's tasks forbid, resolved ONCE and read
    // three times — the preamble here, the tool guard through the input
    // stamp, and the capture backstop in the grant below.
    let forbidden = ctx
        .task_universe
        .map(|universe| super::forbidden_paths::forbidden_paths(universe, &task_ids))
        .unwrap_or_default();
    rendered.push_str(&super::forbidden_paths::preamble(&forbidden));
    super::forbidden_paths::stamp(&mut branch.execution.input, &forbidden);
    // Issue-64: the target set the gates will judge the patch against — the
    // plan's targets as the baseline widened them, plus its directory scopes
    // — stamped for the tool guard, so a write outside it is refused when it
    // is attempted rather than dropped four hours later.
    super::declared_targets::stamp(&mut branch.execution.input, &prepared.coordinator_plan);
    // Issue-52: the caps `validate_patch` will refuse the whole patch over,
    // from the config it will be handed, with each declared target's spent
    // lines. Appended HERE, before `rendered` becomes the restart base and
    // the retry task below, so every session of this branch is told them.
    rendered.push_str(&super::landing_policy::preamble(ctx.cfg, source));
    // Obs-31: which tests in its declared filter were already red on the
    // base commit, and which of those are its own to make pass. Same place
    // as the landing policy, for the same reason: every session sees it.
    if let Some(test_baseline) = &prepared.test_baseline {
        rendered.push_str(&super::test_baseline_preamble::preamble(test_baseline));
    }
    // Issue-71: the test modules its declared filters name are writable;
    // the coder is told so from the same list the stamp above carries.
    rendered.push_str(&super::focused_test_targets::preamble(
        &prepared.focused_test_targets,
    ));
    // Kept so a session restarted mid-attempt (transport drop, host timeout)
    // can be told what its worktree holds by then, not what it held here.
    branch.refresh = Some(super::partial_work::BranchTaskRefresh {
        base_task: rendered.clone(),
        task_ids: task_ids.clone(),
        run_root: ctx.run_root.to_path_buf(),
        stage_id: ctx.execution.call.id.clone(),
        item_id: branch.id.clone(),
    });
    let rendered =
        crate::v2::write_read_set::with_retry_preamble(&rendered, ctx.v2_store, &task_ids);
    // What earlier attempts at these tasks were refused and last ran, so a
    // resumed session does not spend its first minutes repeating them.
    let memory = super::session_memory::SessionMemory::for_tasks(
        ctx.v2_store,
        &task_ids,
        ctx.dispatch.resume_memory_calls(),
    );
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
        &memory,
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
            ctx.adapter.clone(),
            &branch,
            ctx.task_universe,
        ),
    )
    .await?;
    poll_v2_run_control(ctx.store_for_control, ctx.run_id, &branch.id)?;
    // The host cut the session with work on disk: re-ask once in the same
    // worktree, told what it holds, before the wave is allowed to stall.
    if super::worktree_branch_retry::timed_out_with_work_unjudged(&result)
        && let Ok(Some(partial)) = super::partial_work::capture_partial_work(
            &branch.workspace_root,
            ctx.run_root,
            &ctx.execution.call.id,
            &branch.id,
            &task_ids,
            Some(super::partial_work::PartialOrigin::from_result(&result)),
        )
        && !partial.files.is_empty()
    {
        super::worktree_branch_retry::record_retry_row(
            ctx.v2_store,
            &ctx.execution.call.id,
            &branch.id,
            &partial,
        );
        let retry = super::worktree_branch_retry::retry_execution(
            &branch,
            &rendered,
            &partial,
            super::worktree_branch_retry::retry_budget(ctx.dispatch),
            &super::session_memory::SessionMemory::for_branch(
                ctx.v2_store,
                &branch.execution.call.id,
                ctx.dispatch.resume_memory_calls(),
            ),
        );
        let second = crate::control_race::until_run_stops(
            ctx.store_for_control,
            ctx.run_id,
            &branch.id,
            run_worktree_branch_agent(
                &task,
                ctx.target_repository_root.map(str::to_string),
                ctx.dispatch,
                ctx.v2_store,
                ctx.adapter.clone(),
                &retry,
                ctx.task_universe,
            ),
        )
        .await?;
        poll_v2_run_control(ctx.store_for_control, ctx.run_id, &branch.id)?;
        result = super::worktree_branch_retry::settle(result, second);
        // The patch under `partial/` was captured at the FIRST cut. A retry
        // that did not land still edited the worktree for its whole budget,
        // and the wave only re-captures once every branch in it has finished
        // — a run stopped before then keeps the stale patch. Live: a 20:45
        // cut whose partial still carried the 20:15 capture. Best effort, as
        // the first capture is.
        if !matches!(
            result.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        ) {
            let _ = super::partial_work::capture_partial_work(
                &branch.workspace_root,
                ctx.run_root,
                &ctx.execution.call.id,
                &branch.id,
                &task_ids,
                Some(super::partial_work::PartialOrigin::from_result(&result)),
            );
        }
    }
    // Issue-76: a copy of one of the host's own bookkeeping files is removed
    // from the worktree FIRST — before the grant reads the worktree and
    // before any gate reads either — so it is never a grant candidate, never
    // declared, never in the diff. A path at the repository root is exempt
    // from the scope-roots drop below, which is exactly how the live copy was
    // granted, declared and committed into the target repository.
    let host_internal_dropped = super::host_internal_artifacts::drop_host_internal_changes(
        &prepared.coordinator_plan,
        &mut result,
    );
    // ONE grant for all three ownership gates, resolved from the worktree's
    // actual changes and the settled envelope before the first of them runs.
    // Gate 1 replaces the envelope on rejection, so a grant resolved any later
    // would read an empty one.
    let grant = super::worktree_scope_grant::ScopeGrant::resolve(
        &prepared.coordinator_plan,
        &result,
        Some(prepared.wave_claims.as_slice()),
        &forbidden,
    );
    // Issue-13: formatter noise outside the declared targets is restored in
    // the worktree NOW, before anything reads it — the `patch_landed` answer
    // below, gate 2 at capture — so the branch's real work is judged alone.
    let whitespace_dropped = grant.drop_whitespace_only_changes();
    // Issue-27: a real change outside the plan's scope roots is dropped the
    // same way, before the same readers, so it is never granted or declared.
    let out_of_scope_dropped = grant.drop_out_of_scope_changes();
    // Answered against the declared baseline BEFORE validation, because both
    // `validate_worktree_branch_result` and `capture_worktree_branch_manifest`
    // replace `*result` wholesale on rejection — an ownership or size-policy
    // rejection would otherwise discard the very marker that records it landed
    // nothing. The verdict is captured here and stamped last, so it survives
    // whichever result object comes out the far end.
    let mut landed = worktree_patch_landed(&prepared, &grant);
    // Issue-30: a forbidden path was changed. Rejected HERE, before gate 1
    // reads the envelope and before capture reads the worktree: the result
    // is replaced wholesale, so nothing below captures a manifest, and
    // `landed` is answered false because nothing will land. The worktree is
    // left as the coder left it — the wave's partial-work capture keeps it,
    // with this verdict as its origin, for the next attempt to undo.
    if !grant.forbidden.is_empty() {
        let rejection = super::forbidden_paths::forbidden_rejection_result(
            &branch.id,
            &task_ids,
            &grant.forbidden,
        );
        persist_rejected_worktree_result(
            ctx.v2_store,
            &branch.id,
            "forbidden_path_changed",
            &result,
            &rejection.summary,
        );
        result = rejection;
        landed = PatchLanding::default();
    }
    let schema_repair_failed = is_schema_repair_failure_result(&result);
    validate_worktree_branch_result(
        &mut result,
        &branch,
        &prepared.assignment,
        &grant,
        ctx.v2_store,
        ctx.canonical_root.to_str(),
    )?;
    // Issue-69: the declared project artifacts whose digest changed while the
    // agent ran, answered from the capture above BEFORE the gate reads the
    // patch. Live on wf-0ddadd81 agents-6-0: the coverage artifact was
    // regenerated, the repository patch was empty by construction, and the
    // branch was refused as an empty patch with the receipt stamped below.
    let delivered_artifacts = delivery.changed_paths();
    let (mut manifest, pre_hashes) = capture_worktree_branch_manifest(
        &ctx,
        &mut result,
        &prepared,
        &grant,
        delivered_artifacts,
    )?;
    // After the gates, whatever they decided: a rejection replaces the result
    // wholesale, and the dropped paths must be visible on that one too.
    report_whitespace_only_drops(&mut result, &branch.id, &whitespace_dropped);
    report_out_of_scope_drops(
        &mut result,
        &branch.id,
        &out_of_scope_dropped,
        &grant.roots.describe(),
    );
    super::host_internal_artifacts::report_host_internal_drops(
        &mut result,
        &branch.id,
        &host_internal_dropped,
    );
    report_underreported_changes(&mut result, &branch.id, &grant.unreported);
    super::forbidden_paths::report_forbidden_declared_conflict(
        &mut result,
        &branch.id,
        &grant.forbidden_declared,
    );
    mark_patch_landed(
        &mut result,
        &prepared.branch.id,
        landed,
        schema_repair_failed,
    );
    super::focused_test_targets::stamp_result(&mut result, &prepared.focused_test_targets);
    delivery.stamp(&mut result, landed.any());
    // Judged against the same grant as the three ownership gates above: the
    // declared list is only what the preamble showed the agent (Issue-15).
    super::super::audit_gate::enforce(
        ctx.v2_store,
        &prepared.assignment.owned_targets,
        &grant,
        &branch.workspace_root,
        &mut result,
        &mut manifest,
    )?;
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
