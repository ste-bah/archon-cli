use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_decomposed_lifecycle_normalizes_reclassified_ids_and_reaches_terminal() {
    let started = Instant::now();
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).expect("repo src");
    for path in [
        "src/refuted.rs",
        "src/plain.rs",
        "src/contract.rs",
        "src/parameterized.rs",
    ] {
        std::fs::write(repo.join(path), "// pending\n").expect("seed source");
    }
    init_git_repo(&repo);
    std::fs::create_dir_all(temp.path().join(".archon/artifacts")).expect("artifact root");
    std::fs::write(
        temp.path().join(".archon/artifacts/example-contract.json"),
        r#"{"status":"ready","records":[{"id":"example","value":1}]}"#,
    )
    .expect("stub artifact");
    std::fs::write(
        temp.path().join(".archon/artifacts/instances.json"),
        r#"{"records":{}}"#,
    )
    .expect("empty instance source");

    let spec = WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "d64-lifecycle-e2e".to_string(),
        task: "Run a neutral decomposed lifecycle fixture.".to_string(),
        target_repository_root: Some(repo.display().to_string()),
        max_parallelism: 4,
        max_agents: 16,
        stages: Vec::new(),
        permissions: BTreeMap::new(),
        learning_hooks: Vec::new(),
    };
    let workflow_store = WorkflowStore::new(temp.path().join(".archon/workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let llm = Arc::new(CannedLifecycleLlm {
        scenario: CannedLifecycleScenario::FullLifecycle,
        calls: Mutex::new(Vec::new()),
        deliverable_contract_executed: AtomicBool::new(false),
        parameterized_contract_executed: AtomicBool::new(false),
        inventory_calls: AtomicUsize::new(0),
        verification_failure_emitted: AtomicBool::new(false),
    });
    let client = LiveV2AgentClient::new(
        llm.clone(),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        Some(repo.display().to_string()),
        Some(30),
    );
    let runtime = WorkflowV2ScriptRuntime {
        target_repository_root: spec.target_repository_root.clone(),
        generated_config: archon_core::config::GeneratedWorkflowConfig {
            max_repair_iterations: 1,
            max_investigation_iterations: 1,
            verification_branch_timeout_secs: 30,
            host_call_timeout_secs: 30,
            implementation_wave_max_parallelism: None,
        },
    };
    let runner = WorkflowV2ScriptRunner::new(
        spec.task.clone(),
        runtime,
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id,
        true,
        Some(synthetic_task_universe(temp.path())),
        None,
    );

    let summary = tokio::time::timeout(
        Duration::from_secs(120),
        runner.run_decomposed_lifecycle(
            "# Archon decomposed-PRD workflow (native lifecycle e2e fixture)",
            serde_json::json!([]),
        ),
    )
    .await
    .expect("lifecycle harness timeout")
    .expect("lifecycle summary");

    assert_eq!(
        summary.status,
        WorkflowV2Status::Accepted,
        "failed_call={:?} next_action={:?} calls={:?} llm_calls={:?}",
        summary.failed_call,
        summary.next_action,
        summary
            .calls
            .iter()
            .map(|call| call.id.as_str())
            .collect::<Vec<_>>(),
        llm.calls.lock().expect("calls lock"),
    );
    assert!(
        summary
            .calls
            .iter()
            .any(|call| call.id.starts_with("target-file-discovery-wave-1-"))
    );
    assert!(
        summary
            .calls
            .iter()
            .any(|call| call.id == "implementation-wave-1")
    );
    assert!(
        summary
            .calls
            .iter()
            .any(|call| call.id == "verification-wave-1"),
        "accepted implementation with a prefix-stripped ID was not scheduled for verification"
    );
    assert!(
        summary
            .calls
            .iter()
            .any(|call| call.id.starts_with("verification-failure-triage-")
                && call.id.ends_with("-shape-repair-1")),
        "empty triage routes did not trigger bounded shape repair"
    );
    assert!(
        summary
            .calls
            .iter()
            .any(|call| call.id.starts_with("verification-remediation-inventory-")),
        "shape-repaired triage route did not schedule remediation"
    );
    assert!(
        summary
            .calls
            .iter()
            .any(|call| call.id.starts_with("verification-wave-1-post-remediation-")),
        "remediation did not proceed to focused verification"
    );
    assert!(
        summary
            .calls
            .iter()
            .all(|call| !call.id.starts_with("blocked-"))
    );
    assert_eq!(
        summary
            .calls
            .iter()
            .filter(|call| call.id.starts_with("terminal-gate-reroute-"))
            .count(),
        1
    );
    assert_eq!(llm.inventory_calls.load(Ordering::SeqCst), 2);
    assert!(
        llm.deliverable_contract_executed.load(Ordering::SeqCst),
        "declared deliverable contract verification command did not execute"
    );
    assert!(
        llm.parameterized_contract_executed.load(Ordering::SeqCst),
        "vacuous source-backed parameterized contract did not traverse lifecycle verification"
    );
    assert!(
        std::fs::read_to_string(repo.join("src/refuted.rs"))
            .expect("refuted implementation")
            .contains("implemented_TASK_EX_002")
    );
    assert!(
        temp.path()
            .join(".archon/artifacts/artifact-only.json")
            .is_file()
    );

    let final_record = v2_store
        .load_call_record("final-acceptance-report")
        .expect("final record load")
        .expect("final record");
    assert_eq!(final_record.status, WorkflowV2Status::Accepted);
    let report = &final_record.result.data;
    assert_eq!(report["accepted_tasks"].as_array().map(Vec::len), Some(5));
    assert_eq!(report["noop_tasks"], serde_json::json!(["TASK-EX-001"]));
    assert!(report["failed_tasks"].as_array().is_some_and(Vec::is_empty));
    assert!(
        report["blocked_tasks"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    assert!(
        report["missing_tasks"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    assert!(
        started.elapsed() < Duration::from_secs(120),
        "harness took {:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_final_report_emits_host_built_fallback() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "final-report-fallback-e2e".to_string(),
        task: "Force the terminal report fallback path.".to_string(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        stages: Vec::new(),
        permissions: BTreeMap::new(),
        learning_hooks: Vec::new(),
    };
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-EX-FALLBACK".to_string(),
            source_path: "tasks/TASK-EX-FALLBACK.md".to_string(),
            acceptance_criteria: vec!["A terminal report is persisted.".to_string()],
            ..Default::default()
        }],
    };
    let workflow_store = WorkflowStore::new(temp.path().join(".archon/workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let llm = Arc::new(CannedLifecycleLlm {
        scenario: CannedLifecycleScenario::FullLifecycle,
        calls: Mutex::new(Vec::new()),
        deliverable_contract_executed: AtomicBool::new(false),
        parameterized_contract_executed: AtomicBool::new(false),
        inventory_calls: AtomicUsize::new(0),
        verification_failure_emitted: AtomicBool::new(false),
    });
    let client = LiveV2AgentClient::new(llm, ui_sink, Vec::new(), run.id.clone(), None, Some(30));
    let generated_config = archon_core::config::GeneratedWorkflowConfig {
        max_repair_iterations: 1,
        max_investigation_iterations: 1,
        verification_branch_timeout_secs: 30,
        host_call_timeout_secs: 30,
        implementation_wave_max_parallelism: None,
    };
    let runner = WorkflowV2ScriptRunner::new(
        spec.task,
        WorkflowV2ScriptRuntime {
            target_repository_root: None,
            generated_config: generated_config.clone(),
        },
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id,
        true,
        Some(universe.clone()),
        None,
    );
    let host = Arc::new(WorkflowScriptHost {
        scaffold_hash: workflow_scaffold_hash("# final report fallback fixture"),
        runner,
        accumulator: Arc::new(tokio::sync::Mutex::new(WorkflowScriptAccumulator::default())),
    });
    let driver = LifecycleDriver::new(
        host,
        universe,
        None,
        Some(temp.path().display().to_string()),
        serde_json::json!([]),
        Default::default(),
        &generated_config,
    );

    let result = driver
        .final_report(
            "forced-report-failure",
            None,
            "needs_review",
            serde_json::json!([{
                "status": "failed",
                "summary": "forced malformed reducer result",
                "commands_run": "not-a-sequence"
            }]),
            "Emit a terminal report even when reducer evidence is malformed.",
        )
        .await;

    assert!(
        result
            .expect_err("fallback report should terminate needs-review lifecycle")
            .to_string()
            .contains(TERMINAL_HOST_CALL_MARKER)
    );
    assert_eq!(
        v2_store
            .load_call_record("forced-report-failure")
            .expect("failed report record load")
            .expect("failed report record")
            .status,
        WorkflowV2Status::Failed
    );
    let fallback = v2_store
        .load_call_record("forced-report-failure-host-fallback")
        .expect("fallback record load")
        .expect("fallback record");
    assert_eq!(fallback.status, WorkflowV2Status::NeedsReview);
    assert_eq!(
        fallback.result.data["missing_tasks"],
        serde_json::json!(["TASK-EX-FALLBACK"])
    );
    assert!(fallback.result.artifacts.iter().any(|artifact| {
        artifact.id == "forced-report-failure-host-fallback"
            && std::path::Path::new(&artifact.path).is_file()
    }));
}

#[tokio::test]
async fn triage_shape_repair_cannot_trade_predicate_identity_for_better_accounting() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (driver, llm) = boundary_driver(&temp, CannedLifecycleScenario::TriagePreservation);
    let original = serde_json::json!({
        "implementation_failures": [{
            "item_id": "failed-one",
            "source_item_id": "source-one",
            "canonical_task_ids": ["TASK-EX-BOUNDARY"],
            "classification": "implementation_failure",
            "failed_predicate": "original predicate",
            "source_residual_gap_ids": ["gap-one"],
        }],
        "retry_items": [],
        "superseded_items": [],
        "terminal_blockers": [],
    });
    let failed_outcomes = vec![
        serde_json::json!({"item_id": "failed-one"}),
        serde_json::json!({"item_id": "failed-two"}),
    ];
    let mut evidence = LifecycleEvidence::default();

    let retained = driver
        .enforce_triage_accounting(
            "semantic-triage",
            &failed_outcomes,
            original.clone(),
            &mut evidence,
        )
        .await
        .expect("triage repair");

    let routes = lifecycle_policy::verify_routing::triage_routes(&retained);
    assert_eq!(routes.implementation_failures.len(), 1);
    assert!(routes.retry_items.is_empty());
    assert_eq!(
        routes.implementation_failures[0]["failed_predicate"],
        "original predicate"
    );
    assert!(evidence.repair_attempts.iter().any(|attempt| {
        attempt["call_id"] == "semantic-triage-shape-repair-1"
            && attempt["issue_kind"] == "semantic_preservation_rejected"
    }));
    assert_eq!(
        llm.calls.lock().expect("calls lock").as_slice(),
        ["semantic-triage-shape-repair-1"]
    );
    // D78: the rejection must persist as a monitor-visible typed record, not
    // only in in-memory repair-attempt evidence.
    assert!(
        persisted_semantic_rejection_record(temp.path(), "semantic-triage-shape-repair-1"),
        "expected a persisted semantic-preservation rejection record"
    );
}

fn persisted_semantic_rejection_record(root: &std::path::Path, repair_id: &str) -> bool {
    fn walk(dir: &std::path::Path, needle: &str) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if walk(&path, needle) {
                    return true;
                }
            } else if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(needle))
            {
                return true;
            }
        }
        false
    }
    walk(root, &format!("{repair_id}-semantic-preservation-rejected"))
}
