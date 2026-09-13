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
        input_hash: Some(prepared.branch.input_hash()),
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

/// Record that a schema-repair failure nonetheless left a real patch on disk.
///
/// A write branch whose schema repair failed produced NO verdict on the work —
/// but the work may still have landed. Two of TDL-020's three attempts died
/// exactly this way, and charging them to the task discarded a patch that
/// existed. This is the third shape of "an attempt burned by something that
/// says nothing about the work", after the HTTP 520 and the verifier timeout.
///
/// The question is answered against the DECLARED BASELINE, never by asking the
/// worktree whether any files changed. Stray tool output, a partial write, or a
/// worktree dirtied by something other than the patch all answer "yes" to the
/// cheap question, and each would refund an attempt that produced nothing.
///
/// **Marking only.** The budget decision lives in the prelude's
/// `remediationBudget`, bounded to once per task. That bound is the safety
/// argument: schema repair already retries under its own cap, so an unbounded
/// exemption trades a burned attempt for a hung task — strictly worse.
///
/// # What this does NOT do
///
/// It does not preserve the patch. A schema failure classifies as `Contract`,
/// which yields `NeedsReview`, and `capture_worktree_branch_manifest` captures
/// only `Accepted`/`Noop` — so the patch is never turned into a manifest and
/// never reaches the canonical repo. It is stranded in the branch worktree and
/// discarded with it.
///
/// **The refunded attempt therefore starts clean and redoes the work.** Seeing a
/// task visibly repeat itself on this path is expected, not a bug.
///
/// So this buys a retry, not a rescue: it stops a malformed *report* from
/// spending the task's budget. The spec's "re-verify the existing patch rather
/// than re-running the round" is not achievable here — there is no surviving
/// patch to re-verify. Making that true would mean capturing a manifest from a
/// non-accepted branch, which touches the write coordinator's safety model and
/// is deliberately out of scope.
/// Did this branch leave real work on disk, measured against the DECLARED
/// BASELINE?
///
/// Never asks the worktree whether any files changed. Stray tool output, a
/// partial write, or a worktree dirtied by something other than the patch all
/// answer "yes" to the cheap question. Fails CLOSED: if the patch cannot be
/// captured we cannot prove work landed, so the answer is `false`.
pub(super) fn worktree_patch_landed(prepared: &PreparedWorktreeBranch) -> bool {
    capture_patch(
        &prepared.workspace,
        &prepared.coordinator_plan.target_files,
        &prepared.baseline,
    )
    .is_ok_and(|captured| !captured.changed_files.is_empty() || !captured.created_files.is_empty())
}

/// Record on EVERY write branch whether a patch landed.
///
/// `patch_landed` is the general predicate: it is set for accepted, rejected
/// and failed branches alike, so a consumer can ask "did this call change
/// anything?" without having to infer it from a status that answers a different
/// question. Three rejection paths that all land nothing — schema-repair
/// exhaustion, a wholesale size-policy rejection, and an ownership violation —
/// are indistinguishable by status but identical here.
///
/// Its first consumer is the prelude's `remediateFindings`, which used to fire
/// a verifier unconditionally after every fix. Observed live on TDL-041: a fix
/// failed host validation at 09:09:55.153 and a verifier started against
/// unchanged code **85.8 ms later**, then returned the same findings. A status
/// check would not have caught it, and would also have waved through an
/// accepted no-op, which likewise leaves the reviewed code untouched.
///
/// `schema_repair_patch_landed` is kept as the narrower marker that
/// `remediationBudget` reads for its once-per-task attempt refund.
///
/// # Scope: worktree writes only
///
/// There are three write modes — `Serial`, `Coordinated`, `Worktree` — and this
/// is the worktree branch runner, so **coordinated and serial writes carry no
/// `patch_landed` marker**. That is total coverage for the only consumer today,
/// and deliberately so rather than by luck:
///
/// - every write the v3 prelude can request is `write: "worktree"` (both
///   `agent()` and `agents()`), which is the sole source of the remediation
///   fixes the gate exists to judge;
/// - the host never silently downgrades. `workflow_live_v2_write.rs`'s
///   `(_, false)` arm ERRORS when worktree isolation is unavailable instead of
///   falling back, so a worktree request cannot quietly become a serial one.
///
/// The prelude-side test `every_write_the_prelude_requests_is_a_worktree_write`
/// fails if that first premise ever stops holding. A consumer reading this
/// marker on a coordinated or serial branch will see it ABSENT, which
/// `landedNothing` deliberately reads as "run the check" — the old behaviour,
/// not a silent skip.
pub(super) fn mark_patch_landed(
    result: &mut WorkflowV2Result,
    prepared: &PreparedWorktreeBranch,
    landed: bool,
    schema_repair_failed: bool,
) {
    if let Some(data) = result.data.as_object_mut() {
        data.insert("patch_landed".to_string(), serde_json::Value::Bool(landed));
    }
    if !schema_repair_failed || !landed {
        return;
    }
    if let Some(data) = result.data.as_object_mut() {
        data.insert(
            "schema_repair_patch_landed".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    // Typed gap so "was exempted" and "used the exemption" stay separable in the
    // records rather than having to be inferred from attempt counts later.
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "schema_repair_exempted_{}",
            sanitize_v2_path_segment(&prepared.branch.id)
        ),
        description: format!(
            "schema repair failed for branch '{}', but a patch landed against the declared \
             baseline, so the attempt did real work and produced no verdict. The patch is NOT \
             preserved (a NeedsReview branch is never captured), so the refunded attempt redoes \
             the work from a clean worktree. Refunded ONCE for this task — a second such failure \
             is charged normally.",
            prepared.branch.id,
        ),
        severity: Some("info".to_string()),
    });
}

