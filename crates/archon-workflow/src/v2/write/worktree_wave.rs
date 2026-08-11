use super::*;

#[derive(Default)]
pub(super) struct WorktreeWaveArtifacts {
    pub(super) results: Vec<WorkflowV2Result>,
    pub(super) manifests: Vec<PatchManifest>,
    pub(super) pre_hashes: BTreeMap<String, BTreeMap<String, String>>,
    pub(super) completed: Vec<CompletedWorktreeBranch>,
    pub(super) apply_gap: Option<String>,
}

#[derive(Default)]
pub(super) struct WorktreePlanArtifacts {
    pub(super) results: Vec<WorkflowV2Result>,
    pub(super) manifests: Vec<PatchManifest>,
    pub(super) apply_gap: Option<String>,
    pub(super) peak_parallelism: usize,
}

pub(super) struct WorktreePlanRunContext<'a> {
    pub(super) task: &'a str,
    pub(super) target_repository_root: Option<&'a str>,
    pub(super) execution: &'a WorkflowV2CallExecution,
    pub(super) adapter: WorkflowV2AgentAdapter,
    pub(super) dispatch: &'a dyn WorkflowAgentDispatch,
    pub(super) v2_store: &'a WorkflowV2ResultStore,
    pub(super) store_for_control: &'a crate::WorkflowStore,
    pub(super) run_id: &'a str,
    pub(super) setup: &'a WorktreeFanoutSetup,
    pub(super) semaphore: Arc<Semaphore>,
    pub(super) active: Arc<AtomicUsize>,
    pub(super) peak: Arc<AtomicUsize>,
}

pub(super) fn worktree_plan_context<'a>(
    ctx: &WriteFanoutContext<'a>,
    setup: &'a WorktreeFanoutSetup,
) -> WorktreePlanRunContext<'a> {
    let max_parallelism = ctx
        .dispatch
        .fanout_parallelism(ctx.execution.call.options.max_parallelism);
    WorktreePlanRunContext {
        task: ctx.task,
        target_repository_root: ctx.target_repository_root,
        execution: ctx.execution,
        adapter: ctx.adapter.clone(),
        dispatch: ctx.dispatch,
        v2_store: ctx.v2_store,
        store_for_control: ctx.store_for_control,
        run_id: ctx.run_id,
        setup,
        semaphore: Arc::new(Semaphore::new(max_parallelism)),
        active: Arc::new(AtomicUsize::new(0)),
        peak: Arc::new(AtomicUsize::new(0)),
    }
}

pub(super) async fn run_worktree_plan_waves(
    ctx: WorktreePlanRunContext<'_>,
    plan: &WorkflowV2WritePlan,
    branches: &[crate::WorkflowV2FanoutItem],
) -> crate::WorkflowResult<WorktreePlanArtifacts> {
    let mut output = WorktreePlanArtifacts::default();
    for (wave_index, wave) in plan.waves.iter().enumerate() {
        let artifacts = run_one_worktree_wave(&ctx, branches, wave_index, wave).await?;
        let wave_gap = artifacts.apply_gap.clone();
        output.results.extend(artifacts.results);
        output.manifests.extend(artifacts.manifests);
        if let Some(reason) = wave_gap {
            output.apply_gap = Some(reason);
            break;
        }
    }
    output.peak_parallelism = ctx.peak.load(Ordering::SeqCst);
    Ok(output)
}

pub(super) async fn run_one_worktree_wave(
    ctx: &WorktreePlanRunContext<'_>,
    branches: &[crate::WorkflowV2FanoutItem],
    wave_index: usize,
    wave: &WorkflowV2WriteWave,
) -> crate::WorkflowResult<WorktreeWaveArtifacts> {
    let prepared = prepare_worktree_wave(
        wave,
        branches,
        ctx.run_id,
        &ctx.execution.call.id,
        &ctx.setup.canonical_root,
        &ctx.setup.cfg,
        ctx.store_for_control,
    )?;
    let completed = run_prepared_worktree_wave(ctx.wave_context(), prepared).await?;
    let mut artifacts =
        collect_worktree_wave_artifacts(completed, ctx.v2_store, &ctx.execution.call.id)?;
    artifacts.apply_gap = apply_worktree_wave(ctx, wave_index, &artifacts);
    cleanup_completed_worktree_wave(
        &ctx.setup.canonical_root,
        &ctx.setup.cfg,
        &artifacts.completed,
        artifacts.apply_gap.as_deref(),
    );
    Ok(artifacts)
}

