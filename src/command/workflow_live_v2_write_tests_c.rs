#[test]
fn write_fanout_review_branch_stays_needs_review_not_failed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let call = WorkflowV2HostCall {
        id: "impl".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Coordinated),
        options: WorkflowV2HostOptions::default(),
    };
    let plan = WorkflowV2WritePlanner::new(temp.path())
        .plan(&[WorkflowV2WriteItem::new(
            "impl-T001",
            WorkflowV2WriteMode::Coordinated,
            vec!["src/lib.rs".to_string()],
        )])
        .expect("write plan");
    let mut branch_result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "implementation needs remediation".to_string(),
        ..WorkflowV2Result::default()
    };
    branch_result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "reviewed implementation branch",
    ));

    let result = result_from_write_fanout(&call, vec![branch_result], &plan, 1, None);

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert!(result.summary.contains("workflow.js remediation"));
}

#[test]
fn write_fanout_failed_branch_returns_remediation_data_for_script() {
    let temp = tempfile::tempdir().expect("tempdir");
    let call = WorkflowV2HostCall {
        id: "impl".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Coordinated),
        options: WorkflowV2HostOptions::default(),
    };
    let plan = WorkflowV2WritePlanner::new(temp.path())
        .plan(&[WorkflowV2WriteItem::new(
            "impl-T001",
            WorkflowV2WriteMode::Coordinated,
            vec!["src/lib.rs".to_string()],
        )])
        .expect("write plan");
    let mut branch_result = WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: "focused implementation failed and needs another pass".to_string(),
        ..WorkflowV2Result::default()
    };
    branch_result.residual_gaps.push(WorkflowV2ResidualGap {
        id: "gap".to_string(),
        description: "implementation branch did not satisfy acceptance criteria".to_string(),
        severity: Some("high".to_string()),
    });

    let result = result_from_write_fanout(&call, vec![branch_result], &plan, 1, None);

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert!(result.summary.contains("workflow.js remediation"));
    assert!(!result.residual_gaps.is_empty());
    assert_eq!(result.data["items"][0]["status"], "failed");
}

#[test]
fn write_fanout_outcome_gap_evidence_uses_schema_kind() {
    let temp = tempfile::tempdir().expect("tempdir");
    let call = WorkflowV2HostCall {
        id: "impl".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Coordinated),
        options: WorkflowV2HostOptions::default(),
    };
    let plan = WorkflowV2WritePlanner::new(temp.path())
        .plan(&[WorkflowV2WriteItem::new(
            "impl-T001",
            WorkflowV2WriteMode::Coordinated,
            vec!["src/lib.rs".to_string()],
        )])
        .expect("write plan");
    let mut branch_result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "branch needs follow-up".to_string(),
        data: serde_json::json!({
            "item_id": "impl-T001",
            "canonical_task_ids": ["TASK-001"]
        }),
        ..WorkflowV2Result::default()
    };
    branch_result.residual_gaps.push(WorkflowV2ResidualGap {
        id: "missing-evidence".to_string(),
        description: "branch did not provide concrete evidence".to_string(),
        severity: Some("review".to_string()),
    });

    let result = result_from_write_fanout(&call, vec![branch_result], &plan, 1, None);
    let evidence = result.data["outcomes"][0]["evidence"].clone();
    let parsed: Vec<WorkflowV2Evidence> = serde_json::from_value(evidence).expect("evidence schema");

    assert!(parsed.iter().any(|item| {
        item.kind == WorkflowV2EvidenceKind::Review && item.summary.contains("missing-evidence")
    }));
}

#[test]
fn write_branch_validation_error_becomes_branch_error_data() {
    let result = write_branch_validation_error_result(
        "impl-T001",
        None,
        "schema repair failed after one retry: first=implementation agent changed files outside declared target_files; repair=implementation noop requires typed task_coverage evidence",
    );

    assert_eq!(result.status, WorkflowV2Status::Failed);
    assert_eq!(
        result.residual_gaps[0].severity.as_deref(),
        Some("blocking")
    );
    assert_eq!(result.data["branch_error_from_runtime"], true);
}

#[test]
fn transport_error_is_not_reclassified_as_write_branch_validation_error() {
    assert!(!is_write_branch_validation_error(
        "agent transport failed: rate limit"
    ));
}

#[test]
fn write_branch_timeout_returns_js_visible_review_data() {
    let temp = tempfile::tempdir().expect("tempdir");
    let call = WorkflowV2HostCall {
        id: "implementation-wave".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions::default(),
    };
    let plan = WorkflowV2WritePlanner::new(temp.path())
        .plan(&[WorkflowV2WriteItem::new(
            "impl-task",
            WorkflowV2WriteMode::Worktree,
            vec!["src/lib.rs".to_string()],
        )])
        .expect("write plan");
    let branch_input = serde_json::json!({
        "item": {
            "item_id": "impl-task",
            "canonical_task_ids": ["TASK-001"]
        }
    });
    let branch_result = write_branch_runtime_timeout_result(
        "impl-task",
        &branch_input,
        "agent transport failed: subagent timed out after 7200s",
    );

    let result = result_from_write_fanout(&call, vec![branch_result], &plan, 1, None);

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(result.data["items"][0]["status"], "needs_review");
    assert_eq!(
        result.data["items"][0]["data"]["branch_runtime_timeout"],
        true
    );
    assert_eq!(
        result.data["items"][0]["data"]["canonical_task_ids"][0],
        "TASK-001"
    );
}

