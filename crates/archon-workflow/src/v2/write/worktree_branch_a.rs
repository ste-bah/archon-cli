use super::*;

pub(super) struct WorktreeBranchExecution {
    pub(super) id: String,
    pub(super) role: String,
    pub(super) input_hash: Option<String>,
    pub(super) workspace_root: PathBuf,
    pub(super) execution: WorkflowV2CallExecution,
    /// Set for a write branch whose task carries the host preamble, so a
    /// fresh session started mid-attempt is told what its worktree holds.
    pub(super) refresh: Option<super::partial_work::BranchTaskRefresh>,
    /// Wall clock this execution may spend across every re-dispatch, when it
    /// is not the dispatcher's call budget. The in-run timeout retry sets it
    /// to the retry budget: its dispatch timeout is pinned to that budget, and
    /// a re-ask loop bounded by the first session's hours instead would let a
    /// transport drop inside the retry re-dispatch long past it.
    pub(super) time_budget: BranchTimeBudget,
}

/// The wall-clock bound a branch's re-ask loop runs under.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum BranchTimeBudget {
    /// The dispatcher's total call budget (`call_time_budget`).
    #[default]
    CallTimeBudget,
    /// A budget fixed by the caller; `None` is unbounded by time.
    Fixed(Option<std::time::Duration>),
}

impl BranchTimeBudget {
    pub(super) fn resolve(
        self,
        dispatch: &dyn WorkflowAgentDispatch,
    ) -> Option<std::time::Duration> {
        match self {
            Self::CallTimeBudget => dispatch.call_time_budget(),
            Self::Fixed(budget) => budget,
        }
    }
}

pub(super) type CapturedWorktreeManifest =
    (Option<PatchManifest>, Option<BTreeMap<String, String>>);

pub(super) fn prepare_worktree_branch_execution(
    execution: &WorkflowV2CallExecution,
    store_for_control: &crate::WorkflowStore,
    run_id: &str,
    prepared: &PreparedWorktreeBranch,
) -> crate::WorkflowResult<WorktreeBranchExecution> {
    let id = prepared.branch.id.clone();
    poll_v2_run_control(store_for_control, run_id, &id)?;
    let mut call = prepared.branch.call.clone();
    call.options.target_files = prepared.assignment.owned_targets.clone();
    call.options.extra.insert(
        "target_ownership_scopes".to_string(),
        serde_json::to_value(&prepared.assignment.owned_scopes)?,
    );
    call.options.extra.insert(
        "wave_claims".to_string(),
        serde_json::to_value(&prepared.wave_claims)?,
    );
    Ok(WorktreeBranchExecution {
        id,
        role: prepared.branch.role.clone(),
        input_hash: Some(reuse_identity(&prepared.branch)),
        workspace_root: prepared.workspace.plan.isolated_root.clone(),
        execution: WorkflowV2CallExecution {
            call,
            input: prepared.branch.input.clone(),
            depends_on: vec![execution.call.id.clone()],
        },
        refresh: None,
        time_budget: BranchTimeBudget::CallTimeBudget,
    })
}

pub(super) fn completed_worktree_branch(
    branch: WorktreeBranchExecution,
    result: WorkflowV2Result,
    manifest: Option<PatchManifest>,
    pre_hashes: Option<BTreeMap<String, String>>,
) -> CompletedWorktreeBranch {
    CompletedWorktreeBranch {
        item_id: branch.id,
        role: branch.role,
        item_input_hash: branch.input_hash,
        result,
        manifest,
        pre_hashes,
        workspace_root: branch.workspace_root,
    }
}

