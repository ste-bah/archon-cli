use super::*;

pub(super) fn persist_worktree_manifest(
    run_root: &Path,
    run_id: &str,
    execution: &WorkflowV2CallExecution,
    branch_id: &str,
    captured: &CapturedPatch,
) -> crate::WorkflowResult<PatchManifest> {
    let status = if captured.patch_bytes.is_empty() {
        ManifestStatus::IdempotentNoop
    } else {
        ManifestStatus::PendingApply
    };
    let branch_id = branch_id.to_string();
    let path = persist_manifest(
        run_root,
        run_id,
        &execution.call.id,
        &branch_id,
        captured,
        status,
    )
    .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
    let body =
        std::fs::read_to_string(&path).map_err(|err| WorkflowError::Io { path, source: err })?;
    Ok(serde_json::from_str(&body)?)
}

/// Gates 2 and 3, against the plan gate 1 already accepted the envelope under.
pub(super) fn capture_and_validate_worktree_patch(
    workspace: &ItemWorkspace,
    grant: &super::worktree_scope_grant::ScopeGrant,
    baseline: &CanonicalBaseline,
    cfg: &WriteCoordinatorConfig,
    result: &WorkflowV2Result,
    delivered_artifacts: Vec<String>,
) -> crate::WorkflowResult<CapturedPatch> {
    // ONE effective plan for all three gates, resolved once by the caller.
    // Capture reads `workspace.plan`, the diff scope reads the targets
    // argument, and `validate_patch` reads the plan again — widening any one
    // of them alone leaves the other two rejecting the same path.
    let plan = &grant.plan;
    let workspace = ItemWorkspace {
        plan: plan.clone(),
        baseline_commit: workspace.baseline_commit.clone(),
        materialized_ignored: workspace.materialized_ignored.clone(),
    };
    // A granted path was not in the baseline, so it would carry no pre-hash and
    // the apply-time stale recheck would skip it — leaving the overlap guard
    // alone between two items writing the same file. Sound to hash now: every
    // branch in a wave captures before anything applies, so canonical is still
    // the content these patches were computed against.
    let baseline =
        &crate::write_coordinator::worktree_isolation::extend_baseline_with_granted_targets(
            baseline,
            &plan.canonical_root,
            &grant.granted,
        );
    let mut captured = capture_patch(&workspace, &plan.target_files, baseline)
        .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
    // Issue-69: a declared project artifact the host saw change lives outside
    // the repository, so the diff above cannot carry it; told to the gate so
    // an artifact-only delivery is not refused as an empty patch.
    captured.delivered_artifacts = delivered_artifacts;
    let agent_body = serde_json::to_string(result)?;
    validate_captured_patch(plan, cfg, &agent_body, captured)
}

/// Gates 2 and 3 for one branch, then the manifest: skipped unless the
/// envelope was accepted, and judged against the same [`ScopeGrant`] gate 1
/// already accepted it under.
///
/// [`ScopeGrant`]: super::worktree_scope_grant::ScopeGrant
pub(super) fn capture_worktree_branch_manifest(
    ctx: &WorktreeWaveRunContext<'_>,
    result: &mut WorkflowV2Result,
    prepared: &PreparedWorktreeBranch,
    grant: &super::worktree_scope_grant::ScopeGrant,
    delivered_artifacts: Vec<String>,
) -> crate::WorkflowResult<CapturedWorktreeManifest> {
    if !matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        return Ok((None, None));
    }
    let branch_id = prepared.branch.id.as_str();
    // Issue-17: a zero-match test command is a verdict only when it is a
    // declared focused test or cited as evidence; the rest become a review
    // gap once the patch is validated.
    let declared = super::zero_match_commands::declared_focused_tests(&prepared.branch.input);
    let captured = match super::zero_match_commands::reject_declared_zero_match(result, &declared)
        .and_then(|()| {
            capture_and_validate_worktree_patch(
                &prepared.workspace,
                grant,
                &prepared.baseline,
                ctx.cfg,
                result,
                delivered_artifacts,
            )
        }) {
        Ok(captured) => captured,
        Err(err) => {
            persist_rejected_worktree_result(
                ctx.v2_store,
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
    let manifest = persist_worktree_manifest(
        ctx.run_root,
        ctx.run_id,
        ctx.execution,
        branch_id,
        &captured,
    )?;
    // Issue-76: the manifest this just persisted is the HOST's record of the
    // branch, not a deliverable of it, and it used to be pushed onto
    // `result.artifacts` from here. A rejected attempt's envelope is replayed
    // verbatim into the next coder's prompt, so that entry read to the coder
    // as an artifact it was required to produce — and one reproduced the
    // host's file inside its worktree, where it was granted and committed into
    // the target repository (see `host_internal_artifacts`). Host-internal
    // paths are recorded nowhere an agent prompt renders.
    report_ignored_deliverables(result, &manifest);
    report_scope_grant(result, grant);
    super::zero_match_commands::report_incidental_zero_match(result, branch_id, &declared);
    Ok((Some(manifest), Some(captured.pre_hashes)))
}

/// Record on an accepted branch which paths it was granted beyond its declared
/// targets, so a reviewer can see them without diffing the manifest's
/// `declared_target_files` against the task. Silent when nothing was granted.
pub(super) fn report_scope_grant(
    result: &mut WorkflowV2Result,
    grant: &super::worktree_scope_grant::ScopeGrant,
) {
    if grant.granted.is_empty() {
        return;
    }
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        format!(
            "scope granted beyond declared targets (changed, unclaimed by any other item in the \
             wave): {}",
            grant.granted.join(", ")
        ),
    ));
    if let Some(data) = result.data.as_object_mut() {
        data.insert(
            "scope_granted".to_string(),
            serde_json::json!(grant.granted),
        );
    }
}