pub(super) fn prepare_worktree_wave(
    wave: &WorkflowV2WriteWave,
    branches: &[crate::WorkflowV2FanoutItem],
    run_id: &str,
    call_id: &str,
    canonical_root: &Path,
    cfg: &WriteCoordinatorConfig,
    store_for_control: &crate::WorkflowStore,
) -> crate::WorkflowResult<Vec<PreparedWorktreeBranch>> {
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
        prepared.push(PreparedWorktreeBranch {
            branch,
            assignment: assignment.clone(),
            coordinator_plan,
            baseline,
            workspace,
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

pub(super) async fn run_prepared_worktree_wave(
    ctx: WorktreeWaveRunContext<'_>,
    prepared: Vec<PreparedWorktreeBranch>,
) -> crate::WorkflowResult<Vec<CompletedWorktreeBranch>> {
    // Captured BEFORE the jobs consume `prepared`: a branch that returns `Err`
    // hands back nothing, so without this its identity is gone and the only
    // thing left to do with the error is throw the whole wave away.
    let identities: Vec<WorktreeBranchIdentity> =
        prepared.iter().map(worktree_branch_identity).collect();
    let jobs = prepared
        .into_iter()
        .map(|prepared| worktree_branch_job(ctx.clone(), prepared));
    worktree_wave_outcomes(identities, futures_util::future::join_all(jobs).await)
}

/// What one prepared branch is, independent of whether its job succeeded.
///
/// Exactly the fields [`CompletedWorktreeBranch`] needs to exist without a
/// result — everything else a completed branch carries (manifest, pre-hashes)
/// is produced BY the run and is legitimately absent when the run errored.
pub(super) struct WorktreeBranchIdentity {
    pub(super) item_id: String,
    pub(super) role: String,
    pub(super) item_input_hash: Option<String>,
    pub(super) workspace_root: PathBuf,
    pub(super) input: serde_json::Value,
}

pub(super) fn worktree_branch_identity(
    prepared: &PreparedWorktreeBranch,
) -> WorktreeBranchIdentity {
    WorktreeBranchIdentity {
        item_id: prepared.branch.id.clone(),
        role: prepared.branch.role.clone(),
        // Same expression `prepare_worktree_branch_execution` stamps, so a
        // branch that errored records the identity a successful one would have.
        item_input_hash: Some(prepared.branch.input_hash()),
        workspace_root: prepared.workspace.plan.isolated_root.clone(),
        input: prepared.branch.input.clone(),
    }
}

impl WorktreeBranchIdentity {
    fn into_failed_branch(self, error: &str) -> CompletedWorktreeBranch {
        let result = write_branch_unhandled_error_result(&self.item_id, Some(&self.input), error);
        CompletedWorktreeBranch {
            item_id: self.item_id,
            role: self.role,
            item_input_hash: self.item_input_hash,
            result,
            // No manifest and no pre-hashes: the branch never produced a
            // capture, so it contributes nothing to `apply_wave`. Its failure is
            // data; it is not work to merge.
            manifest: None,
            pre_hashes: None,
            workspace_root: self.workspace_root,
        }
    }
}

/// Turn each branch's `Result` into a branch outcome, so ONE branch's error
/// cannot discard the finished work of its siblings.
///
/// The collection used to be `completed.push(item?)`, which meant a single
/// unrecognised `Err` returned before [`collect_worktree_wave_artifacts`] ever
/// ran — and that is the only place `save_write_branch_outcome` is called. Every
/// sibling that had ALREADY FINISHED was therefore dropped without ever being
/// written to `v2/branches/<call_id>/`.
///
/// Fatal errors still unwind, and the first one in wave order wins. When they
/// do, branch results accumulated so far are discarded exactly as before: a
/// paused, cancelled or host-broken run is not a run whose partial wave should
/// be persisted as findings.
pub(super) fn worktree_wave_outcomes(
    identities: Vec<WorktreeBranchIdentity>,
    outcomes: Vec<crate::WorkflowResult<CompletedWorktreeBranch>>,
) -> crate::WorkflowResult<Vec<CompletedWorktreeBranch>> {
    let mut completed = Vec::new();
    for (identity, outcome) in identities.into_iter().zip(outcomes) {
        match outcome {
            Ok(branch) => completed.push(branch),
            Err(error) if is_fatal_worktree_wave_error(&error) => return Err(error),
            Err(error) => completed.push(identity.into_failed_branch(&error.to_string())),
        }
    }
    Ok(completed)
}

/// The CLOSED list of errors that must still unwind the whole wave.
///
/// Deliberately an explicit allow-list of fatals rather than a broad "anything
/// unrecognised is branch data": every entry below is a statement about the RUN
/// or about the HOST, never about the work one branch did, and demoting one to
/// per-branch review data is the #153 failure mode — a subsystem that reports
/// healthy while it is broken.
///
/// - [`WorkflowError::ControlPaused`] / [`WorkflowError::ControlCancelled`] —
///   raised by `poll_v2_run_control`, which every branch polls on entry
///   (`worktree_branch_job`), again in `prepare_worktree_branch_execution`, and
///   again after its agent returns. They are the operator's verdict on the run,
///   not a branch result. Recording one as branch data would let a paused or
///   cancelled run finish dispatching its wave and then report as "needs
///   review".
/// - [`WorkflowError::NotificationDelivery`] — a delivery the run declared
///   REQUIRED did not happen. The entire point of "required" is that the run
///   must not continue unobserved; folding it into one branch's findings is
///   precisely the silence the error exists to prevent.
/// - [`WorkflowError::SpecInvalid`] — the deliberate "this is a host bug, not
///   branch data" signal. `validate_worktree_branch_result` raises it exactly
///   when an ownership rejection is NOT in `is_write_branch_validation_error`'s
///   allow-list, and `branch_for_assignment` raises it when the write plan
///   references a fan-out item that does not exist. Both mean the host or the
///   plan is wrong, and neither is remediable by re-running the branch.
///
/// Everything else describes one branch's own attempt — `StageFailed` from
/// workspace or baseline setup, `Io` while persisting that branch's manifest,
/// `Json`, a transport `Port` error — and is scoped to that branch so its
/// siblings' finished work still reaches disk.
pub(super) fn is_fatal_worktree_wave_error(error: &WorkflowError) -> bool {
    matches!(
        error,
        WorkflowError::ControlPaused(_)
            | WorkflowError::ControlCancelled(_)
            | WorkflowError::NotificationDelivery(_)
            | WorkflowError::SpecInvalid(_)
    )
}

#[derive(Clone)]
pub(super) struct WorktreeWaveRunContext<'a> {
    pub(super) task: &'a str,
    pub(super) target_repository_root: Option<&'a str>,
    pub(super) execution: &'a WorkflowV2CallExecution,
    pub(super) adapter: WorkflowV2AgentAdapter,
    pub(super) dispatch: &'a dyn WorkflowAgentDispatch,
    pub(super) v2_store: &'a WorkflowV2ResultStore,
    pub(super) store_for_control: &'a crate::WorkflowStore,
    pub(super) run_id: &'a str,
    pub(super) run_root: &'a Path,
    pub(super) canonical_root: &'a Path,
    pub(super) cfg: &'a WriteCoordinatorConfig,
    pub(super) semaphore: Arc<Semaphore>,
    pub(super) active: Arc<AtomicUsize>,
    pub(super) peak: Arc<AtomicUsize>,
}

impl WorktreePlanRunContext<'_> {
    pub(super) fn wave_context(&self) -> WorktreeWaveRunContext<'_> {
        WorktreeWaveRunContext {
            task: self.task,
            target_repository_root: self.target_repository_root,
            execution: self.execution,
            adapter: self.adapter.clone(),
            dispatch: self.dispatch,
            v2_store: self.v2_store,
            store_for_control: self.store_for_control,
            run_id: self.run_id,
            run_root: &self.setup.run_root,
            canonical_root: &self.setup.canonical_root,
            cfg: &self.setup.cfg,
            semaphore: self.semaphore.clone(),
            active: self.active.clone(),
            peak: self.peak.clone(),
        }
    }
}

