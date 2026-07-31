#[tokio::test]
async fn repair_plan_shape_repair_cannot_drop_source_gap_identity() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (driver, llm) = boundary_driver(&temp, CannedLifecycleScenario::RepairPlanPreservation);
    let post_plan = serde_json::json!({
        "items": [{
            "item_id": "retry-check",
            "source_item_id": "source-check",
            "canonical_task_ids": ["TASK-EX-BOUNDARY"],
            "classification": "retryable_verification_shape_issue",
            "failed_predicate": "focused check passes",
            "source_residual_gap_ids": ["gap-check"],
            "focused_verification": "cargo test focused_check -- --exact",
        }],
        "unresolved_issues": [{
            "kind": "inventory_shape_repair",
            "field": "items",
            "message": "repair the plan shape",
        }],
    });
    let mut evidence = LifecycleEvidence::default();

    let retained = driver
        .repair_post_remediation_plan_once(
            &serde_json::json!({"items": []}),
            &serde_json::json!({"outcomes": []}),
            post_plan,
            1,
            &1,
            1,
            &mut evidence,
        )
        .await
        .expect("post-remediation plan repair");

    assert_eq!(
        retained["items"][0]["source_residual_gap_ids"],
        serde_json::json!(["gap-check"])
    );
    assert!(
        retained["unresolved_issues"]
            .as_array()
            .is_some_and(|issues| issues.iter().any(|issue| {
                issue["kind"] == "semantic_preservation"
                    && issue["message"]
                        .as_str()
                        .is_some_and(|message| message.contains("source_residual_gap_ids"))
            }))
    );
    assert!(evidence.repair_attempts.iter().any(|attempt| {
        attempt["call_id"] == "post-remediation-verification-plan-repair-1-1-1"
            && attempt["issue_kind"] == "semantic_preservation_rejected"
    }));
    assert_eq!(
        llm.calls.lock().expect("calls lock").as_slice(),
        ["post-remediation-verification-plan-repair-1-1-1"]
    );
}

#[tokio::test]
async fn accepted_zero_match_verification_is_demoted_and_routed_to_retry() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (driver, llm) = boundary_driver(&temp, CannedLifecycleScenario::ZeroTestRetry);
    let mut result = WorkflowV2Result::accepted("focused verification claimed acceptance");
    result.commands_run.push(WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: "cargo test missing_check -- --exact".to_string(),
        status: WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary:
            "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 99 filtered out"
                .to_string(),
    });
    result.data = serde_json::json!({
        "source_item_id": "source-zero-check",
        "canonical_task_ids": ["TASK-EX-BOUNDARY"],
    });
    let mut outcome = WorkflowV2BranchOutcome {
        item_id: "zero-check".to_string(),
        role: "verifier".to_string(),
        status: WorkflowV2Status::Accepted,
        result: Some(result),
        error: None,
        failure_kind: None,
        item_input_hash: Some("zero-test-input".to_string()),
        completion_evidence: Vec::new(),
    };
    workflow_live_v2_verification::normalize_focused_verification_outcome(
        "verification-wave-1-1",
        &mut outcome,
    );

    assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
    assert_eq!(outcome.failure_kind, Some(BranchFailureKind::Semantic));
    assert_eq!(
        outcome.result.as_ref().expect("result").data["zero_test_match"],
        true
    );

    let failed_outcomes = vec![serde_json::to_value(&outcome).expect("serialize outcome")];
    let mut evidence = LifecycleEvidence::default();
    let triage = driver
        .enforce_triage_accounting(
            "zero-test-triage",
            &failed_outcomes,
            serde_json::json!({
                "implementation_failures": [],
                "retry_items": [],
                "superseded_items": [],
                "terminal_blockers": [],
            }),
            &mut evidence,
        )
        .await
        .expect("zero-test triage repair");

    let routes = workflow_live_v2_lifecycle_verify_routing::triage_routes(&triage);
    assert_eq!(routes.retry_items.len(), 1);
    assert_eq!(
        routes.retry_items[0]["classification"],
        "retryable_verification_shape_issue"
    );
    assert_eq!(
        llm.calls.lock().expect("calls lock").as_slice(),
        ["zero-test-triage-shape-repair-1"]
    );
}