/// Gap id prefix for the whitespace-only out-of-scope paths a branch dropped.
pub(crate) const WHITESPACE_ONLY_DROPPED_GAP_PREFIX: &str = "whitespace_only_changes_dropped_";

/// Gap id prefix for the real changes outside its scope roots a branch dropped.
pub(crate) const OUT_OF_SCOPE_DROPPED_GAP_PREFIX: &str = "out_of_scope_changes_dropped_";

/// Gap id prefix for the changed paths a branch's envelope did not list.
pub(crate) const FILES_CHANGED_UNDERREPORTED_GAP_PREFIX: &str = "files_changed_underreported_";

/// How many paths a gap names before summarising the rest.
const GAP_PATHS_LISTED: usize = 20;

/// `paths` for a gap or evidence line: the first [`GAP_PATHS_LISTED`] in
/// full, the rest as a count, so a tree-wide change stays readable.
fn bounded_path_list(paths: &[String]) -> String {
    let listed = paths
        .iter()
        .take(GAP_PATHS_LISTED)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if paths.len() > GAP_PATHS_LISTED {
        format!(
            "{listed} … ({} more; {} in total)",
            paths.len() - GAP_PATHS_LISTED,
            paths.len()
        )
    } else {
        listed
    }
}

/// Record the changed paths this branch's envelope did not list (Issue-16) as
/// a review gap and an evidence line, whatever the branch's status. The
/// paths were judged by the ownership gates like any listed one — granted,
/// or refused as contested — so under-reporting is a finding for the
/// reviewer, never a verdict. Status and summary are untouched.
pub(super) fn report_underreported_changes(
    result: &mut WorkflowV2Result,
    branch_id: &str,
    unreported: &[String],
) {
    if unreported.is_empty() {
        return;
    }
    let paths = bounded_path_list(unreported);
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "{FILES_CHANGED_UNDERREPORTED_GAP_PREFIX}{}",
            sanitize_v2_path_segment(branch_id)
        ),
        description: format!(
            "write item '{branch_id}' changed {} path(s) in its worktree that its envelope's \
             files_changed did not list; each was judged by the ownership gates exactly as \
             a listed path is: {paths}. Report every file you change.",
            unreported.len()
        ),
        severity: Some("review".to_string()),
    });
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        format!(
            "files_changed under-reported: {} changed path(s) not listed in the envelope: \
             {paths}",
            unreported.len()
        ),
    ));
    if let Some(data) = result.data.as_object_mut() {
        data.insert(
            "files_changed_underreported".to_string(),
            serde_json::json!(unreported),
        );
    }
}

/// Record the whitespace-only out-of-scope paths this branch dropped (Issue-13)
/// as a review gap and an evidence line, whatever the branch's status: the
/// files were restored in the worktree before any gate read it, so a reviewer
/// has to be told from here. Status and summary are untouched.
pub(super) fn report_whitespace_only_drops(
    result: &mut WorkflowV2Result,
    branch_id: &str,
    dropped: &[String],
) {
    if dropped.is_empty() {
        return;
    }
    let paths = bounded_path_list(dropped);
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "{WHITESPACE_ONLY_DROPPED_GAP_PREFIX}{}",
            sanitize_v2_path_segment(branch_id)
        ),
        description: format!(
            "write item '{branch_id}' changed {} path(s) outside its declared targets by \
             whitespace only (a tree-wide formatter, most likely); each was restored to \
             the baseline in the worktree and excluded from the patch rather than \
             failing the branch: {paths}. Run formatters on the files you changed, not \
             the whole tree.",
            dropped.len()
        ),
        severity: Some("review".to_string()),
    });
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        format!(
            "whitespace-only changes outside the declared targets dropped from the patch \
             ({} path(s)): {paths}",
            dropped.len()
        ),
    ));
    if let Some(data) = result.data.as_object_mut() {
        data.insert(
            "whitespace_only_dropped".to_string(),
            serde_json::json!(dropped),
        );
    }
}