/// Keyed on the runtime's own error text for the bounded-retry exhaustion, which
/// is the only place this phrasing is produced (`write_errors.rs:213`).
pub(super) fn is_schema_repair_failure_result(result: &WorkflowV2Result) -> bool {
    result
        .data
        .get("error")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|error| error.contains("schema repair failed"))
}

pub(super) fn validate_worktree_branch_result(
    result: &mut WorkflowV2Result,
    branch: &WorktreeBranchExecution,
    assignment: &WorkflowV2WriteAssignment,
    v2_store: &WorkflowV2ResultStore,
    canonical_root: Option<&str>,
) -> crate::WorkflowResult<()> {
    let mut item = WorkflowV2WriteItem::new(
        branch.execution.call.id.clone(),
        WorkflowV2WriteMode::Worktree,
        assignment.owned_targets.clone(),
    )
    .with_owned_scopes(assignment.owned_scopes.clone());
    item.artifact_only = assignment.artifact_only;
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
        persist_rejected_worktree_result(
            v2_store,
            &branch.id,
            "ownership_validation",
            result,
            &err.to_string(),
        );
        if is_write_branch_validation_error(&err.to_string()) {
            *result = write_branch_validation_error_result(
                &branch.id,
                Some(&branch.execution.input),
                &err.to_string(),
            );
        } else {
            return Err(WorkflowError::SpecInvalid(err.to_string()));
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

pub(super) fn capture_worktree_branch_manifest(
    run_root: &Path,
    run_id: &str,
    execution: &WorkflowV2CallExecution,
    cfg: &WriteCoordinatorConfig,
    v2_store: &WorkflowV2ResultStore,
    result: &mut WorkflowV2Result,
    prepared: &PreparedWorktreeBranch,
) -> crate::WorkflowResult<CapturedWorktreeManifest> {
    if !matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        return Ok((None, None));
    }
    let branch_id = prepared.branch.id.as_str();
    let captured = match capture_and_validate_worktree_patch(
        &prepared.workspace,
        &prepared.coordinator_plan,
        &prepared.baseline,
        cfg,
        result,
        Some(prepared.wave_claims.as_slice()),
    ) {
        Ok(captured) => captured,
        Err(err) => {
            persist_rejected_worktree_result(
                v2_store,
                branch_id,
                "patch_validation",
                result,
                &err.to_string(),
            );
            if is_write_branch_validation_error(&err.to_string()) {
                *result = write_branch_validation_error_result(
                    branch_id,
                    Some(&prepared.branch.input),
                    &err.to_string(),
                );
                return Ok((None, None));
            }
            return Err(err);
        }
    };
    let manifest = persist_worktree_manifest(run_root, run_id, execution, branch_id, &captured)?;
    push_patch_manifest_artifact(result, run_root, &execution.call.id, branch_id);
    super::worktree_branch_b::report_ignored_deliverables(result, &manifest);
    Ok((Some(manifest), Some(captured.pre_hashes)))
}

#[cfg(test)]
#[path = "read_set_retry_tests.rs"]
mod read_set_retry_tests;

#[cfg(test)]
#[path = "restart_refresh_tests.rs"]
mod restart_refresh_tests;
