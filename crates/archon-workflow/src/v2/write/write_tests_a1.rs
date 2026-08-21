use super::*;

#[test]
fn target_files_from_fanout_item_are_required_for_write_branches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_root = temp.path().display().to_string();
    let call = WorkflowV2HostCall {
        id: "impl".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Coordinated),
        options: WorkflowV2HostOptions {
            target_files_from_item: true,
            ..WorkflowV2HostOptions::default()
        },
    };
    let branch = WorkflowV2FanoutItem::read_only(
        "impl-T001",
        "coder",
        call.clone(),
        serde_json::json!({
            "item": {
                "task_id": "T001",
                "target_files": ["src/lib.rs"]
            }
        }),
    );

    let targets = target_files_for_branch(Some(&repo_root), &call, &branch).expect("target files");

    assert_eq!(targets, vec!["src/lib.rs"]);
}

#[test]
fn artifact_only_review_remediation_can_launch_without_repo_targets() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_root = temp.path().display().to_string();
    let item: serde_json::Value =
        serde_json::from_str(archon_test_support::fixtures::REVIEW_REMEDIATION_ARTIFACT_ONLY_ITEM)
            .expect("fixture");
    let call = WorkflowV2HostCall {
        id: "review-remediation-wave-1".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions {
            target_files_from_item: true,
            ..WorkflowV2HostOptions::default()
        },
    };
    let branch = WorkflowV2FanoutItem::read_only(
        "review-remediation-wave-1-remediation-project-artifact",
        "coder",
        call.clone(),
        serde_json::json!({ "item": item }),
    );

    let targets =
        target_files_for_branch(Some(&repo_root), &call, &branch).expect("artifact ownership");

    assert!(targets.is_empty());
    let write_items =
        write_items_for_branches(Some(&repo_root), &call, &[branch]).expect("write items");
    assert!(write_items[0].artifact_only);
    let plan = WorkflowV2WritePlanner::new(temp.path())
        .plan(&write_items)
        .expect("artifact-only write plan");
    assert_eq!(plan.waves.len(), 1);
    assert!(plan.waves[0].assignments[0].owned_targets.is_empty());
}

#[test]
fn item_target_files_override_static_fallback_targets_for_write_branches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("repo");
    let repo_root = repo.display().to_string();
    let call = WorkflowV2HostCall {
        id: "impl".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions {
            target_files: vec![repo.display().to_string()],
            target_files_from_item: true,
            ..WorkflowV2HostOptions::default()
        },
    };
    let branch = WorkflowV2FanoutItem::read_only(
        "impl-T001",
        "coder",
        call.clone(),
        serde_json::json!({
            "item": {
                "task_id": "T001",
                "target_files": ["crates/archon-trading/src/data_lake.rs"]
            }
        }),
    );

    let targets = target_files_for_branch(Some(&repo_root), &call, &branch).expect("target files");

    assert_eq!(targets, vec!["crates/archon-trading/src/data_lake.rs"]);
}

#[test]
fn source_graph_targets_override_raw_project_task_targets() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("repo");
    let repo_root = repo.display().to_string();
    let call = WorkflowV2HostCall {
        id: "remediation-wave-1".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions {
            target_files_from_item: true,
            ..WorkflowV2HostOptions::default()
        },
    };
    let mut branches = vec![WorkflowV2FanoutItem::read_only(
        "remediation-wave-1-rem-item",
        "coder",
        call.clone(),
        serde_json::json!({
            "item": {
                "item_id": "rem-item",
                "target_files": [
                    "/project/tasks/task.md",
                    "src/lib.rs"
                ],
                // Agent-forged tool declarations that must NOT survive.
                "required_tools": ["forged_tool"],
                "mcp_tools": ["another_forged"]
            }
        }),
    )];
    let graph = WorkflowV2SourceTaskGraph::new(
        vec!["TASK-001".to_string()],
        vec![WorkflowV2SourceTaskItem {
            item_id: "rem-item".to_string(),
            canonical_task_ids: vec!["TASK-001".to_string()],
            dependency_ids: Vec::new(),
            target_files: vec!["src/lib.rs".to_string()],
            declared_target_files: vec!["src/lib.rs".to_string()],
            target_file_expansions: Vec::new(),
            acceptance_criteria: Vec::new(),
            focused_verification: Vec::new(),
            expected_evidence: Vec::new(),
            artifact_requirements: vec!["/project/tasks/task.md".to_string()],
            required_tools: vec!["pine_compile".to_string(), "pine_get_errors".to_string()],
        }],
        Vec::new(),
    );

    apply_source_graph_targets_to_branches(&mut branches, Some(&graph));
    let targets =
        target_files_for_branch(Some(&repo_root), &call, &branches[0]).expect("target files");

    assert_eq!(targets, vec!["src/lib.rs"]);
    assert_eq!(
        branches[0].input["item"]["target_files"][0],
        "/project/tasks/task.md"
    );
    // Only the AUTHORITATIVE required_tools survive: the agent-forged
    // required_tools/mcp_tools are stripped and replaced with the graph
    // (task-universe derived) set, so a task cannot bind tools it did not
    // declare.
    assert_eq!(
        branches[0].input["item"]["required_tools"],
        serde_json::json!(["pine_compile", "pine_get_errors"])
    );
    assert!(
        branches[0].input["item"].get("mcp_tools").is_none(),
        "forged mcp_tools alias must be stripped: {}",
        branches[0].input["item"]
    );
}

