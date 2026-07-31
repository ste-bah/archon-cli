fn push_patch_manifest_artifact(
    result: &mut WorkflowV2Result,
    run_root: &Path,
    call_id: &str,
    branch_id: &str,
) {
    result.artifacts.push(archon_workflow::WorkflowV2Artifact {
        id: format!("patch_manifest_{branch_id}"),
        path: manifest_path_for(run_root, call_id, branch_id),
        description: Some("worktree patch manifest".to_string()),
    });
}

fn persist_worktree_manifest(
    run_root: &Path,
    run_id: &str,
    execution: &WorkflowV2CallExecution,
    branch_id: &str,
    captured: &CapturedPatch,
) -> archon_workflow::WorkflowResult<PatchManifest> {
    let status = if captured.patch_bytes.is_empty() {
        ManifestStatus::IdempotentNoop
    } else {
        ManifestStatus::PendingApply
    };
    let branch_id = branch_id.to_string();
    let path = persist_manifest(run_root, run_id, &execution.call.id, &branch_id, captured, status)
        .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
    let body = std::fs::read_to_string(&path).map_err(|err| WorkflowError::Io { path, source: err })?;
    Ok(serde_json::from_str(&body)?)
}

fn capture_and_validate_worktree_patch(
    workspace: &ItemWorkspace,
    coordinator_plan: &WritePlan,
    baseline: &CanonicalBaseline,
    cfg: &WriteCoordinatorConfig,
    result: &WorkflowV2Result,
) -> archon_workflow::WorkflowResult<CapturedPatch> {
    let captured = capture_patch(workspace, &coordinator_plan.target_files, baseline)
        .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
    let agent_body = serde_json::to_string(result)?;
    validate_captured_patch(coordinator_plan, cfg, &agent_body, captured)
}

fn validate_captured_patch(
    coordinator_plan: &WritePlan,
    cfg: &WriteCoordinatorConfig,
    agent_body: &str,
    captured: CapturedPatch,
) -> archon_workflow::WorkflowResult<CapturedPatch> {
    archon_workflow::write_coordinator::patch_manifest::validate_patch(
        &captured,
        coordinator_plan,
        cfg,
        agent_body,
    )
    .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
    Ok(captured)
}

fn coordinator_plan_for_assignment(
    run_id: &str,
    stage_id: &str,
    assignment: &WorkflowV2WriteAssignment,
    canonical_root: &Path,
) -> archon_workflow::WorkflowResult<WritePlan> {
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

fn isolated_root_for_assignment(
    assignment: &WorkflowV2WriteAssignment,
) -> archon_workflow::WorkflowResult<PathBuf> {
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

fn normalized_assignment_targets(
    assignment: &WorkflowV2WriteAssignment,
    canonical_root: &Path,
) -> archon_workflow::WorkflowResult<Vec<NormalizedPath>> {
    assignment
        .owned_targets
        .iter()
        .map(|target| {
            normalize_target(target, canonical_root)
                .map_err(|err| WorkflowError::SpecInvalid(err.to_string()))
        })
        .collect()
}

fn normalized_assignment_scopes(
    assignment: &WorkflowV2WriteAssignment,
    canonical_root: &Path,
) -> archon_workflow::WorkflowResult<Vec<NormalizedPath>> {
    assignment
        .owned_scopes
        .iter()
        .map(|target| {
            normalize_target(target, canonical_root)
                .map_err(|err| WorkflowError::SpecInvalid(err.to_string()))
        })
        .collect()
}

fn manifest_path_for(run_root: &Path, stage_id: &str, item_id: &str) -> String {
    run_root
        .join("write-coordination")
        .join("stages")
        .join(stage_id)
        .join("manifests")
        .join(format!("{item_id}.json"))
        .display()
        .to_string()
}