#[tokio::test]
async fn inventory_repair_tombstone_cannot_remove_scheduled_work() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (driver, llm) = boundary_driver(&temp, CannedLifecycleScenario::InventoryTombstone);
    let inventory = serde_json::json!({
        "items": [{
            "item_id": "scheduled-item",
            "work_type": "implementation",
            "canonical_task_ids": ["TASK-EX-BOUNDARY"],
            "dependency_ids": [],
            "target_files": ["src/boundary.rs"],
            "acceptance_criteria": ["Scheduled work survives inventory repair."],
            "focused_verification": "cargo test boundary_check -- --exact",
            "artifact_requirements": [],
        }],
        "unresolved_issues": [{
            "kind": "inventory_shape_repair",
            "field": "items",
            "message": "exercise the bounded inventory repair",
        }],
    });
    let mut evidence = LifecycleEvidence::default();

    let repaired = driver
        .repair_inventory(inventory, &serde_json::json!([]), &mut evidence)
        .await
        .expect("inventory repair");

    assert_eq!(repaired["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(repaired["items"][0]["item_id"], "scheduled-item");
    assert_eq!(
        repaired["items"][0]["canonical_task_ids"],
        serde_json::json!(["TASK-EX-BOUNDARY"])
    );
    assert!(
        repaired["unresolved_issues"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    assert_eq!(
        llm.calls.lock().expect("calls lock").as_slice(),
        ["inventory-shape-repair-1"]
    );
}

fn boundary_driver(
    temp: &tempfile::TempDir,
    scenario: CannedLifecycleScenario,
) -> (LifecycleDriver, Arc<CannedLifecycleLlm>) {
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-EX-BOUNDARY".to_string(),
            source_path: "tasks/TASK-EX-BOUNDARY.md".to_string(),
            acceptance_criteria: vec!["Boundary behavior remains semantic.".to_string()],
            ..Default::default()
        }],
    };
    let spec = WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "boundary-preservation-e2e".to_string(),
        task: "Exercise a neutral lifecycle boundary fixture.".to_string(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        provider_tiers: BTreeMap::new(),
        stages: Vec::new(),
        artifact_policy: Default::default(),
        permissions: BTreeMap::new(),
        quality_gates: BTreeMap::new(),
        learning_hooks: Vec::new(),
    };
    let workflow_store = WorkflowStore::new(temp.path().join(".archon/workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    // 87e2bb69 made TUI delivery *required*: send_async returns
    // NotificationDelivery when the receiver is gone, where v3 previously
    // ignored the result via `let _ = tui_tx.send(..)`. The two fixtures above
    // bind their receiver inside the #[tokio::test] body, so it lives for the
    // whole test; this helper RETURNS, so a local receiver drops here and
    // closes the channel -- failing the fixture on teardown rather than on the
    // lifecycle behaviour under test. Drain in the background so the bounded
    // capacity also cannot stall a long run.
    let (tui_tx, mut tui_rx) = bounded_tui_event_channel();
    tokio::spawn(async move { while tui_rx.recv().await.is_some() {} });
    let llm = Arc::new(CannedLifecycleLlm {
        scenario,
        calls: Mutex::new(Vec::new()),
        deliverable_contract_executed: AtomicBool::new(false),
        parameterized_contract_executed: AtomicBool::new(false),
        inventory_calls: AtomicUsize::new(0),
        verification_failure_emitted: AtomicBool::new(false),
    });
    let client = LiveV2AgentClient::new(
        llm.clone(),
        tui_tx,
        Vec::new(),
        run.id.clone(),
        None,
        Some(30),
    );
    let generated_config = archon_core::config::GeneratedWorkflowConfig {
        max_repair_iterations: 1,
        max_investigation_iterations: 1,
        verification_branch_timeout_secs: 30,
        host_call_timeout_secs: 30,
    };
    let runner = WorkflowV2ScriptRunner::new(
        spec.task,
        WorkflowV2ScriptRuntime {
            target_repository_root: None,
            generated_config: generated_config.clone(),
        },
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store,
        workflow_store,
        run.id,
        true,
        Some(universe.clone()),
        None,
    );
    let host = Arc::new(WorkflowScriptHost {
        scaffold_hash: workflow_scaffold_hash("# boundary preservation fixture"),
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
    (driver, llm)
}

fn synthetic_task_universe(root: &std::path::Path) -> WorkflowV2TaskUniverse {
    let task = |id: &str, criterion: &str| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.to_string(),
        source_path: root
            .join("tasks")
            .join(format!("{id}.md"))
            .display()
            .to_string(),
        acceptance_criteria: vec![criterion.to_string()],
        ..Default::default()
    };
    let mut contract_task = task("TASK-EX-004", "Declared artifact verification passes.");
    contract_task.artifact_requirements =
        vec![".archon/artifacts/example-contract.json".to_string()];
    contract_task.deliverable_contracts = vec![WorkflowV2DeliverableContract {
        kind: "example-record".to_string(),
        artifact_path: ".archon/artifacts/example-contract.json".to_string(),
        required_universe: false,
        ..Default::default()
    }];
    let mut artifact_only_task = task("TASK-EX-005", "Artifact-only output is produced.");
    artifact_only_task.artifact_requirements =
        vec![".archon/artifacts/artifact-only.json".to_string()];
    artifact_only_task.deliverable_contracts = vec![WorkflowV2DeliverableContract {
        kind: "artifact-only".to_string(),
        artifact_path: ".archon/artifacts/artifact-only.json".to_string(),
        ..Default::default()
    }];
    let mut parameterized_task = task(
        "TASK-EX-006",
        "Parameterized instance reports are contract-verified.",
    );
    parameterized_task.artifact_requirements =
        vec![".archon/artifacts/instances/<instance-id>/report.json".to_string()];
    parameterized_task.deliverable_contracts = vec![WorkflowV2DeliverableContract {
        kind: "instance_report".to_string(),
        artifact_path: ".archon/artifacts/instances/<instance-id>/report.json".to_string(),
        instance_source_path: Some(".archon/artifacts/instances.json".to_string()),
        instance_source_records_field: Some("records".to_string()),
        instance_artifact_field: Some("report_path".to_string()),
        validation_status_field: Some("status".to_string()),
        validation_checks_field: Some("checks".to_string()),
        validation_check_status_field: Some("status".to_string()),
        validation_failed_values: vec!["failed".to_string()],
        validation_passed_values: vec!["passed".to_string()],
        ..Default::default()
    }];
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec![root.join("tasks").display().to_string()],
        tasks: vec![
            task("TASK-EX-001", "Existing evidence is sufficient."),
            task("TASK-EX-002", "Refuted work is implemented."),
            task("TASK-EX-003", "Plain implementation is present."),
            contract_task,
            artifact_only_task,
            parameterized_task,
        ],
    }
}

fn synthetic_inventory_items() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "item_id": "noop-legit",
            "work_type": "verified_noop",
            "canonical_task_ids": ["TASK-EX-001"],
            "dependency_ids": [],
            "acceptance_criteria": ["Existing evidence is sufficient."],
            "noop_proof": "existing neutral fixture evidence",
            "noop_proof_refs": ["fixture:existing-evidence"],
            "artifact_requirements": [],
        }),
        serde_json::json!({
            "item_id": "noop-refutable",
            "work_type": "verified_noop",
            "canonical_task_ids": ["TASK-EX-002"],
            "dependency_ids": [],
            "acceptance_criteria": ["Refuted work is implemented."],
            "noop_proof": "unsupported inherited claim",
            "noop_proof_refs": ["fixture:missing-evidence"],
            "artifact_requirements": [],
        }),
        serde_json::json!({
            "item_id": "noop-artifact-only",
            "work_type": "verified_noop",
            "canonical_task_ids": ["TASK-EX-005"],
            "dependency_ids": [],
            "acceptance_criteria": ["Artifact-only output is produced."],
            "noop_proof": "unsupported inherited artifact claim",
            "noop_proof_refs": ["fixture:missing-artifact"],
            "artifact_requirements": [],
        }),
        implementation_item(
            "implementation-plain",
            "TASK-EX-003",
            "src/plain.rs",
            "Plain implementation is present.",
        ),
        implementation_item(
            "implementation-contract",
            "TASK-EX-004",
            "src/contract.rs",
            "Declared artifact verification passes.",
        ),
        implementation_item(
            "implementation-parameterized",
            "TASK-EX-006",
            "src/parameterized.rs",
            "Parameterized instance reports are contract-verified.",
        ),
    ]
}

