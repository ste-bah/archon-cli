#[test]
fn unresolved_branch_reports_generic_ownership_expansion_need() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wfab880_ownership_expansion_needed.json"
    ))
    .expect("fixture");
    let temp = tempfile::tempdir().expect("tempdir");
    let call = WorkflowV2HostCall {
        id: fixture["call_id"].as_str().expect("call id").to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions::default(),
    };
    let branch_id = fixture["branch_id"].as_str().expect("branch id");
    let targets = fixture["source_item"]["target_files"]
        .as_array()
        .expect("targets")
        .iter()
        .map(|value| value.as_str().expect("target").to_string())
        .collect::<Vec<_>>();
    let plan = WorkflowV2WritePlanner::new(temp.path())
        .plan(&[WorkflowV2WriteItem::new(
            branch_id,
            WorkflowV2WriteMode::Worktree,
            targets,
        )])
        .expect("write plan");
    let branch_result: WorkflowV2Result =
        serde_json::from_value(fixture["branch_result"].clone()).expect("branch result");

    let result = result_from_write_fanout(&call, vec![branch_result], &plan, 1, None);

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    let item = &result.data["items"][0];
    assert_eq!(item["data"]["ownership_expansion_required"], true);
    assert_eq!(
        item["data"]["proposed_ownership_expansions"][0]["path"],
        fixture["expected_missing_ownership_path"]
    );
    assert_eq!(
        item["data"]["proposed_ownership_expansions"][0]["role"],
        "source"
    );
}

#[test]
fn ownership_expansion_does_not_propose_artifact_or_owned_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let call = WorkflowV2HostCall {
        id: "remediation-wave".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions::default(),
    };
    let plan = WorkflowV2WritePlanner::new(temp.path())
        .plan(&[WorkflowV2WriteItem::new(
            "remediation-item",
            WorkflowV2WriteMode::Worktree,
            vec!["src/lib.rs".to_string()],
        )])
        .expect("write plan");
    let mut branch_result = WorkflowV2Result {
        status: WorkflowV2Status::Blocked,
        summary: "Read src/lib.rs and wrote .archon/workflows/run/artifacts/out.json".to_string(),
        ..WorkflowV2Result::default()
    };
    branch_result.evidence.push(WorkflowV2Evidence {
        kind: WorkflowV2EvidenceKind::Review,
        summary: "src/lib.rs is already owned; raw/request.json and raw/headers.redacted.json are artifact contract leaves, not source ownership".to_string(),
        source: Some("src/lib.rs".to_string()),
    });
    branch_result
        .commands_run
        .push(archon_workflow::WorkflowV2CommandRecord {
            kind: archon_workflow::WorkflowV2CommandKind::Test,
            command: "focused command".to_string(),
            status: archon_workflow::WorkflowV2CommandStatus::Failed,
            exit_code: Some(1),
            output_summary: ".archon/workflows/run/artifacts/out.json".to_string(),
        });
    branch_result.data = serde_json::json!({
        "item_id": "remediation-item",
        "canonical_task_ids": ["TASK-001"]
    });

    let result = result_from_write_fanout(&call, vec![branch_result], &plan, 1, None);

    assert_ne!(
        result.data["items"][0]["data"]["ownership_expansion_required"],
        true
    );
}

#[test]
fn ownership_expansion_ignores_artifact_gap_without_ownership_context() {
    let temp = tempfile::tempdir().expect("tempdir");
    let call = WorkflowV2HostCall {
        id: "remediation-wave".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions::default(),
    };
    let plan = WorkflowV2WritePlanner::new(temp.path())
        .plan(&[WorkflowV2WriteItem::new(
            "remediation-item",
            WorkflowV2WriteMode::Worktree,
            vec!["src/lib.rs".to_string()],
        )])
        .expect("write plan");
    let mut branch_result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "Missing artifact contract leaves: raw/request.json, raw/provider-notes.md"
            .to_string(),
        ..WorkflowV2Result::default()
    };
    branch_result.residual_gaps.push(WorkflowV2ResidualGap {
        id: "missing_artifact_contract".to_string(),
        description: "Missing raw/request.json and raw/provider-notes.md artifact evidence"
            .to_string(),
        severity: Some("review".to_string()),
    });
    branch_result.data = serde_json::json!({
        "item_id": "remediation-item",
        "canonical_task_ids": ["TASK-001"]
    });

    let result = result_from_write_fanout(&call, vec![branch_result], &plan, 1, None);

    assert_ne!(
        result.data["items"][0]["data"]["ownership_expansion_required"],
        true
    );
}