#[test]
fn unsafe_write_mode_errors_remain_safety_branch_errors() {
    for error in [
        "write target '/tmp/outside.rs' for item 'impl-T001' is unsafe",
        "write item 'impl-T001' changed undeclared path 'src/other.rs'",
    ] {
        assert!(is_write_branch_validation_error(error));
        assert_eq!(write_branch_error_kind(error), BranchFailureKind::Safety);
    }
}

#[test]
fn empty_patch_without_noop_is_contract_data_not_terminal_failure() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wfc022_empty_patch_no_noop_branch_failure.json"
    ))
    .expect("fixture");
    let error = fixture["old_error"].as_str().expect("old error");

    assert!(is_write_branch_validation_error(error));
    assert_eq!(write_branch_error_kind(error), BranchFailureKind::Contract);

    let result = write_branch_validation_error_result(
        fixture["branch_id"].as_str().expect("branch id"),
        None,
        error,
    );

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(result.residual_gaps[0].severity.as_deref(), Some("review"));
    assert_eq!(result.data["failure_kind"], "contract");
}

#[test]
fn wf139_runtime_project_artifact_is_not_repo_source_write() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wf139_project_artifact_write_false_safety.json"
    ))
    .expect("fixture");
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("runtime-project");
    let repo_root = temp.path().join("target-repo");
    let run_id = fixture["run_id"].as_str().expect("run id");
    let v2_root = project_root
        .join(".archon/workflows")
        .join(run_id)
        .join("v2");
    std::fs::create_dir_all(&v2_root).expect("v2 root");
    std::fs::create_dir_all(repo_root.join("src")).expect("repo src");
    let artifact_path = fixture["reported_changed_file"]
        .as_str()
        .expect("reported changed file");
    std::fs::write(project_root.join(artifact_path), "{}").expect("artifact");
    assert!(
        fixture["old_error"]
            .as_str()
            .expect("old error")
            .contains("outside declared target_files")
    );

    let request = archon_workflow::WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: fixture["branch_id"]
                .as_str()
                .expect("branch id")
                .to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "write runtime project artifact".to_string(),
        constraints: Vec::new(),
        input: serde_json::Value::Null,
        repository_root: Some(repo_root.display().to_string()),
        project_artifacts: archon_workflow::project_artifact_context_from_v2_root(&v2_root),
        target_files: vec!["src/lib.rs".to_string()],
    };
    let mut branch_result = WorkflowV2Result::accepted("wrote project artifact");
    branch_result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "created the runtime project gap audit artifact",
    ));
    branch_result
        .files_changed
        .push(WorkflowV2FileRecord::new(artifact_path));
    branch_result
        .task_coverage
        .push(archon_workflow::WorkflowV2TaskCoverage {
            task_id: "TASK-TDL-001".to_string(),
            status: WorkflowV2TaskCoverageStatus::Accepted,
            summary: "gap audit artifact created".to_string(),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Artifact,
                artifact_path,
            )],
        });
    branch_result.data = serde_json::json!({
        "item_id": fixture["branch_id"],
        "canonical_task_ids": fixture["source_item"]["canonical_task_ids"],
    });

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(
            &request,
            &serde_json::to_string(&branch_result).expect("json"),
        )
        .expect("project artifact accepted");

    assert!(parsed.files_changed.is_empty());
    assert_eq!(parsed.artifacts[0].path, artifact_path);
    let write_item = WorkflowV2WriteItem::new(
        fixture["branch_id"].as_str().expect("branch id"),
        WorkflowV2WriteMode::Coordinated,
        vec!["src/lib.rs".to_string()],
    );
    validate_changed_files_for_repository(
        &write_item,
        &parsed,
        Some(&repo_root.display().to_string()),
    )
    .expect("artifact-only branch keeps source ownership strict");
}