pub(super) async fn worktree_branch_job(
    ctx: WorktreeWaveRunContext<'_>,
    prepared: PreparedWorktreeBranch,
) -> crate::WorkflowResult<CompletedWorktreeBranch> {
    let _permit = ctx
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
    poll_v2_run_control(ctx.store_for_control, ctx.run_id, &prepared.branch.id)?;
    let now_active = ctx.active.fetch_add(1, Ordering::SeqCst) + 1;
    record_write_peak(&ctx.peak, now_active);
    // Same `Arc`, held across the move of `ctx` into the branch runner so the
    // active-count decrement below still lands on the shared counter.
    let active = ctx.active.clone();
    let result = run_one_worktree_branch(ctx, prepared).await;
    active.fetch_sub(1, Ordering::SeqCst);
    result
}

pub(super) fn collect_worktree_wave_artifacts(
    completed: Vec<CompletedWorktreeBranch>,
    v2_store: &WorkflowV2ResultStore,
    call_id: &str,
) -> crate::WorkflowResult<WorktreeWaveArtifacts> {
    let mut artifacts = WorktreeWaveArtifacts::default();
    for completed_branch in completed {
        let mut result = completed_branch.result.clone();
        tag_branch_result(&mut result, &completed_branch.item_id);
        normalize_write_branch_contract_result(&mut result);
        save_write_branch_outcome(
            v2_store,
            call_id,
            &completed_branch.item_id,
            &completed_branch.role,
            completed_branch.item_input_hash.clone(),
            &result,
        )?;
        artifacts.results.push(result);
        push_worktree_manifest_artifacts(&mut artifacts, &completed_branch);
        artifacts.completed.push(completed_branch);
    }
    Ok(artifacts)
}

