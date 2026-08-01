async fn run_one_worktree_branch(
    task: &str,
    target_repository_root: Option<String>,
    execution: &WorkflowV2CallExecution,
    adapter: WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    store_for_control: &archon_workflow::WorkflowStore,
    run_id: &str,
    run_root: &Path,
    canonical_root: &Path,
    v2_store: &WorkflowV2ResultStore,
    cfg: &WriteCoordinatorConfig,
    prepared: PreparedWorktreeBranch,
) -> archon_workflow::WorkflowResult<CompletedWorktreeBranch> {
    let branch = prepare_worktree_branch_execution(execution, store_for_control, run_id, &prepared)?;
    let mut result = run_worktree_branch_agent(
        task,
        target_repository_root,
        client,
        v2_store,
        adapter,
        &branch,
    )
    .await
    ?;
    poll_v2_run_control(store_for_control, run_id, &branch.id)?;
    // Answered against the declared baseline BEFORE validation, because both
    // `validate_worktree_branch_result` and `capture_worktree_branch_manifest`
    // replace `*result` wholesale on rejection — an ownership or size-policy
    // rejection would otherwise discard the very marker that records it landed
    // nothing. The verdict is captured here and stamped last, so it survives
    // whichever result object comes out the far end.
    let landed = worktree_patch_landed(&prepared);
    let schema_repair_failed = is_schema_repair_failure_result(&result);
    validate_worktree_branch_result(&mut result, &branch, &prepared.assignment, v2_store)?;
    let (manifest, pre_hashes) =
        capture_worktree_branch_manifest(
            run_root,
            run_id,
            execution,
            cfg,
            v2_store,
            &mut result,
            &prepared,
        )?;
    mark_patch_landed(&mut result, &prepared, landed, schema_repair_failed);
    let _ = canonical_root;
    Ok(completed_worktree_branch(branch, result, manifest, pre_hashes))
}

struct WorktreeBranchExecution {
    id: String,
    role: String,
    input_hash: Option<String>,
    workspace_root: PathBuf,
    execution: WorkflowV2CallExecution,
}

type CapturedWorktreeManifest = (
    Option<PatchManifest>,
    Option<BTreeMap<String, String>>,
);

fn prepare_worktree_branch_execution(
    execution: &WorkflowV2CallExecution,
    store_for_control: &archon_workflow::WorkflowStore,
    run_id: &str,
    prepared: &PreparedWorktreeBranch,
) -> archon_workflow::WorkflowResult<WorktreeBranchExecution> {
    let id = prepared.branch.id.clone();
    poll_v2_run_control(store_for_control, run_id, &id)?;
    let mut call = prepared.branch.call.clone();
    call.options.target_files = prepared.assignment.owned_targets.clone();
    call.options.extra.insert(
        "target_ownership_scopes".to_string(),
        serde_json::to_value(&prepared.assignment.owned_scopes)?,
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
    })
}

fn completed_worktree_branch(
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

async fn run_worktree_branch_agent(
    task: &str,
    target_repository_root: Option<String>,
    client: &LiveV2AgentClient,
    v2_store: &WorkflowV2ResultStore,
    adapter: WorkflowV2AgentAdapter,
    branch: &WorktreeBranchExecution,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let result = run_single_v2_agent_call_in_repository(
        task,
        target_repository_root,
        &branch.execution,
        &adapter,
        client,
        Some(v2_store),
        None,
        Some(branch.workspace_root.display().to_string()),
    )
    .await;
    normalize_worktree_agent_result(result, branch)
}

fn normalize_worktree_agent_result(
    result: archon_workflow::WorkflowResult<WorkflowV2Result>,
    branch: &WorktreeBranchExecution,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    match result {
        Ok(result) => Ok(result),
        Err(err) if is_recoverable_write_branch_timeout(&err.to_string()) => Ok(
            write_branch_runtime_timeout_result(&branch.id, &branch.execution.input, &err.to_string()),
        ),
        Err(err) if is_write_branch_validation_error(&err.to_string()) => Ok(
            write_branch_validation_error_result(&branch.id, Some(&branch.execution.input), &err.to_string()),
        ),
        Err(err) => Err(err),
    }
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
fn worktree_patch_landed(prepared: &PreparedWorktreeBranch) -> bool {
    capture_patch(
        &prepared.workspace,
        &prepared.coordinator_plan.target_files,
        &prepared.baseline,
    )
    .is_ok_and(|captured| {
        !captured.changed_files.is_empty() || !captured.created_files.is_empty()
    })
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
fn mark_patch_landed(
    result: &mut WorkflowV2Result,
    prepared: &PreparedWorktreeBranch,
    landed: bool,
    schema_repair_failed: bool,
) {
    if let Some(data) = result.data.as_object_mut() {
        data.insert(
            "patch_landed".to_string(),
            serde_json::Value::Bool(landed),
        );
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
fn is_schema_repair_failure_result(result: &WorkflowV2Result) -> bool {
    result
        .data
        .get("error")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|error| error.contains("schema repair failed"))
}

fn validate_worktree_branch_result(
    result: &mut WorkflowV2Result,
    branch: &WorktreeBranchExecution,
    assignment: &WorkflowV2WriteAssignment,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<()> {
    let mut item = WorkflowV2WriteItem::new(
        branch.execution.call.id.clone(),
        WorkflowV2WriteMode::Worktree,
        assignment.owned_targets.clone(),
    )
    .with_owned_scopes(assignment.owned_scopes.clone());
    item.artifact_only = assignment.artifact_only;
    let root = branch.workspace_root.display().to_string();
    if let Err(err) = validate_changed_files_for_repository(&item, result, Some(&root)) {
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
    )
    {
        persist_rejected_worktree_result(
            v2_store,
            &branch.id,
            "artifact_verification",
            result,
            &error,
        );
        *result = write_branch_validation_error_result(
            &branch.id,
            Some(&branch.execution.input),
            &error,
        );
    }
    Ok(())
}

fn verify_declared_artifacts_for_result(
    input: &serde_json::Value,
    result: &WorkflowV2Result,
    workspace_root: &Path,
) -> Result<(), String> {
    if !result_requires_declared_artifact_verification(result) {
        return Ok(());
    }
    run_declared_artifact_verifiers(input, workspace_root)
}

fn result_requires_declared_artifact_verification(result: &WorkflowV2Result) -> bool {
    matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) || result
        .data
        .get("idempotent_noop")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

fn run_declared_artifact_verifiers(
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
        let output = std::process::Command::new(crate::command::posix_shell::posix_shell())
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

fn capture_worktree_branch_manifest(
    run_root: &Path,
    run_id: &str,
    execution: &WorkflowV2CallExecution,
    cfg: &WriteCoordinatorConfig,
    v2_store: &WorkflowV2ResultStore,
    result: &mut WorkflowV2Result,
    prepared: &PreparedWorktreeBranch,
) -> archon_workflow::WorkflowResult<CapturedWorktreeManifest> {
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
    Ok((Some(manifest), Some(captured.pre_hashes)))
}

fn persist_rejected_worktree_result(
    store: &WorkflowV2ResultStore,
    branch_id: &str,
    attempt: &str,
    result: &WorkflowV2Result,
    error: &str,
) {
    let raw_body = serde_json::to_string(result).unwrap_or_else(|_| result.summary.clone());
    let record = WorkflowV2RejectedOutput {
        attempt: attempt.to_string(),
        error: error.to_string(),
        raw_body,
    };
    let _ = store.append_rejected_output(branch_id, record);
}

