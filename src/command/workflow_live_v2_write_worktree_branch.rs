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
        let output = std::process::Command::new("sh")
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
