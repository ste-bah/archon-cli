#[test]
fn blocked_final_report_preserves_prior_dynamic_wave_completion_evidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let upstream = execution("implementation-wave-1", WorkflowV2HostMethod::Fanout, None);
    let mut result = WorkflowV2Result::accepted("implementation wave accepted");
    result.task_coverage.push(WorkflowV2TaskCoverage {
        task_id: "TASK-TDL-010".to_string(),
        status: WorkflowV2TaskCoverageStatus::Noop,
        summary: "registry already implemented and verified".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Implementation,
            "focused registry evidence",
        )],
    });
    result.commands_run.push(WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: "cargo test registry_schema".to_string(),
        status: WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary: "passed".to_string(),
    });
    let evidence = WorkflowV2TaskCompletionEvidence::new(
        "TASK-TDL-010",
        WorkflowV2TaskCompletionEvidenceKind::VerifiedNoop,
        "implementation-wave-1",
        "impl-TASK-TDL-010",
        WorkflowV2Status::Noop,
    );
    store
        .save_call_record(
            &WorkflowV2CallRecord::new(
                store.run_id(),
                upstream.call,
                1,
                "hash".to_string(),
                result,
                Vec::new(),
            )
            .with_completion_evidence(vec![evidence]),
        )
        .expect("record");

    let blocked_source: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wf139e_blocked_verification_failed_source_result.json"
    ))
    .expect("fixture json");
    let final_report = WorkflowV2CallExecution {
        input: serde_json::json!({ "source_data": blocked_source }),
        ..execution(
            "blocked-verification-failed-1",
            WorkflowV2HostMethod::FinalReport,
            None,
        )
    };

    let report = execute_local_host_call(&final_report, &store, Some(&task_universe_010()))
        .expect("final")
        .expect("local result");

    assert_eq!(report.status, WorkflowV2Status::NeedsReview);
    assert!(
        report.data["missing_tasks"]
            .as_array()
            .map(|missing| {
                !missing
                    .iter()
                    .any(|value| value.as_str() == Some("TASK-TDL-010"))
            })
            .unwrap_or(true),
        "{:#?}",
        report.data
    );
    assert!(
        report.data["noop_tasks"]
            .as_array()
            .is_some_and(|tasks| tasks.iter().any(|value| value == "TASK-TDL-010")),
        "{:#?}",
        report.data
    );
    assert!(
        report.data["task_coverage"]
            .as_array()
            .is_some_and(|coverage| coverage.iter().any(|value| value["task_id"] == "TASK-TDL-010")),
        "{:#?}",
        report.data
    );
    assert!(
        report.data["commands_run"].as_array().is_some_and(|commands| {
            commands
                .iter()
                .any(|value| value["command"] == "cargo test registry_schema")
        }),
        "{:#?}",
        report.data
    );
    assert!(
        report.data["residual_gaps"]
            .as_array()
            .is_some_and(|gaps| gaps.iter().any(|gap| gap["id"].as_str()
                == Some("dynamic_wave_source_metadata_verification-wave-1-3"))),
        "{:#?}",
        report.data
    );
}