pub(super) fn push_worktree_manifest_artifacts(
    artifacts: &mut WorktreeWaveArtifacts,
    completed_branch: &CompletedWorktreeBranch,
) {
    if let Some(manifest) = &completed_branch.manifest {
        artifacts.manifests.push(manifest.clone());
    }
    if let Some(pre_hashes) = &completed_branch.pre_hashes {
        artifacts
            .pre_hashes
            .insert(completed_branch.item_id.clone(), pre_hashes.clone());
    }
}

pub(super) fn apply_worktree_wave(
    ctx: &WorktreePlanRunContext<'_>,
    wave_index: usize,
    artifacts: &WorktreeWaveArtifacts,
) -> Option<String> {
    if artifacts.manifests.is_empty() {
        return None;
    }
    let apply_result = with_repo_lock(&ctx.setup.canonical_root, || {
        apply_wave(
            &ctx.setup.canonical_root,
            &artifacts.manifests,
            &artifacts.pre_hashes,
            wave_index as u32,
            &ctx.setup.run_root,
            ctx.run_id,
            &ctx.execution.call.id,
        )
    });
    worktree_apply_gap(apply_result)
}

pub(super) fn worktree_apply_gap(
    result: Result<crate::write_coordinator::ApplyRecord, impl std::fmt::Display>,
) -> Option<String> {
    match result {
        Ok(record) if !record.items_failed.is_empty() => Some(format!(
            "worktree patch apply left {} item(s) unapplied: {}",
            record.items_failed.len(),
            record
                .items_failed
                .iter()
                .map(|(item, reason)| format!("{item}: {reason}"))
                .collect::<Vec<_>>()
                .join("; ")
        )),
        Ok(_) => None,
        Err(err) => Some(format!("worktree patch apply failed: {err}")),
    }
}

pub(super) fn cleanup_completed_worktree_wave(
    canonical_root: &Path,
    cfg: &WriteCoordinatorConfig,
    completed: &[CompletedWorktreeBranch],
    apply_gap: Option<&str>,
) {
    let status = if apply_gap.is_none() {
        WorkspaceStatus::Succeeded
    } else {
        WorkspaceStatus::Failed
    };
    for completed_branch in completed {
        let _ = cleanup_workspace(
            canonical_root,
            &completed_branch.workspace_root,
            status,
            cfg,
        );
    }
}
