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

pub(super) fn capture_and_validate_worktree_patch(
    workspace: &ItemWorkspace,
    coordinator_plan: &WritePlan,
    baseline: &CanonicalBaseline,
    cfg: &WriteCoordinatorConfig,
    result: &WorkflowV2Result,
    wave_claims: Option<&[crate::v2::write_scope_extension::WaveClaim]>,
) -> crate::WorkflowResult<CapturedPatch> {
    // ONE effective plan for all three gates. Capture reads
    // `workspace.plan`, the diff scope reads the targets argument, and
    // `validate_patch` reads the plan again — widening any one of them alone
    // leaves the other two rejecting the same path.
    let plan = super::worktree_scope_grant::plan_extended_to_unclaimed_changes(
        coordinator_plan,
        result,
        wave_claims,
    );
    let workspace = ItemWorkspace {
        plan: plan.clone(),
        baseline_commit: workspace.baseline_commit.clone(),
    };
    // A granted path was not in the baseline, so it would carry no pre-hash and
    // the apply-time stale recheck would skip it — leaving the overlap guard
    // alone between two items writing the same file. Sound to hash now: every
    // branch in a wave captures before anything applies, so canonical is still
    // the content these patches were computed against.
    let granted: Vec<String> = plan
        .target_files
        .iter()
        .map(|path| path.as_str().to_string())
        .filter(|path| {
            !coordinator_plan
                .target_files
                .iter()
                .any(|declared| declared.as_str() == path.as_str())
        })
        .collect();
    let baseline =
        &crate::write_coordinator::worktree_isolation::extend_baseline_with_granted_targets(
            baseline,
            &plan.canonical_root,
            &granted,
        );
    let captured = capture_patch(&workspace, &plan.target_files, baseline)
        .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
    let agent_body = serde_json::to_string(result)?;
    validate_captured_patch(&plan, cfg, &agent_body, captured)
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
