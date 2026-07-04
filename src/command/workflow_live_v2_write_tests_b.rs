    #[test]
    fn worktree_assignment_builds_coordinator_plan_with_isolated_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).expect("repo");
        std::fs::write(repo.join("src/lib.rs"), "pub fn existing() {}\n").expect("file");
        let assignment = WorkflowV2WriteAssignment {
            item_id: "impl-T001".to_string(),
            owned_targets: vec!["src/lib.rs".to_string()],
            worktree_path: Some(temp.path().join("wt").display().to_string()),
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
