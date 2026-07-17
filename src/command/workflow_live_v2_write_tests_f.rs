#[test]
fn wf66_inherited_undeclared_path_evidence_does_not_block_isolated_worktree() {
    let result = wf66_preflight_result();

    assert!(
        result.is_none(),
        "historical undeclared-path evidence must not block a worktree-isolated retry"
    );
}

#[test]
fn declared_artifact_verifier_rejects_unsigned_branch_output() {
    let workspace = tempfile::tempdir().expect("workspace");
    let accepted = serde_json::json!({
        "item": {"artifact_verification_commands": ["test -f signed-artifact"]}
    });
    let rejected = run_declared_artifact_verifiers(&accepted, workspace.path())
        .expect_err("missing signed artifact must fail");
    assert!(rejected.contains("declared artifact verifier failed"));

    std::fs::write(workspace.path().join("signed-artifact"), "signed\n")
        .expect("signed fixture");
    run_declared_artifact_verifiers(&accepted, workspace.path())
        .expect("verified branch output");
}

fn wf66_preflight_result() -> Option<WorkflowV2Result> {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wf66_remediation_wave_1_3_source_preflight.json"
    ))
    .expect("fixture");
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).expect("repo src");
    std::fs::write(repo.join("src/shared.rs"), "pub fn shared() {}\n").expect("shared");
    std::fs::write(repo.join("src/feature_a.rs"), "pub fn feature() {}\n").expect("feature");
    std::fs::write(repo.join("src/large_tests.rs"), "x\n".repeat(501)).expect("large");
    let repo_root = repo.display().to_string();
    let call = WorkflowV2HostCall {
        id: fixture["call_id"].as_str().expect("call id").to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions {
            item_kind: Some("implementation".to_string()),
            target_files_from_item: true,
            ..WorkflowV2HostOptions::default()
        },
    };
    let branches = branches_from_fixture_items(&fixture, &call);
    let write_items = write_items_for_branches(Some(&repo_root), &call, &branches).unwrap();
    let plan = WorkflowV2WritePlanner::new(temp.path())
        .plan(&write_items)
        .expect("plan");

    preflight_write_fanout_source_contract(
        &call,
        &branches,
        &write_items,
        &plan,
        Some(&repo_root),
    )
}

#[test]
fn non_isolated_broad_duplicate_ownership_remains_preflight_data() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).expect("repo src");
    for path in ["src/shared.rs", "src/a.rs", "src/b.rs"] {
        std::fs::write(repo.join(path), "pub fn f() {}\n").expect("source");
    }
    let repo_root = repo.display().to_string();
    let call = WorkflowV2HostCall {
        id: "implementation-wave".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Coordinated),
        options: WorkflowV2HostOptions {
            item_kind: Some("implementation".to_string()),
            target_files_from_item: true,
            ..WorkflowV2HostOptions::default()
        },
    };
    let targets = serde_json::json!(["src/shared.rs", "src/a.rs", "src/b.rs"]);
    let branches = vec![
        branch_for_item(&call, "item-a", targets.clone()),
        branch_for_item(&call, "item-b", targets),
    ];
    let write_items = write_items_for_branches(Some(&repo_root), &call, &branches).unwrap();
    let plan = WorkflowV2WritePlanner::new(temp.path()).plan(&write_items).unwrap();

    let result =
        preflight_write_fanout_source_contract(&call, &branches, &write_items, &plan, Some(&repo_root))
            .expect("preflight result");

    let issues = result.data["source_preflight_issues"].as_array().unwrap();
    assert!(issues.iter().any(|issue| issue["kind"] == "duplicate_broad_ownership"));
}

#[test]
fn distinct_small_write_items_pass_source_preflight() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).expect("repo src");
    std::fs::write(repo.join("src/a.rs"), "pub fn a() {}\n").expect("a");
    std::fs::write(repo.join("src/b.rs"), "pub fn b() {}\n").expect("b");
    let repo_root = repo.display().to_string();
    let call = WorkflowV2HostCall {
        id: "implementation-wave".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions {
            item_kind: Some("implementation".to_string()),
            target_files_from_item: true,
            ..WorkflowV2HostOptions::default()
        },
    };
    let branches = vec![
        branch_for_item(&call, "item-a", serde_json::json!(["src/a.rs"])),
        branch_for_item(&call, "item-b", serde_json::json!(["src/b.rs"])),
    ];
    let write_items = write_items_for_branches(Some(&repo_root), &call, &branches).unwrap();
    let plan = WorkflowV2WritePlanner::new(temp.path())
        .plan(&write_items)
        .expect("plan");

    assert!(preflight_write_fanout_source_contract(
        &call,
        &branches,
        &write_items,
        &plan,
        Some(&repo_root),
    )
    .is_none());
}

#[test]
fn owned_oversized_existing_target_does_not_block_source_preflight() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).expect("repo src");
    std::fs::write(repo.join("src/large_tests.rs"), "x\n".repeat(955)).expect("large");
    let repo_root = repo.display().to_string();
    let call = WorkflowV2HostCall {
        id: "implementation-wave".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions {
            item_kind: Some("implementation".to_string()),
            target_files_from_item: true,
            ..WorkflowV2HostOptions::default()
        },
    };
    let branches = vec![branch_for_item(
        &call,
        "item-large-test-split",
        serde_json::json!(["src/large_tests.rs"]),
    )];
    let write_items = write_items_for_branches(Some(&repo_root), &call, &branches).unwrap();
    let plan = WorkflowV2WritePlanner::new(temp.path())
        .plan(&write_items)
        .expect("plan");

    assert!(preflight_write_fanout_source_contract(
        &call,
        &branches,
        &write_items,
        &plan,
        Some(&repo_root),
    )
    .is_none());
}

fn branches_from_fixture_items(
    fixture: &serde_json::Value,
    call: &WorkflowV2HostCall,
) -> Vec<WorkflowV2FanoutItem> {
    fixture["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| {
            let id = item["item_id"].as_str().expect("item id");
            WorkflowV2FanoutItem::read_only(
                id,
                "coder",
                call.clone(),
                serde_json::json!({"item": item}),
            )
        })
        .collect()
}

fn branch_for_item(
    call: &WorkflowV2HostCall,
    item_id: &str,
    target_files: serde_json::Value,
) -> WorkflowV2FanoutItem {
    WorkflowV2FanoutItem::read_only(
        item_id,
        "coder",
        call.clone(),
        serde_json::json!({
            "item": {
                "item_id": item_id,
                "canonical_task_ids": [format!("TASK-{item_id}")],
                "target_files": target_files
            }
        }),
    )
}
