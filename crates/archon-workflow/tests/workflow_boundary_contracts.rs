use archon_workflow::{
    StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor, WorkflowPolicy, WorkflowSpec,
    WorkflowStageRunner, WorkflowStore,
};

fn executor(store: WorkflowStore) -> WorkflowExecutor {
    WorkflowExecutor::new(
        store,
        WorkflowPolicy {
            require_human_for_dangerous_tools: false,
            ..WorkflowPolicy::default()
        },
    )
}

#[tokio::test]
async fn item_producer_rejects_malformed_completed_items_before_empty_fanout() {
    struct Runner;

    impl archon_workflow::WriteBoundaryProbe for Runner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for Runner {
        async fn run_stage(
            &self,
            _request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            Ok(StageRunOutput::markdown(
                r#"{
                    "items": [],
                    "completed_items": [{
                        "task_id": "TASK-TDL-001",
                        "status": "complete_or_audit_only",
                        "evidence": [
                            "Current repository contains the previously claimed-missing anchors."
                        ]
                    }]
                }"#,
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let spec = WorkflowSpec::from_generated_yaml(
        r#"
schema: archon.workflow.v1
name: malformed-completed-items-rejected
task: Implement decomposed work.
stages:
  - id: implementation_inventory
    kind: agent
    outputs: [items]
  - id: implement_T001
    kind: fanout
    item_kind: implementation
    foreach: ${implementation_inventory.items}
    filter: item.wave == 'T001'
    depends_on: [implementation_inventory]
"#,
        "Implement decomposed work.",
    )
    .unwrap();

    let run = executor(store.clone()).start(spec).unwrap();
    let report = executor(store.clone())
        .execute_with_runner(run.clone(), &Runner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 1);
    let inventory = finished.stages.get("implementation_inventory").unwrap();
    assert_eq!(inventory.status, StageStatus::Failed);
    assert!(
        inventory
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("invalid completed_items claim")
    );
}

#[tokio::test]
async fn quality_gate_rejects_malformed_completed_items_dependency() {
    struct Runner;

    impl archon_workflow::WriteBoundaryProbe for Runner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for Runner {
        async fn run_stage(
            &self,
            _request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            Ok(StageRunOutput::markdown(
                r#"{
                    "completed_items": [{
                        "task_id": "T001",
                        "status": "already_implemented",
                        "verified": true,
                        "evidence": ["vague string evidence is not enough"]
                    }]
                }"#,
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: malformed-completed-items-quality-gate
task: Gate completed claim evidence.
stages:
  - id: inventory
    kind: agent
  - id: inventory_quality_gate
    kind: quality_gate
    depends_on: [inventory]
"#,
    )
    .unwrap();

    let run = executor(store.clone()).start(spec).unwrap();
    let report = executor(store.clone())
        .execute_with_runner(run.clone(), &Runner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 1);
    let gate = finished.stages.get("inventory_quality_gate").unwrap();
    assert_eq!(gate.status, StageStatus::Failed);
    assert!(
        gate.error
            .as_deref()
            .unwrap_or_default()
            .contains("failed completed-items contract")
    );
}

#[tokio::test]
async fn quality_gate_allows_reducer_report_with_completed_items_proof() {
    struct Runner;

    impl archon_workflow::WriteBoundaryProbe for Runner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for Runner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            assert_eq!(request.stage_id, "inventory");
            Ok(StageRunOutput::markdown(
                r#"{
                    "items": [],
                    "completed_items": [{
                        "task_ids": ["TASK-TDL-001"],
                        "canonical_task_ids": ["T001-data-lake-gap-audit"],
                        "verified": true,
                        "status": "accepted",
                        "evidence": [
                            "crates/archon-trading/src/data_store.rs shows the existing data root.",
                            "tasks/PRD-TRADING-DATA-LAKE-AHDM-001/context/progress.md records the audit-only status."
                        ],
                        "target_files": []
                    }]
                }"#,
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: reducer-completed-proof-gate
task: Gate reducer evidence without treating it as write output.
stages:
  - id: inventory
    kind: agent
    outputs: [items]
  - id: reduce
    kind: reduce
    depends_on: [inventory]
  - id: gate
    kind: quality_gate
    depends_on: [reduce]
"#,
    )
    .unwrap();

    let run = executor(store.clone()).start(spec).unwrap();
    let report = executor(store.clone())
        .execute_with_runner(run.clone(), &Runner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 0);
    assert_eq!(
        finished.stages.get("gate").unwrap().status,
        StageStatus::Accepted
    );
}

#[tokio::test]
async fn implementation_fanout_rejects_items_without_work_unit_metadata() {
    struct Runner;

    impl archon_workflow::WriteBoundaryProbe for Runner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for Runner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            assert_eq!(request.stage_id, "inventory");
            Ok(StageRunOutput::markdown(
                r#"{"items":[{"target_files":["src/lib.rs"],"task":"edit something"}]}"#,
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: missing-work-unit-item
task: Reject unscoped implementation items.
stages:
  - id: inventory
    kind: agent
    outputs: [items]
  - id: implement
    kind: fanout
    item_kind: implementation
    foreach: ${inventory.items}
    depends_on: [inventory]
"#,
    )
    .unwrap();

    let run = executor(store.clone()).start(spec).unwrap();
    let report = executor(store.clone())
        .execute_with_runner(run.clone(), &Runner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 1);
    let implement = finished.stages.get("implement").unwrap();
    assert_eq!(implement.status, StageStatus::Failed);
    assert!(
        implement
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("requires work_unit_id")
    );
}

#[tokio::test]
async fn implementation_fanout_items_inherit_stage_work_unit_scope() {
    struct Runner {
        first_target: String,
        second_target: String,
    }

    impl archon_workflow::WriteBoundaryProbe for Runner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for Runner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            match request.stage_id.as_str() {
                "inventory" => Ok(StageRunOutput::markdown(
                    serde_json::json!({
                        "items": [
                            {
                                "target_files": [self.first_target.clone()],
                                "task": "rewrite migrated metadata"
                            },
                            {
                                "target_files": [self.second_target.clone()],
                                "task": "make provider notes atomic"
                            }
                        ]
                    })
                    .to_string(),
                )),
                stage if stage.starts_with("implement-") => Ok(StageRunOutput::markdown(
                    serde_json::json!({
                        "status": "accepted",
                        "completed_task_ids": ["TASK-TDL-010"],
                        "source_files_changed": [
                            self.first_target.clone(),
                            self.second_target.clone()
                        ],
                        "focused_tests": [
                            {
                                "command": "cargo test -p archon-trading data_store::tests::metadata_json_contains_self_describing_paths_and_checksums",
                                "exit_status": 0,
                                "result": "passed"
                            }
                        ],
                        "notes": "No residual gaps for this remediation item."
                    })
                    .to_string(),
                )),
                other => panic!("unexpected stage {other}"),
            }
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let first_target = temp.path().join("src/data_store.rs");
    let second_target = temp.path().join("src/data_store/io.rs");
    std::fs::create_dir_all(first_target.parent().unwrap()).unwrap();
    std::fs::create_dir_all(second_target.parent().unwrap()).unwrap();
    std::fs::write(&first_target, "pub fn data_store() {}\n").unwrap();
    std::fs::write(&second_target, "pub fn io() {}\n").unwrap();
    let store = WorkflowStore::project(temp.path());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: stage-scoped-work-unit-items
task: Allow generated remediation items to inherit stage scope.
stages:
  - id: inventory
    kind: agent
    outputs: [items]
  - id: implement
    kind: fanout
    item_kind: implementation
    completion_task_ids: ["TASK-TDL-010"]
    foreach: ${inventory.items}
    depends_on: [inventory]
"#,
    )
    .unwrap();

    let run = executor(store.clone()).start(spec).unwrap();
    let report = executor(store.clone())
        .execute_with_runner(
            run.clone(),
            &Runner {
                first_target: first_target.display().to_string(),
                second_target: second_target.display().to_string(),
            },
        )
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    let implement = finished.stages.get("implement").unwrap();
    assert_eq!(
        report.failed, 0,
        "implement stage error: {:?}",
        implement.error
    );
    assert_eq!(implement.status, StageStatus::Accepted);
}

#[tokio::test]
async fn item_producer_rejects_non_array_completed_items_even_with_items() {
    struct Runner;

    impl archon_workflow::WriteBoundaryProbe for Runner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for Runner {
        async fn run_stage(
            &self,
            _request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            Ok(StageRunOutput::markdown(
                r#"{
                    "items": [{
                        "task_id": "T010",
                        "target_files": ["src/lib.rs"],
                        "task": "implement T010"
                    }],
                    "completed_items": "T001 is done"
                }"#,
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: completed-items-type-contract
task: Reject malformed completed_items.
stages:
  - id: inventory
    kind: agent
    outputs: [items]
"#,
    )
    .unwrap();

    let run = executor(store.clone()).start(spec).unwrap();
    let report = executor(store.clone())
        .execute_with_runner(run.clone(), &Runner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 1);
    let inventory = finished.stages.get("inventory").unwrap();
    assert_eq!(inventory.status, StageStatus::Failed);
    assert!(
        inventory
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("completed_items must be an array")
    );
}

#[tokio::test]
async fn implementation_fanout_rejects_items_without_declared_targets() {
    struct Runner;

    impl archon_workflow::WriteBoundaryProbe for Runner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for Runner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            assert_eq!(request.stage_id, "inventory");
            Ok(StageRunOutput::markdown(
                r#"{"items":[{"task_id":"T001","task":"edit something"}]}"#,
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: missing-target-item
task: Reject targetless implementation items.
stages:
  - id: inventory
    kind: agent
    outputs: [items]
  - id: implement
    kind: fanout
    item_kind: implementation
    foreach: ${inventory.items}
    depends_on: [inventory]
"#,
    )
    .unwrap();

    let run = executor(store.clone()).start(spec).unwrap();
    let report = executor(store.clone())
        .execute_with_runner(run.clone(), &Runner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 1);
    let implement = finished.stages.get("implement").unwrap();
    assert_eq!(implement.status, StageStatus::Failed);
    assert!(
        implement
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("requires target_files")
    );
}