#[test]
fn missing_runtime_project_artifact_is_needs_review_not_safety() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wfe2c_missing_project_artifact_false_safety.json"
    ))
    .expect("fixture");
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("runtime-project");
    let repo_root = temp.path().join("target-repo");
    let run_id = fixture["run_id"].as_str().expect("run id");
    let v2_root = project_root
        .join(".archon/workflows")
        .join(run_id)
        .join("v2");
    std::fs::create_dir_all(&v2_root).expect("v2 root");
    std::fs::create_dir_all(repo_root.join("src")).expect("repo src");
    let artifact_path = fixture["reported_changed_file"]
        .as_str()
        .expect("reported changed file");
    assert!(!project_root.join(artifact_path).exists());
    assert!(
        fixture["old_error"]
            .as_str()
            .expect("old error")
            .contains("outside declared target_files")
    );

    let request = archon_workflow::WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: fixture["branch_id"]
                .as_str()
                .expect("branch id")
                .to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "write runtime project artifact".to_string(),
        constraints: Vec::new(),
        input: serde_json::Value::Null,
        repository_root: Some(repo_root.display().to_string()),
        project_artifacts: archon_workflow::project_artifact_context_from_v2_root(&v2_root),
        target_files: vec!["src/lib.rs".to_string()],
    };
    let mut branch_result = WorkflowV2Result::accepted("reported project artifact");
    branch_result
        .files_changed
        .push(WorkflowV2FileRecord::new(artifact_path));
    branch_result
        .task_coverage
        .push(archon_workflow::WorkflowV2TaskCoverage {
            task_id: "TASK-TDL-001".to_string(),
            status: WorkflowV2TaskCoverageStatus::Accepted,
            summary: "gap audit artifact reported".to_string(),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Artifact,
                artifact_path,
            )],
        });
    branch_result.data = serde_json::json!({
        "item_id": fixture["branch_id"],
        "canonical_task_ids": fixture["source_item"]["canonical_task_ids"],
    });

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(
            &request,
            &serde_json::to_string(&branch_result).expect("json"),
        )
        .expect("missing allowed project artifact is JS-visible review data");

    assert_eq!(parsed.status, WorkflowV2Status::NeedsReview);
    assert!(parsed.files_changed.is_empty());
    assert!(parsed.artifacts.is_empty());
    assert_eq!(parsed.data["canonical_task_ids"][0], "TASK-TDL-001");
    assert_eq!(parsed.task_coverage[0].task_id, "TASK-TDL-001");
    assert!(parsed.residual_gaps.iter().any(|gap| {
        gap.description.contains("missing project artifact")
            && gap.description.contains(artifact_path)
    }));
}

#[test]
fn missing_declared_project_artifact_does_not_count_as_evidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("runtime-project");
    let repo_root = temp.path().join("target-repo");
    let run_id = "wf-missing-artifact";
    let v2_root = project_root
        .join(".archon/workflows")
        .join(run_id)
        .join("v2");
    std::fs::create_dir_all(&v2_root).expect("v2 root");
    std::fs::create_dir_all(repo_root.join("src")).expect("repo src");
    let artifact_path = ".archon/workflows/wf-missing-artifact/artifacts/missing.json";

    let request = archon_workflow::WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "impl-artifact".to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "write runtime project artifact".to_string(),
        constraints: Vec::new(),
        input: serde_json::Value::Null,
        repository_root: Some(repo_root.display().to_string()),
        project_artifacts: archon_workflow::project_artifact_context_from_v2_root(&v2_root),
        target_files: vec!["src/lib.rs".to_string()],
    };
    let mut branch_result = WorkflowV2Result::accepted("claimed project artifact");
    branch_result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Artifact,
        artifact_path,
    ));
    branch_result
        .artifacts
        .push(archon_workflow::WorkflowV2Artifact {
            id: "missing".to_string(),
            path: artifact_path.to_string(),
            description: None,
        });

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(
            &request,
            &serde_json::to_string(&branch_result).expect("json"),
        )
        .expect("missing artifact path is review data");

    assert_eq!(parsed.status, WorkflowV2Status::NeedsReview);
    assert!(parsed.artifacts.is_empty());
    assert!(parsed.residual_gaps.iter().any(|gap| {
        gap.description.contains("missing project artifact")
            && gap.description.contains(artifact_path)
    }));
}

#[test]
fn write_branch_input_hash_includes_project_artifact_policy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let v2_root = temp
        .path()
        .join("runtime-project/.archon/workflows/wf-test/v2");
    std::fs::create_dir_all(&v2_root).expect("v2 root");
    let v2_store = archon_workflow::WorkflowV2ResultStore::new(&v2_root);
    let call = WorkflowV2HostCall {
        id: "implementation-wave-test".to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Coordinated),
        options: WorkflowV2HostOptions::default(),
    };
    let branch = WorkflowV2FanoutItem::read_only(
        "impl-test",
        "coder",
        call,
        serde_json::json!({"item": {"target_files": ["src/lib.rs"]}}),
    );
    let old_hash = branch.input_hash();

    let stamped = stamp_project_artifact_policy(vec![branch], &v2_store);

    assert_ne!(old_hash, stamped[0].input_hash());
    let policy = &stamped[0].input["_workflow_project_artifact_policy"];
    assert_eq!(
        policy["version"],
        archon_workflow::PROJECT_ARTIFACT_POLICY_VERSION
    );
    assert!(
        policy["project_root"]
            .as_str()
            .expect("project root")
            .ends_with("runtime-project")
    );
}