pub(super) async fn run_worktree_branch_agent(
    task: &str,
    target_repository_root: Option<String>,
    dispatch: &dyn WorkflowAgentDispatch,
    v2_store: &WorkflowV2ResultStore,
    adapter: WorkflowV2AgentAdapter,
    branch: &WorktreeBranchExecution,
    task_universe: Option<&crate::task_universe::WorkflowV2TaskUniverse>,
) -> crate::WorkflowResult<WorkflowV2Result> {
    // The worktree branch runs against its own sealed workspace, which takes
    // precedence over the run's target repository root — the same `or`
    // precedence the two-parameter host function applied.
    let repository_root =
        Some(branch.workspace_root.display().to_string()).or(target_repository_root);
    // A wholesale line-cap rejection discards the branch's ENTIRE patch and
    // says exactly how to avoid it. That is a correctable instruction, not a
    // verdict on the work, so it is fed back and the branch re-asked for as
    // long as it keeps getting closer to the cap.
    let original_prompt = branch.execution.call.options.task.as_deref().unwrap_or(task);
    // `base` is the task as the next session should see it; a fresh session
    // after a transport drop gets it re-rendered against the worktree as it
    // stands. `size_notice` is the standing rejection it is re-asked under.
    let mut base = original_prompt.to_string();
    let mut size_notice: Option<String> = None;
    let mut prompt = base.clone();
    let mut previous_overshoot: Option<u32> = None;
    // Transport failures are counted separately: a dropped provider connection
    // is not an answer about the work, so it must not consume the budget that
    // exists for correcting a rejection.
    let mut transport_failures = 0usize;
    let mut size_retries = 0usize;
    let started = std::time::Instant::now();
    let time_budget = branch.time_budget.resolve(dispatch);
    for _ in 0..super::size_retry::MAX_BRANCH_DISPATCHES {
        if super::size_retry::call_time_budget_exhausted(started, time_budget) {
            let err = super::size_retry::call_time_budget_error(&branch.id, started, time_budget);
            return normalize_worktree_agent_result(Err(err), &branch.id, &branch.execution.input);
        }
        let dispatch_prompt = crate::v2::write_read_set::with_current_preamble(
            &prompt, v2_store, &branch.execution.call.id,
        );
        let mut execution = branch.execution.clone();
        execution.call.options.task = Some(dispatch_prompt.clone());
        let result = dispatch
            .run_call(
                &dispatch_prompt,
                repository_root.clone(),
                &execution,
                &adapter,
                Some(v2_store),
                task_universe,
            )
            .await;
        let Err(err) = &result else {
            return normalize_worktree_agent_result(result, &branch.id, &branch.execution.input);
        };
        // The host's own timer ended the session. That is the budget the
        // host chose for this dispatch, not a provider failure, so the loop
        // does not re-ask: the interrupted result goes up to the caller,
        // whose retry-once / stall logic is the only thing allowed to decide.
        // Without this the retry's 1800 s cut re-entered the transport path
        // and started a third session with an identical prompt (Issue-10).
        if err.is_host_call_timeout() {
            return normalize_worktree_agent_result(result, &branch.id, &branch.execution.input);
        }
        let text = err.to_string();
        // The provider dropped the call. Nothing landed and no verdict was
        // produced, so re-ask rather than ending the branch and, with it, the
        // wave — two runs died this way in one morning on `response_failed`.
        if crate::v2::transport_retry::is_transport_failure(&text)
            && !crate::v2::transport_retry::is_content_rejection(&text)
        {
            if transport_failures >= crate::v2::transport_retry::MAX_TRANSPORT_RETRIES {
                return normalize_worktree_agent_result(
                    result,
                    &branch.id,
                    &branch.execution.input,
                );
            }
            transport_failures += 1;
            // The next dispatch is a fresh session in the same worktree. Its
            // task was rendered when the worktree was clean, so re-render it:
            // the files this attempt already wrote, the recorded read set, and
            // the wall clock this session actually has.
            if let Some(refresh) = &branch.refresh {
                base = refresh.restarted_task(
                    v2_store,
                    &branch.workspace_root,
                    &branch.execution.call.id,
                    dispatch.resume_memory_calls(),
                    super::partial_work::effective_call_budget(
                        dispatch.dispatch_timeout(),
                        time_budget,
                        started.elapsed(),
                    ),
                );
                prompt = match &size_notice {
                    Some(notice) => format!("{base}\n\n{notice}"),
                    None => base.clone(),
                };
            }
            continue;
        }
        if !super::size_retry::is_line_cap_rejection(&text) {
            return normalize_worktree_agent_result(result, &branch.id, &branch.execution.input);
        }
        // Counted here rather than by the loop, so a dropped transport cannot
        // spend an attempt that exists for correcting a rejection.
        if size_retries >= super::size_retry::MAX_SIZE_RETRIES {
            let outcome =
                normalize_worktree_agent_result(result, &branch.id, &branch.execution.input);
            return stamp_no_progress(outcome, size_retries);
        }
        let overshoot = super::size_retry::rejected_line_count(&text);
        if !super::size_retry::should_retry(previous_overshoot, overshoot) {
            let outcome =
                normalize_worktree_agent_result(result, &branch.id, &branch.execution.input);
            return stamp_no_progress(outcome, size_retries);
        }
        size_retries += 1;
        previous_overshoot = overshoot;
        let notice = super::size_retry::retry_notice(&text);
        prompt = format!("{base}\n\n{notice}");
        size_notice = Some(notice);
    }
    let exhausted = normalize_worktree_agent_result(
        Err(crate::WorkflowError::port(format!(
            "write branch '{}' could not fit its patch under the source-file line cap",
            branch.id
        ))),
        &branch.id,
        &branch.execution.input,
    );
    stamp_no_progress(exhausted, size_retries)
}