#[test]
fn final_report_accepts_repository_relative_focused_verification_artifacts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("project-1");
    let repo_root = temp.path().join("archon-cli");
    let v2_root = project_root
        .join(".archon")
        .join("workflows")
        .join("wf-test")
        .join("v2");
    std::fs::create_dir_all(v2_root.join("results")).expect("results dir");
    std::fs::create_dir_all(repo_root.join("src/cli_args")).expect("repo fixture dir");
    std::fs::write(repo_root.join("src/cli_args/tests.rs"), "// tests").expect("tests rs");
    std::fs::write(
        repo_root.join("src/cli_args/trading_market_actions.rs"),
        "// cli",
    )
    .expect("actions rs");
    std::fs::write(
        v2_root.parent().unwrap().join("state.json"),
        serde_json::json!({
            "spec": {
                "target_repository_root": repo_root
            }
        })
        .to_string(),
    )
    .expect("state json");
    let store = WorkflowV2ResultStore::new(&v2_root);

    let mut implementation_result = WorkflowV2Result::accepted("implementation accepted");
    implementation_result
        .task_coverage
        .push(WorkflowV2TaskCoverage {
            task_id: "TASK-TDL-010".to_string(),
            status: WorkflowV2TaskCoverageStatus::Accepted,
            summary: "implemented registry schema".to_string(),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Implementation,
                "changed registry files",
            )],
        });
    let mut implementation_evidence = WorkflowV2TaskCompletionEvidence::new(
        "TASK-TDL-010",
        WorkflowV2TaskCompletionEvidenceKind::ImplementationCandidate,
        "implementation-wave-1",
        "impl-TASK-TDL-010",
        WorkflowV2Status::Accepted,
    );
    implementation_evidence.evidence_refs = vec!["changed registry files".to_string()];
    store
        .save_call_record(
            &WorkflowV2CallRecord::new(
                store.run_id(),
                execution("implementation-wave-1", WorkflowV2HostMethod::Fanout, None).call,
                1,
                "impl-hash".to_string(),
                implementation_result,
                Vec::new(),
            )
            .with_completion_evidence(vec![implementation_evidence]),
        )
        .expect("implementation record");

    let mut verification_result = WorkflowV2Result::accepted("verification accepted");
    verification_result.commands_run.push(WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: "cargo test -p archon-cli-workspace cli_args::tests::trading_parse_tests::trading_data_prd_commands_parse -- --exact".to_string(),
        status: WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary: "1 passed; 0 failed".to_string(),
    });
    verification_result
        .artifacts
        .push(archon_workflow::WorkflowV2Artifact {
            id: "cli-args-tests".to_string(),
            path: "src/cli_args/tests.rs".to_string(),
            description: None,
        });
    verification_result
        .artifacts
        .push(archon_workflow::WorkflowV2Artifact {
            id: "trading-market-actions".to_string(),
            path: "src/cli_args/trading_market_actions.rs".to_string(),
            description: None,
        });
    verification_result
        .task_coverage
        .push(WorkflowV2TaskCoverage {
            task_id: "TASK-TDL-010".to_string(),
            status: WorkflowV2TaskCoverageStatus::Accepted,
            summary: "focused verification accepted".to_string(),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Test,
                "focused cargo test passed",
            )],
        });
    let mut verification_evidence = WorkflowV2TaskCompletionEvidence::new(
        "TASK-TDL-010",
        WorkflowV2TaskCompletionEvidenceKind::FocusedVerification,
        "verification-wave-1",
        "verify-TASK-TDL-010",
        WorkflowV2Status::Accepted,
    );
    verification_evidence.artifact_paths = vec![
        "src/cli_args/tests.rs".to_string(),
        "src/cli_args/trading_market_actions.rs".to_string(),
    ];
    verification_evidence.evidence_refs = vec!["focused cargo test passed".to_string()];
    store
        .save_call_record(
            &WorkflowV2CallRecord::new(
                store.run_id(),
                execution("verification-wave-1", WorkflowV2HostMethod::Parallel, None).call,
                1,
                "verify-hash".to_string(),
                verification_result,
                Vec::new(),
            )
            .with_completion_evidence(vec![verification_evidence]),
        )
        .expect("verification record");

    let final_report = execution(
        "final",
        WorkflowV2HostMethod::FinalReport,
        Some("[implementation-wave-1,verification-wave-1]"),
    );
    let report = execute_local_host_call(&final_report, &store, Some(&task_universe_010()))
        .expect("final")
        .expect("local result");

    assert_eq!(report.status, WorkflowV2Status::Accepted, "{report:#?}");
    assert_eq!(
        report.data["missing_tasks"].as_array().map(Vec::len),
        Some(0)
    );
    assert!(
        !report.data["residual_gaps"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .any(|gap| gap["id"].as_str() == Some("missing_evidence_artifact_TASK-TDL-010")),
        "{:#?}",
        report.data
    );
}