fn implementation_item(
    item_id: &str,
    task_id: &str,
    target_file: &str,
    criterion: &str,
) -> serde_json::Value {
    serde_json::json!({
        "item_id": item_id,
        "work_type": "implementation",
        "canonical_task_ids": [task_id],
        "dependency_ids": [],
        "target_files": [target_file],
        "acceptance_criteria": [criterion],
        "focused_verification": format!("test -f {target_file}"),
        "artifact_requirements": if task_id == "TASK-EX-004" {
            serde_json::json!([".archon/artifacts/example-contract.json"])
        } else {
            serde_json::json!([])
        },
    })
}

fn verification_item(item_id: &str, task_id: &str, target_file: &str) -> serde_json::Value {
    serde_json::json!({
        "item_id": item_id,
        "source_item_id": item_id.replace("verify-", "implementation-"),
        "canonical_task_ids": [task_id],
        "focused_verification": format!("test -f {target_file}"),
        "expected_evidence": format!("{target_file} exists"),
        "artifact_requirements": [],
    })
}

fn verification_remediation_item() -> serde_json::Value {
    serde_json::json!({
        "item_id": "remediate-plain",
        "source_item_id": "implementation-plain",
        "work_type": "implementation",
        "canonical_task_ids": ["TASK-EX-003"],
        "dependency_ids": [],
        "target_files": ["src/plain.rs"],
        "failure_status": "needs_review",
        "failure_evidence": "the first focused check failed",
        "required_fix": "re-apply the neutral implementation",
        "focused_verification": "test -f src/plain.rs",
        "artifact_requirements": [],
    })
}