/// Record the real changes outside the plan's scope roots this branch dropped
/// (Issue-27) as a review gap and an evidence line, whatever the branch's
/// status: like a whitespace-only drop, the files were restored or removed
/// in the worktree before any gate read it, so a reviewer has to be told from
/// here — and told the roots, so the finding can be judged against the task
/// rather than the tree. Status and summary are untouched.
pub(super) fn report_out_of_scope_drops(
    result: &mut WorkflowV2Result,
    branch_id: &str,
    dropped: &[String],
    roots: &str,
) {
    if dropped.is_empty() {
        return;
    }
    let paths = bounded_path_list(dropped);
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "{OUT_OF_SCOPE_DROPPED_GAP_PREFIX}{}",
            sanitize_v2_path_segment(branch_id)
        ),
        description: format!(
            "write item '{branch_id}' changed {} path(s) outside its scope roots ({roots}); \
             each was restored to the baseline or removed in the worktree and excluded \
             from the patch rather than granted: {paths}. Change only files inside the \
             task's scope; a needed change elsewhere is a residual gap to report, not an \
             edit to make.",
            dropped.len()
        ),
        severity: Some("review".to_string()),
    });
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        format!(
            "out-of-scope changes dropped from the patch ({} path(s) outside scope roots \
             {roots}): {paths}",
            dropped.len()
        ),
    ));
    if let Some(data) = result.data.as_object_mut() {
        data.insert(
            "out_of_scope_dropped".to_string(),
            serde_json::json!(dropped),
        );
    }
}

pub(super) fn validate_captured_patch(
    coordinator_plan: &WritePlan,
    cfg: &WriteCoordinatorConfig,
    agent_body: &str,
    captured: CapturedPatch,
) -> crate::WorkflowResult<CapturedPatch> {
    crate::write_coordinator::patch_manifest::validate_patch(
        &captured,
        coordinator_plan,
        cfg,
        agent_body,
    )
    .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
    Ok(captured)
}

pub(crate) fn coordinator_plan_for_assignment(
    run_id: &str,
    stage_id: &str,
    assignment: &WorkflowV2WriteAssignment,
    canonical_root: &Path,
) -> crate::WorkflowResult<WritePlan> {
    let isolated_root = isolated_root_for_assignment(assignment)?;
    let targets = normalized_assignment_targets(assignment, canonical_root)?;
    let resource_keys = resource_keys_for_targets(&targets, canonical_root, &[])
        .map_err(|err| WorkflowError::SpecInvalid(err.to_string()))?;
    Ok(WritePlan {
        run_id: run_id.to_string(),
        stage_id: stage_id.to_string(),
        item_id: assignment.item_id.clone(),
        canonical_root: canonical_root.to_path_buf(),
        isolated_root,
        target_files: targets,
        target_dir_scopes: normalized_assignment_scopes(assignment, canonical_root)?,
        target_files_source: TargetFilesSource::Item,
        read_context_files: Vec::new(),
        verify_inputs: Vec::new(),
        baseline_id: "git:HEAD".to_string(),
        workspace_boundary_required: true,
        resource_keys,
    })
}

pub(super) fn isolated_root_for_assignment(
    assignment: &WorkflowV2WriteAssignment,
) -> crate::WorkflowResult<PathBuf> {
    assignment
        .worktree_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| {
            WorkflowError::SpecInvalid(format!(
                "worktree assignment '{}' has no isolated root",
                assignment.item_id
            ))
        })
}

pub(super) fn normalized_assignment_targets(
    assignment: &WorkflowV2WriteAssignment,
    canonical_root: &Path,
) -> crate::WorkflowResult<Vec<NormalizedPath>> {
    assignment
        .owned_targets
        .iter()
        .map(|target| {
            normalize_target(target, canonical_root)
                .map_err(|err| WorkflowError::SpecInvalid(err.to_string()))
        })
        .collect()
}

pub(super) fn normalized_assignment_scopes(
    assignment: &WorkflowV2WriteAssignment,
    canonical_root: &Path,
) -> crate::WorkflowResult<Vec<NormalizedPath>> {
    assignment
        .owned_scopes
        .iter()
        .map(|target| {
            normalize_target(target, canonical_root)
                .map_err(|err| WorkflowError::SpecInvalid(err.to_string()))
        })
        .collect()
}

pub(crate) fn manifest_path_for(run_root: &Path, stage_id: &str, item_id: &str) -> String {
    run_root
        .join("write-coordination")
        .join("stages")
        .join(stage_id)
        .join("manifests")
        .join(format!("{item_id}.json"))
        .display()
        .to_string()
}

pub(super) fn report_ignored_deliverables(result: &mut WorkflowV2Result, manifest: &PatchManifest) {
    for (path, artifact) in &manifest.skipped_ignored {
        result.summary.push_str(&format!(
            "\nDeliverable {path} is gitignored — not committed; retained at {artifact}."
        ));
        result.artifacts.push(crate::WorkflowV2Artifact {
            id: format!("ignored_{}_{}", manifest.item_id, result.artifacts.len()),
            path: artifact.clone(),
            description: Some(format!("gitignored deliverable {path}; not committed")),
        });
    }
    if let Some(data) = result.data.as_object_mut()
        && !manifest.skipped_ignored.is_empty()
    {
        // `patch_landed` is NOT answered here. A skipped-ignored manifest is
        // "nothing to commit", not "nothing changed": whether the deliverable
        // moved off the baseline is `worktree_patch_landed`'s answer, stamped
        // once by `mark_patch_landed`.
        data.insert(
            "skipped_ignored".into(),
            serde_json::json!(manifest.skipped_ignored),
        );
    }
}
