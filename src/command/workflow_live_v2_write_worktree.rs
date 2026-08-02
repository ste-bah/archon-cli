use super::*;

pub(super) struct PreparedWorktreeBranch {
    pub(super) branch: archon_workflow::WorkflowV2FanoutItem,
    pub(super) assignment: WorkflowV2WriteAssignment,
    pub(super) coordinator_plan: WritePlan,
    pub(super) baseline: CanonicalBaseline,
    pub(super) workspace: ItemWorkspace,
}

pub(super) struct CompletedWorktreeBranch {
    pub(super) item_id: String,
    pub(super) role: String,
    pub(super) item_input_hash: Option<String>,
    pub(super) result: WorkflowV2Result,
    pub(super) manifest: Option<PatchManifest>,
    pub(super) pre_hashes: Option<BTreeMap<String, String>>,
    pub(super) workspace_root: PathBuf,
}

pub(super) struct WorktreeFanoutSetup {
    pub(super) canonical_root: PathBuf,
    pub(super) cfg: WriteCoordinatorConfig,
    pub(super) run_root: PathBuf,
}

pub(super) async fn run_worktree_v2_write_fanout(
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
    let setup = worktree_fanout_setup(target_repository_root, v2_store)?;
    let artifacts = run_worktree_plan_waves(
        worktree_plan_context(
            task,
            target_repository_root,
            execution,
            adapter,
            client,
            v2_store,
            store_for_control,
            run_id,
            &setup,
        ),
        &plan,
        &branches,
    )
    .await?;

    Ok(worktree_fanout_result(
        &execution.call,
        &plan,
        &setup.run_root,
        reused_results,
        artifacts,
    ))
}

pub(super) fn worktree_fanout_setup(
    target_repository_root: Option<&str>,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<WorktreeFanoutSetup> {
    let canonical_root = target_repository_root
        .filter(|root| !root.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            WorkflowError::SpecInvalid(
                "worktree write mode requires target_repository_root".to_string(),
            )
        })?;
    let run_root = v2_store
        .root()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| v2_store.root().to_path_buf());
    Ok(WorktreeFanoutSetup {
        canonical_root,
        cfg: WriteCoordinatorConfig::default(),
        run_root,
    })
}

pub(super) fn worktree_manifest_paths(
    run_root: &Path,
    call_id: &str,
    manifests: &[PatchManifest],
) -> serde_json::Value {
    serde_json::Value::Array(
        manifests
            .iter()
            .map(|manifest| {
                serde_json::Value::String(manifest_path_for(run_root, call_id, &manifest.item_id))
            })
            .collect(),
    )
}

pub(super) fn worktree_fanout_result(
    call: &WorkflowV2HostCall,
    plan: &WorkflowV2WritePlan,
    run_root: &Path,
    mut reused_results: Vec<WorkflowV2Result>,
    artifacts: WorktreePlanArtifacts,
) -> WorkflowV2Result {
    reused_results.extend(artifacts.results);
    let mut result =
        result_from_write_fanout(call, reused_results, plan, artifacts.peak_parallelism, None);
    attach_worktree_apply_gap(call, &mut result, artifacts.apply_gap);
    if let Some(object) = result.data.as_object_mut() {
        object.insert(
            "worktree_apply_manifests".to_string(),
            worktree_manifest_paths(run_root, &call.id, &artifacts.manifests),
        );
    }
    result
}

pub(super) fn attach_worktree_apply_gap(
    call: &WorkflowV2HostCall,
    result: &mut WorkflowV2Result,
    apply_gap: Option<String>,
) {
    let Some(reason) = apply_gap else {
        return;
    };
    result.status = WorkflowV2Status::NeedsReview;
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        reason.clone(),
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "worktree_apply_review_{}",
            sanitize_v2_path_segment(&call.id)
        ),
        description: reason,
        severity: Some("review".to_string()),
    });
}
