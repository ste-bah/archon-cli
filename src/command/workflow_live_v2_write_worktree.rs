struct PreparedWorktreeBranch {
    branch: archon_workflow::WorkflowV2FanoutItem,
    assignment: WorkflowV2WriteAssignment,
    coordinator_plan: WritePlan,
    baseline: CanonicalBaseline,
    workspace: ItemWorkspace,
}

struct CompletedWorktreeBranch {
    item_id: String,
    role: String,
    item_input_hash: Option<String>,
    result: WorkflowV2Result,
    manifest: Option<PatchManifest>,
    pre_hashes: Option<BTreeMap<String, String>>,
    workspace_root: PathBuf,
}

async fn run_worktree_v2_write_fanout(
    task: &str,
    target_repository_root: Option<&str>,
    execution: &WorkflowV2CallExecution,
    adapter: WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: &WorkflowV2ResultStore,
    store_for_control: &archon_workflow::WorkflowStore,
    run_id: &str,
    branches: Vec<archon_workflow::WorkflowV2FanoutItem>,
    plan: WorkflowV2WritePlan,
    reused_results: Vec<WorkflowV2Result>,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let canonical_root = target_repository_root
        .filter(|root| !root.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            WorkflowError::SpecInvalid(
                "worktree write mode requires target_repository_root".to_string(),
            )
        })?;
    let cfg = WriteCoordinatorConfig::default();
    let run_root = v2_store
        .root()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| v2_store.root().to_path_buf());
    let mut prepared = Vec::new();
    for wave in &plan.waves {
        for assignment in &wave.assignments {
            let branch = branches
                .iter()
                .find(|branch| branch.id == assignment.item_id)
                .cloned()
                .ok_or_else(|| {
                    WorkflowError::SpecInvalid(format!(
                        "write plan referenced missing fanout item '{}'",
                        assignment.item_id
                    ))
                })?;
            poll_v2_run_control(store_for_control, run_id, &branch.id)?;
            let coordinator_plan = coordinator_plan_for_assignment(
                run_id,
                &execution.call.id,
                assignment,
                &canonical_root,
            )?;
            let baseline = capture_canonical_baseline(
                &canonical_root,
                &coordinator_plan,
                &coordinator_plan.verify_inputs,
                &cfg,
            )
            .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
            let workspace = create_item_workspace(&canonical_root, &coordinator_plan, &baseline)
                .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
            prepared.push(PreparedWorktreeBranch {
                branch,
                assignment: assignment.clone(),
                coordinator_plan,
                baseline,
                workspace,
            });
        }
    }

    let max_parallelism = client.fanout_parallelism(execution.call.options.max_parallelism);
    let semaphore = Arc::new(Semaphore::new(max_parallelism));
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let jobs = prepared.into_iter().map(|prepared| {
        let adapter = adapter.clone();
        let control_store = store_for_control.clone();
        let run_id = run_id.to_string();
        let task = task.to_string();
        let target_repository_root = target_repository_root.map(str::to_string);
        let run_root = run_root.clone();
        let canonical_root = canonical_root.clone();
        let branch_v2_store = v2_store.clone();
        let cfg = cfg.clone();
        let semaphore = semaphore.clone();
        let active = active.clone();
        let peak = peak.clone();
        async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
            poll_v2_run_control(&control_store, &run_id, &prepared.branch.id)?;
            let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
            record_write_peak(&peak, now_active);
            let result = run_one_worktree_branch(
                &task,
                target_repository_root,
                execution,
                adapter,
                client,
                &control_store,
                &run_id,
                &run_root,
                &canonical_root,
                &branch_v2_store,
                &cfg,
                prepared,
            )
            .await;
            active.fetch_sub(1, Ordering::SeqCst);
            result
        }
    });

    let mut completed = Vec::new();
    for item in futures_util::future::join_all(jobs).await {
        completed.push(item?);
    }
    let peak_parallelism = peak.load(Ordering::SeqCst);

    let mut branch_results = Vec::new();
    let mut manifests = Vec::new();
    let mut pre_hashes_by_item = BTreeMap::new();
    for completed_branch in &completed {
        let mut result = completed_branch.result.clone();
        tag_branch_result(&mut result, &completed_branch.item_id);
        normalize_write_branch_contract_result(&mut result);
        save_write_branch_outcome(
            v2_store,
            &execution.call.id,
            &completed_branch.item_id,
            &completed_branch.role,
            completed_branch.item_input_hash.clone(),
            &result,
        )?;
        branch_results.push(result);
        if let Some(manifest) = &completed_branch.manifest {
            manifests.push(manifest.clone());
        }
        if let Some(pre_hashes) = &completed_branch.pre_hashes {
            pre_hashes_by_item.insert(completed_branch.item_id.clone(), pre_hashes.clone());
        }
    }

    let mut apply_gap = None::<String>;
    if !manifests.is_empty() {
        if !plan.conflicts.is_empty() {
            apply_gap = Some(
                "worktree patches were captured but overlapping targets require operator review before canonical apply"
                    .to_string(),
            );
        } else {
            let apply_result = with_repo_lock(&canonical_root, || {
                apply_wave(
                    &canonical_root,
                    &manifests,
                    &pre_hashes_by_item,
                    0,
                    &run_root,
                    run_id,
                    &execution.call.id,
                )
            });
            match apply_result {
                Ok(record) if !record.items_failed.is_empty() => {
                    apply_gap = Some(format!(
                        "worktree patch apply left {} item(s) unapplied: {}",
                        record.items_failed.len(),
                        record
                            .items_failed
                            .iter()
                            .map(|(item, reason)| format!("{item}: {reason}"))
                            .collect::<Vec<_>>()
                            .join("; ")
                    ));
                }
                Ok(_) => {}
                Err(err) => {
                    apply_gap = Some(format!("worktree patch apply failed: {err}"));
                }
            }
        }
    }

    for completed_branch in &completed {
        let status = if apply_gap.is_none() {
            WorkspaceStatus::Succeeded
        } else {
            WorkspaceStatus::Failed
        };
        let _ = cleanup_workspace(
            &canonical_root,
            &completed_branch.workspace_root,
            status,
            &cfg,
        );
    }

    let mut all_results = reused_results;
    all_results.extend(branch_results);
    let mut result =
        result_from_write_fanout(&execution.call, all_results, &plan, peak_parallelism, None);
    if let Some(reason) = apply_gap {
        result.status = WorkflowV2Status::NeedsReview;
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Review,
            reason.clone(),
        ));
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id: format!(
                "worktree_apply_review_{}",
                sanitize_v2_path_segment(&execution.call.id)
            ),
            description: reason,
            severity: Some("review".to_string()),
        });
    }
    if let Some(object) = result.data.as_object_mut() {
        object.insert(
            "worktree_apply_manifests".to_string(),
            serde_json::Value::Array(
                manifests
                    .iter()
                    .map(|manifest| {
                        serde_json::Value::String(manifest_path_for(
                            &run_root,
                            &execution.call.id,
                            &manifest.item_id,
                        ))
                    })
                    .collect(),
            ),
        );
    }
    Ok(result)
}

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
    let branch_id = prepared.branch.id.clone();
    let branch_role = prepared.branch.role.clone();
    let branch_input_hash = Some(prepared.branch.input_hash());
    let workspace_root = prepared.workspace.plan.isolated_root.clone();
    poll_v2_run_control(store_for_control, run_id, &branch_id)?;
    let mut branch_call = prepared.branch.call;
    branch_call.options.target_files = prepared.assignment.owned_targets.clone();
    let branch_execution = WorkflowV2CallExecution {
        call: branch_call,
        input: prepared.branch.input,
        depends_on: vec![execution.call.id.clone()],
    };
    let mut result = match run_single_v2_agent_call_in_repository(
        task,
        target_repository_root,
        &branch_execution,
        &adapter,
        client,
        Some(v2_store),
        Some(workspace_root.display().to_string()),
    )
    .await
    {
        Ok(result) => result,
        Err(err) if is_recoverable_write_branch_timeout(&err.to_string()) => {
            write_branch_runtime_timeout_result(
                &branch_id,
                &branch_execution.input,
                &err.to_string(),
            )
        }
        Err(err) if is_write_branch_validation_error(&err.to_string()) => {
            write_branch_validation_error_result(
                &branch_id,
                Some(&branch_execution.input),
                &err.to_string(),
            )
        }
        Err(err) => return Err(err),
    };
    poll_v2_run_control(store_for_control, run_id, &branch_id)?;
    let write_item = WorkflowV2WriteItem::new(
        branch_execution.call.id.clone(),
        WorkflowV2WriteMode::Worktree,
        prepared.assignment.owned_targets.clone(),
    );
    if let Err(err) = validate_changed_files_for_repository(
        &write_item,
        &result,
        Some(&workspace_root.display().to_string()),
    ) {
        if is_write_branch_validation_error(&err.to_string()) {
            result = write_branch_validation_error_result(
                &branch_id,
                Some(&branch_execution.input),
                &err.to_string(),
            );
        } else {
            return Err(WorkflowError::SpecInvalid(err.to_string()));
        }
    }
    let mut manifest = None;
    let mut pre_hashes = None;
    if matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        let captured = match capture_and_validate_worktree_patch(
            &prepared.workspace,
            &prepared.coordinator_plan,
            &prepared.baseline,
            cfg,
            &result,
        ) {
            Ok(captured) => captured,
            Err(err) if is_write_branch_validation_error(&err.to_string()) => {
                result = write_branch_validation_error_result(
                    &branch_id,
                    Some(&branch_execution.input),
                    &err.to_string(),
                );
                return Ok(CompletedWorktreeBranch {
                    item_id: branch_id,
                    role: branch_role,
                    item_input_hash: branch_input_hash,
                    result,
                    manifest: None,
                    pre_hashes: None,
                    workspace_root,
                });
            }
            Err(err) => return Err(err),
        };
        let status = if captured.patch_bytes.is_empty() {
            ManifestStatus::IdempotentNoop
        } else {
            ManifestStatus::PendingApply
        };
        let manifest_path = persist_manifest(
            run_root,
            run_id,
            &execution.call.id,
            &branch_id,
            &captured,
            status,
        )
        .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
        let manifest_body =
            std::fs::read_to_string(&manifest_path).map_err(|err| WorkflowError::Io {
                path: manifest_path.clone(),
                source: err,
            })?;
        let parsed_manifest: PatchManifest = serde_json::from_str(&manifest_body)?;
        result.artifacts.push(archon_workflow::WorkflowV2Artifact {
            id: format!("patch_manifest_{branch_id}"),
            path: manifest_path.display().to_string(),
            description: Some("worktree patch manifest".to_string()),
        });
        pre_hashes = Some(captured.pre_hashes.clone());
        manifest = Some(parsed_manifest);
    }
    let _ = canonical_root;
    Ok(CompletedWorktreeBranch {
        item_id: branch_id,
        role: branch_role,
        item_input_hash: branch_input_hash,
        result,
        manifest,
        pre_hashes,
        workspace_root,
    })
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
    validate_patch(&captured, coordinator_plan, cfg, &agent_body)
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
