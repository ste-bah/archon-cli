use super::*;

fn wave_branch_identity(item_id: &str, workspace_root: &Path) -> WorktreeBranchIdentity {
    WorktreeBranchIdentity {
        item_id: item_id.to_string(),
        role: "coder".to_string(),
        item_input_hash: Some(format!("hash-{item_id}")),
        workspace_root: workspace_root.to_path_buf(),
        input: serde_json::json!({
            "item": {"item_id": item_id, "canonical_task_ids": ["TASK-001"]},
        }),
    }
}

fn accepted_wave_branch(item_id: &str, workspace_root: &Path) -> CompletedWorktreeBranch {
    let mut result = WorkflowV2Result::accepted("sibling branch implemented its item");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "sibling branch changed its declared target file",
    ));
    result.data = serde_json::json!({
        "item_id": item_id,
        "canonical_task_ids": ["TASK-001"],
    });
    CompletedWorktreeBranch {
        item_id: item_id.to_string(),
        role: "coder".to_string(),
        item_input_hash: Some(format!("hash-{item_id}")),
        result,
        manifest: None,
        pre_hashes: None,
        workspace_root: workspace_root.to_path_buf(),
    }
}

/// The #163 wave-scoped discard: `run_prepared_worktree_wave` collected with
/// `completed.push(item?)`, so one unrecognised `Err` returned before
/// `collect_worktree_wave_artifacts` — the ONLY caller of
/// `save_write_branch_outcome` — ever ran, and siblings that had already
/// finished were dropped without reaching `v2/branches/<call_id>/`.
#[test]
fn one_branch_error_still_persists_its_siblings_wave_outcome() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let identities = vec![
        wave_branch_identity("remediation-wave-1-task-a", temp.path()),
        wave_branch_identity("remediation-wave-1-task-b", temp.path()),
    ];
    let outcomes = vec![
        Ok(accepted_wave_branch(
            "remediation-wave-1-task-a",
            temp.path(),
        )),
        Err(WorkflowError::StageFailed(
            "worktree branch workspace vanished mid-run".to_string(),
        )),
    ];

    let completed = worktree_wave_outcomes(identities, outcomes)
        .expect("an unrecognised branch error must not unwind the wave");
    assert_eq!(completed.len(), 2);

    let artifacts = collect_worktree_wave_artifacts(completed, &store, "remediation-wave-1")
        .expect("collect wave artifacts");
    assert_eq!(artifacts.results.len(), 2);

    let sibling = store
        .load_branch_outcome("remediation-wave-1", "remediation-wave-1-task-a")
        .expect("load sibling")
        .expect("the finished sibling's outcome must be on disk");
    assert_eq!(sibling.status, WorkflowV2Status::Accepted);

    let failed = store
        .load_branch_outcome("remediation-wave-1", "remediation-wave-1-task-b")
        .expect("load failed branch")
        .expect("the failed branch is recorded as branch-scoped data");
    assert_ne!(failed.status, WorkflowV2Status::Accepted);
    let failed_result = failed.result.expect("failed branch keeps a typed result");
    assert_eq!(failed_result.data["branch_error_unclassified"], true);
    assert_eq!(failed_result.data["branch_error_from_runtime"], true);
    assert!(
        failed_result
            .residual_gaps
            .iter()
            .any(|gap| gap.id == "invalid_write_branch_output_remediation-wave-1-task-b")
    );
}

/// Control-flow and host-bug errors are NOT branch data and must still unwind.
#[test]
fn control_and_host_bug_errors_still_unwind_the_wave() {
    let temp = tempfile::tempdir().expect("tempdir");
    for error in [
        WorkflowError::ControlPaused("run paused by operator".to_string()),
        WorkflowError::ControlCancelled("run cancelled by operator".to_string()),
        WorkflowError::NotificationDelivery("required approval mail failed".to_string()),
        WorkflowError::SpecInvalid("write plan referenced missing fanout item".to_string()),
    ] {
        assert!(is_fatal_worktree_wave_error(&error));
        let identities = vec![
            wave_branch_identity("remediation-wave-1-task-a", temp.path()),
            wave_branch_identity("remediation-wave-1-task-b", temp.path()),
        ];
        let outcomes = vec![
            Ok(accepted_wave_branch(
                "remediation-wave-1-task-a",
                temp.path(),
            )),
            Err(error),
        ];

        assert!(
            worktree_wave_outcomes(identities, outcomes).is_err(),
            "control-flow and host-bug errors must unwind the wave"
        );
    }
}

/// `WorkflowV2WriteSafetyError::UnsafeTarget` is the phrasing the ownership
/// guard actually produced live, and `undeclared_write_paths` did not match it —
/// so the actionable `scope_expansion_needed_*` gap the code already knows how
/// to build was never emitted for the failure that happened.
#[test]
fn unsafe_write_target_emits_the_scope_expansion_gap() {
    let error = "write target '/tmp/repo/crates/archon-trading/src/data_lake.rs' for item 'impl-T001' is unsafe";

    assert_eq!(
        undeclared_write_paths(error),
        vec!["/tmp/repo/crates/archon-trading/src/data_lake.rs".to_string()]
    );

    let result = write_branch_validation_error_result("impl-T001", None, error);

    assert!(result.residual_gaps.iter().any(|gap| {
        gap.id == "scope_expansion_needed_impl-T001"
            && gap
                .description
                .contains("crates/archon-trading/src/data_lake.rs")
    }));
}