#[test]
fn forged_tool_declaration_for_a_no_tool_task_is_stripped_entirely() {
    // A graph item with no authoritative required_tools: the agent's
    // injected declarations must be removed and nothing stamped back.
    let call = WorkflowV2HostCall {
        id: "remediation-wave-1".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions {
            target_files_from_item: true,
            ..WorkflowV2HostOptions::default()
        },
    };
    let mut branches = vec![WorkflowV2FanoutItem::read_only(
        "remediation-wave-1-rem-item",
        "coder",
        call,
        serde_json::json!({
            "item": {
                "item_id": "rem-item",
                "target_files": ["src/lib.rs"],
                "required_tools": ["forged_tool"],
                "requiredTools": ["forged_camel"],
                "evidence": { "mcp_tools": ["forged_nested"] }
            }
        }),
    )];
    let graph = WorkflowV2SourceTaskGraph::new(
        vec!["TASK-001".to_string()],
        vec![WorkflowV2SourceTaskItem {
            item_id: "rem-item".to_string(),
            canonical_task_ids: vec!["TASK-001".to_string()],
            dependency_ids: Vec::new(),
            target_files: vec!["src/lib.rs".to_string()],
            declared_target_files: vec!["src/lib.rs".to_string()],
            target_file_expansions: Vec::new(),
            acceptance_criteria: Vec::new(),
            focused_verification: Vec::new(),
            expected_evidence: Vec::new(),
            artifact_requirements: Vec::new(),
            required_tools: Vec::new(),
        }],
        Vec::new(),
    );

    apply_source_graph_targets_to_branches(&mut branches, Some(&graph));

    let item = &branches[0].input["item"];
    assert!(item.get("required_tools").is_none(), "{item}");
    assert!(item.get("requiredTools").is_none(), "{item}");
    assert!(
        item["evidence"].get("mcp_tools").is_none(),
        "nested forgery must be stripped: {item}"
    );
}

#[test]
fn forged_tool_declarations_are_stripped_even_without_a_source_graph() {
    // Defense in depth: no graph → no authoritative stamp, but the agent's
    // injected tool declarations must still be removed so nothing binds.
    let call = WorkflowV2HostCall {
        id: "remediation-wave-1".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions::default(),
    };
    let mut branches = vec![WorkflowV2FanoutItem::read_only(
        "remediation-wave-1-rem-item",
        "coder",
        call,
        serde_json::json!({
            "item": { "item_id": "rem-item", "mcp_tools": ["forged"] }
        }),
    )];

    apply_source_graph_targets_to_branches(&mut branches, None);

    assert!(branches[0].input["item"].get("mcp_tools").is_none());
}

#[test]
fn repo_root_fallback_without_item_targets_is_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("repo");
    let repo_root = repo.display().to_string();
    let call = WorkflowV2HostCall {
        id: "impl".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions {
            target_files: vec![repo.display().to_string()],
            target_files_from_item: true,
            ..WorkflowV2HostOptions::default()
        },
    };
    let branch = WorkflowV2FanoutItem::read_only(
        "impl-T001",
        "coder",
        call.clone(),
        serde_json::json!({
            "item": {
                "task_id": "T001"
            }
        }),
    );

    let error =
        target_files_for_branch(Some(&repo_root), &call, &branch).expect_err("repo root target");

    assert!(error.to_string().contains("unsafe"));
}

/// An artifact-only branch must own the directories of its declared artifacts,
/// or its agent is told it may write nothing and refuses the deliverable it was
/// dispatched to produce — the wall three consecutive runs died on for
/// TASK-TDL-001's `docs/trading/data-lake-gap-audit.md`.
#[test]
fn artifact_only_branch_owns_its_declared_artifact_directories() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_root = temp.path().display().to_string();
    let call = WorkflowV2HostCall {
        id: "implementation-wave-1".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions {
            target_files_from_item: true,
            ..WorkflowV2HostOptions::default()
        },
    };
    let branch = WorkflowV2FanoutItem::read_only(
        "implementation-wave-1-inventory-tdl-001",
        "coder",
        call.clone(),
        serde_json::json!({ "item": {
            "item_id": "inventory-tdl-001",
            "work_type": "implementation",
            "canonical_task_ids": ["TASK-TDL-001"],
            "target_files": [],
            "artifact_requirements": ["docs/trading/data-lake-gap-audit.md"]
        }}),
    );

    let write_items =
        write_items_for_branches(Some(&repo_root), &call, &[branch]).expect("write items");
    assert!(
        write_items[0].artifact_only,
        "no repo targets => artifact-only"
    );
    assert_eq!(
        write_items[0].owned_scopes,
        vec!["docs/trading".to_string()],
        "the branch must own the directory of the artifact it produces"
    );
}