/// Gate 1: judge the envelope against the SAME plan capture will use.
///
/// The item is built from `grant.plan` — the coordinator plan widened by the
/// unclaimed paths this branch changed — never from the declared targets
/// alone. Issue-11: built from `assignment.owned_targets`, this gate refused an
/// unclaimed path before capture ever saw it, replaced the envelope with an
/// empty one, and the grant at capture had nothing left to widen for. A
/// contested path is not in the widened plan and is refused here exactly as
/// before. A whitespace-only one is not judged at all (Issue-13), nor is an
/// out-of-scope one (Issue-27): their entries are dropped from
/// `files_changed`, matching the worktree, where the files have already been
/// restored to the baseline or removed.
pub(super) fn validate_worktree_branch_result(
    result: &mut WorkflowV2Result,
    branch: &WorktreeBranchExecution,
    assignment: &WorkflowV2WriteAssignment,
    grant: &super::worktree_scope_grant::ScopeGrant,
    v2_store: &WorkflowV2ResultStore,
    canonical_root: Option<&str>,
) -> crate::WorkflowResult<()> {
    let mut item = WorkflowV2WriteItem::new(
        branch.execution.call.id.clone(),
        WorkflowV2WriteMode::Worktree,
        grant
            .plan
            .target_files
            .iter()
            .map(|path| path.as_str().to_string())
            .collect(),
    )
    .with_owned_scopes(
        grant
            .plan
            .target_dir_scopes
            .iter()
            .map(|path| path.as_str().to_string())
            .collect(),
    );
    item.artifact_only = assignment.artifact_only;
    result
        .files_changed
        .retain(|file| !grant.is_whitespace_only(&file.path) && !grant.is_out_of_scope(&file.path));
    let root = branch.workspace_root.display().to_string();
    // A branch works inside its own worktree, but an agent may report the file
    // it changed by the canonical project path instead -- the same file, named
    // from the other checkout. Stripping only the worktree root turned that
    // naming choice into a safety failure that killed the whole run.
    //
    // Ownership is still enforced: whichever root strips, the remaining
    // relative path must sit inside the item's declared targets, so a genuine
    // escape is still rejected.
    let outcome =
        validate_changed_files_for_repository(&item, result, Some(&root)).or_else(|err| {
            match canonical_root.filter(|canonical| *canonical != root) {
                Some(canonical) => {
                    validate_changed_files_for_repository(&item, result, Some(canonical))
                        .map_err(|_| err)
                }
                None => Err(err),
            }
        });
    if let Err(err) = outcome {
        let error = err.to_string();
        persist_rejected_worktree_result(
            v2_store,
            &branch.id,
            "ownership_validation",
            result,
            &error,
        );
        if is_write_branch_validation_error(&error) {
            *result = write_branch_validation_error_result(
                &branch.id,
                Some(&branch.execution.input),
                &error,
            );
        } else {
            return Err(WorkflowError::SpecInvalid(error));
        }
    }
    if let Err(error) = verify_declared_artifacts_for_result(
        &branch.execution.input,
        result,
        &branch.workspace_root,
    ) {
        persist_rejected_worktree_result(
            v2_store,
            &branch.id,
            "artifact_verification",
            result,
            &error,
        );
        *result =
            write_branch_validation_error_result(&branch.id, Some(&branch.execution.input), &error);
    }
    Ok(())
}

pub(crate) fn verify_declared_artifacts_for_result(
    input: &serde_json::Value,
    result: &WorkflowV2Result,
    workspace_root: &Path,
) -> Result<(), String> {
    if !result_requires_declared_artifact_verification(result) {
        return Ok(());
    }
    run_declared_artifact_verifiers(input, workspace_root)
}

pub(super) fn result_requires_declared_artifact_verification(result: &WorkflowV2Result) -> bool {
    matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) || result
        .data
        .get("idempotent_noop")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

pub(crate) fn run_declared_artifact_verifiers(
    input: &serde_json::Value,
    workspace_root: &Path,
) -> Result<(), String> {
    let commands = input
        .get("item")
        .and_then(|item| item.get("artifact_verification_commands"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty());
    for command in commands {
        let output = std::process::Command::new(archon_shell::resolve_posix_shell())
            .arg("-lc")
            .arg(command)
            .current_dir(workspace_root)
            .output()
            .map_err(|error| format!("artifact verifier could not start: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "declared artifact verifier failed with {}: {}{}",
                output.status,
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "read_set_retry_tests.rs"]
mod read_set_retry_tests;

#[cfg(test)]
#[path = "restart_refresh_tests.rs"]
mod restart_refresh_tests;
