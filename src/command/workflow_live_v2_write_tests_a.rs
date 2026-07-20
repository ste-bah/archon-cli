    use archon_workflow::{
        WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FanoutItem, WorkflowV2FileRecord,
        WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2SourceTaskGraph,
        WorkflowV2SourceTaskItem, validate_changed_files_for_repository,
    };

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

        let targets =
            target_files_for_branch(Some(&repo_root), &call, &branch).expect("target files");

        assert_eq!(targets, vec!["src/lib.rs"]);
    }

    #[test]
    fn artifact_only_review_remediation_can_launch_without_repo_targets() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_root = temp.path().display().to_string();
        let item: serde_json::Value = serde_json::from_str(include_str!(
            "fixtures/review_remediation_artifact_only_item.json"
        ))
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

        let targets =
            target_files_for_branch(Some(&repo_root), &call, &branch).expect("target files");

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

        let error = target_files_for_branch(Some(&repo_root), &call, &branch)
            .expect_err("repo root target");

        assert!(error.to_string().contains("unsafe"));
    }

    #[test]
    fn absolute_item_target_inside_repository_is_made_relative() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("crates/example/src")).expect("repo");
        let repo_root = repo.display().to_string();
        let call = WorkflowV2HostCall {
            id: "impl".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Serial),
            options: WorkflowV2HostOptions {
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        };
        let target = repo.join("crates/example/src/lib.rs");
        let branch = WorkflowV2FanoutItem::read_only(
            "impl-T001",
            "coder",
            call.clone(),
            serde_json::json!({
                "item": {
                    "id": "T001",
                    "target_files": [target.display().to_string()]
                }
            }),
        );

        let targets =
            target_files_for_branch(Some(&repo_root), &call, &branch).expect("target files");

        assert_eq!(targets, vec!["crates/example/src/lib.rs"]);
    }

    #[test]
    fn rust_module_child_declared_by_owned_target_is_owned_for_write_branch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("crates/archon-trading/src/data_store"))
            .expect("module dir");
        std::fs::write(
            repo.join("crates/archon-trading/src/data_store.rs"),
            "mod io;\nmod missing;\n",
        )
        .expect("data_store");
        std::fs::write(repo.join("crates/archon-trading/src/data_store/io.rs"), "").expect("io");
        let repo_root = repo.display().to_string();
        let call = WorkflowV2HostCall {
            id: "implementation-wave-1".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions {
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        };
        let branch = WorkflowV2FanoutItem::read_only(
            "implementation-wave-1-inventory-tdl-010-registry-schema-v2",
            "coder",
            call.clone(),
            serde_json::json!({
                "item": {
                    "id": "inventory-tdl-010-registry-schema-v2",
                    "target_files": ["crates/archon-trading/src/data_store.rs"]
                }
            }),
        );

        let targets =
            target_files_for_branch(Some(&repo_root), &call, &branch).expect("target files");

        assert!(targets.contains(&"crates/archon-trading/src/data_store.rs".to_string()));
        assert!(targets.contains(&"crates/archon-trading/src/data_store/io.rs".to_string()));

        let write_item =
            WorkflowV2WriteItem::new(branch.id, WorkflowV2WriteMode::Coordinated, targets);
        let mut result = WorkflowV2Result::accepted("changed declared module child");
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Implementation,
            "changed data_store/io.rs through data_store.rs module ownership",
        ));
        result.files_changed.push(WorkflowV2FileRecord::new(
            "crates/archon-trading/src/data_store/io.rs",
        ));

        validate_changed_files_for_repository(&write_item, &result, Some(&repo_root))
            .expect("declared module child is owned");
    }

    #[test]
    fn wf98_module_child_ownership_fixture_no_longer_reports_undeclared_path() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "fixtures/wf98_implementation_wave_1_module_child_ownership.json"
        ))
        .expect("fixture");
        let repo_root = env!("CARGO_MANIFEST_DIR").to_string();
        let branch_id = fixture["branch_id"].as_str().expect("branch id");
        let call = WorkflowV2HostCall {
            id: fixture["call_id"].as_str().expect("call id").to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions {
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        };
        let branch = WorkflowV2FanoutItem::read_only(
            branch_id,
            "coder",
            call.clone(),
            serde_json::json!({
                "item": fixture["source_item"].clone()
            }),
        );
        assert!(
            fixture["old_error"]
                .as_str()
                .expect("old error")
                .contains("changed undeclared path 'crates/archon-trading/src/data_store/io.rs'")
        );

        let targets =
            target_files_for_branch(Some(&repo_root), &call, &branch).expect("target files");

        assert!(targets.contains(&"crates/archon-trading/src/data_store.rs".to_string()));
        assert!(targets.contains(&"crates/archon-trading/src/data_store/io.rs".to_string()));
        let write_item =
            WorkflowV2WriteItem::new(branch_id, WorkflowV2WriteMode::Coordinated, targets);
        let mut result = WorkflowV2Result::accepted("changed module child");
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Implementation,
            "changed data_store/io.rs through declared module ownership",
        ));
        result.files_changed.push(WorkflowV2FileRecord::new(
            fixture["changed_file"].as_str().expect("changed file"),
        ));

        validate_changed_files_for_repository(&write_item, &result, Some(&repo_root))
            .expect("wf98 module child is owned");
    }

    #[test]
    fn absolute_item_target_outside_repository_is_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let repo_root = repo.display().to_string();
        let call = WorkflowV2HostCall {
            id: "impl".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Serial),
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
                    "id": "T001",
                    "target_files": [temp.path().join("other/src/lib.rs").display().to_string()]
                }
            }),
        );

        let error =
            target_files_for_branch(Some(&repo_root), &call, &branch).expect_err("outside repo");

        assert!(error.to_string().contains("unsafe"));
    }

    #[test]
    fn wf98_false_safety_fixture_accepts_declared_absolute_in_repo_change() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "fixtures/wf98_implementation_wave_2_false_safety.json"
        ))
        .expect("fixture");
        let repo_root = fixture["repository_root"]
            .as_str()
            .expect("repository root");
        let branch_id = fixture["branch_id"].as_str().expect("branch id");
        let owned_targets = fixture["assignment"]["owned_targets"]
            .as_array()
            .expect("owned targets")
            .iter()
            .map(|value| value.as_str().expect("target").to_string())
            .collect::<Vec<_>>();
        let absolute_changed_file = fixture["source_item"]["target_files"][0]
            .as_str()
            .expect("changed file");
        assert!(
            fixture["old_error"]
                .as_str()
                .expect("old error")
                .contains("outside declared target_files")
        );
        let write_item =
            WorkflowV2WriteItem::new(branch_id, WorkflowV2WriteMode::Coordinated, owned_targets);
        let mut result = WorkflowV2Result::accepted("changed declared target");
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Implementation,
            "changed declared TASK-TDL-020 target",
        ));
        result
            .files_changed
            .push(WorkflowV2FileRecord::new(absolute_changed_file));

        validate_changed_files_for_repository(&write_item, &result, Some(repo_root))
            .expect("absolute declared in-repo change is owned");
    }

    #[test]
    fn write_fanout_result_records_serial_fallback_reason() {
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
            "changed src/lib.rs",
        ));
        branch_result
            .files_changed
            .push(WorkflowV2FileRecord::new("src/lib.rs"));
        branch_result.data = serde_json::json!({
            "item_id": "impl-T001",
            "canonical_task_ids": ["TASK-TDL-001"],
        });

        let result = result_from_write_fanout(
            &call,
            vec![branch_result],
            &plan,
            1,
            Some("workspace boundary support is unavailable; serialized fallback used".to_string()),
        );

        assert_eq!(result.status, WorkflowV2Status::Accepted);
        assert_eq!(
            result
                .data
                .get("serial_fallback_reason")
                .and_then(serde_json::Value::as_str),
            Some("workspace boundary support is unavailable; serialized fallback used")
        );
    }
