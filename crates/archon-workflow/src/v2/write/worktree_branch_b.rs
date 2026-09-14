use super::*;

pub(super) fn push_patch_manifest_artifact(
    result: &mut WorkflowV2Result,
    run_root: &Path,
    call_id: &str,
    branch_id: &str,
) {
    result.artifacts.push(crate::WorkflowV2Artifact {
        id: format!("patch_manifest_{branch_id}"),
        path: manifest_path_for(run_root, call_id, branch_id),
        description: Some("worktree patch manifest".to_string()),
    });
}

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
    let captured = capture_patch(&workspace, &plan.target_files, baseline)
        .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
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
        grant,
        &prepared.baseline,
        ctx.cfg,
        result,
    ) {
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
    push_patch_manifest_artifact(result, ctx.run_root, &ctx.execution.call.id, branch_id);
    report_ignored_deliverables(result, &manifest);
    report_scope_grant(result, grant);
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
        result.summary.push_str(&format!("\nDeliverable {path} is gitignored — not committed; retained at {artifact}."));
        result.artifacts.push(crate::WorkflowV2Artifact { id: format!("ignored_{}_{}", manifest.item_id, result.artifacts.len()),
            path: artifact.clone(), description: Some(format!("gitignored deliverable {path}; not committed")) });
    }
    if let Some(data) = result.data.as_object_mut() {
        if !manifest.skipped_ignored.is_empty() {
            data.insert("skipped_ignored".into(), serde_json::json!(manifest.skipped_ignored));
            if manifest.status == ManifestStatus::SkippedIgnored { data.insert("patch_landed".into(), false.into()); }
        }
    }
}
