use super::*;

#[test]
fn worktree_assignment_builds_coordinator_plan_with_isolated_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).expect("repo");
    std::fs::write(repo.join("src/lib.rs"), "pub fn existing() {}\n").expect("file");
    let assignment = WorkflowV2WriteAssignment {
        item_id: "impl-T001".to_string(),
        owned_targets: vec!["src/lib.rs".to_string()],
        owned_scopes: Vec::new(),
        worktree_path: Some(temp.path().join("wt").display().to_string()),
        artifact_only: false,
    };

    let plan = coordinator_plan_for_assignment("wf-test", "impl", &assignment, &repo)
        .expect("coordinator plan");

    assert_eq!(plan.item_id, "impl-T001");
    assert_eq!(plan.stage_id, "impl");
    assert_eq!(plan.isolated_root, temp.path().join("wt"));
    assert_eq!(plan.target_files[0].as_str(), "src/lib.rs");
    assert!(plan.workspace_boundary_required);
}

#[test]
fn worktree_write_result_does_not_record_serial_fallback_when_active() {
    let temp = tempfile::tempdir().expect("tempdir");
    let call = WorkflowV2HostCall {
        id: "impl".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions::default(),
    };
    let plan = WorkflowV2WritePlanner::new(temp.path())
        .plan(&[WorkflowV2WriteItem::new(
            "impl-T001",
            WorkflowV2WriteMode::Worktree,
            vec!["src/lib.rs".to_string()],
        )])
        .expect("write plan");
    let mut branch_result = WorkflowV2Result::accepted("changed file");
    branch_result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "changed src/lib.rs in isolated worktree",
    ));
    branch_result
        .files_changed
        .push(WorkflowV2FileRecord::new("src/lib.rs"));
    branch_result.data = serde_json::json!({
        "item_id": "impl-T001",
        "canonical_task_ids": ["TASK-TDL-001"],
    });

    let result = result_from_write_fanout(&call, vec![branch_result], &plan, 1, None);

    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(
        result
            .data
            .get("serial_fallback_reason")
            .and_then(serde_json::Value::as_str),
        None
    );
    assert!(result.evidence.iter().any(|evidence| {
        evidence
            .summary
            .contains("write-capable fanout used Worktree")
    }));
}

#[test]
fn owned_module_scope_allows_new_child_file_but_not_sibling() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src/data_store")).expect("module dir");
    std::fs::write(repo.join("src/data_store.rs"), "mod io;\n").expect("module");
    std::fs::write(repo.join("src/data_store/io.rs"), "").expect("child");
    let repo_root = repo.display().to_string();
    let expanded = expand_declared_rust_module_targets(
        "impl-data-store",
        &["src/data_store.rs".to_string()],
        Some(&repo_root),
    )
    .expect("expanded");
    let item = WorkflowV2WriteItem::new(
        "impl-data-store",
        WorkflowV2WriteMode::Worktree,
        expanded.target_files,
    )
    .with_owned_scopes(expanded.target_dir_scopes);
    let mut result = WorkflowV2Result::accepted("created module child");
    result
        .files_changed
        .push(WorkflowV2FileRecord::new("src/data_store/new_tests.rs"));

    validate_changed_files_for_repository(&item, &result, Some(&repo_root))
        .expect("owned module directory permits new child files");

    result.files_changed = vec![WorkflowV2FileRecord::new("src/other/new_tests.rs")];
    match validate_changed_files_for_repository(&item, &result, Some(&repo_root)) {
        Err(crate::WorkflowV2WriteSafetyError::ChangedFileOutsideOwnership { path, .. }) => {
            assert_eq!(path, "src/other/new_tests.rs")
        }
        other => panic!("expected ChangedFileOutsideOwnership, got {other:?}"),
    }
}
