use archon_workflow::{
    RunStatus, StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor, WorkflowPolicy,
    WorkflowSpec, WorkflowStageRunner, WorkflowStore,
};

#[test]
fn generated_implementation_fanouts_get_completion_contracts() {
    let spec = WorkflowSpec::from_generated_yaml(
        r#"
schema: archon.workflow.v1
name: generated-completion-contracts
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
  - id: implement_T040_T050_T060_T070
    kind: fanout
    item_kind: implementation
    foreach: ${implementation_inventory.items}
    filter: item.wave == 'T040_T050_T060_T070'
    depends_on: [implementation_inventory]
"#,
        "Implement decomposed work.",
    )
    .unwrap();

    let t001 = stage(&spec, "implement_T001");
    assert_eq!(
        t001.extra
            .get("allow_empty_when_completed")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        strings(t001.extra.get("completion_task_ids")),
        vec!["T001".to_string()]
    );

    let providers = stage(&spec, "implement_T040_T050_T060_T070");
    assert_eq!(
        strings(providers.extra.get("completion_task_ids")),
        vec![
            "T040".to_string(),
            "T050".to_string(),
            "T060".to_string(),
            "T070".to_string()
        ]
    );
}

#[tokio::test]
async fn generated_empty_implementation_uses_completed_items_alias_proof() {
    struct Runner;

    impl archon_workflow::WriteBoundaryProbe for Runner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for Runner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            assert_eq!(request.stage_id, "implementation_inventory");
            Ok(StageRunOutput::markdown(
                r#"{
                    "items": [],
                    "completed_items": [{
                        "wave": "T001",
                        "task_ids": ["TASK-TDL-001"],
                        "status": "completed_audit_only",
                        "verified": true,
                        "evidence": [{
                            "path": "tasks/context/activeContext.md",
                            "summary": "T001 is audit-only and complete"
                        }]
                    }]
                }"#,
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(
        store.clone(),
        WorkflowPolicy {
            require_human_for_dangerous_tools: false,
            ..WorkflowPolicy::default()
        },
    );
    let spec = WorkflowSpec::from_generated_yaml(
        r#"
schema: archon.workflow.v1
name: generated-empty-completion-proof
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

    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &Runner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 0);
    assert_eq!(finished.status, RunStatus::Completed);
    assert_eq!(
        finished.stages.get("implement_T001").unwrap().status,
        StageStatus::Accepted
    );
}

#[tokio::test]
async fn item_producer_rejects_conversational_output() {
    struct Runner;

    impl archon_workflow::WriteBoundaryProbe for Runner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for Runner {
        async fn run_stage(
            &self,
            _request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            Ok(StageRunOutput::markdown(
                "Context restored. What would you like me to do next?",
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(
        store.clone(),
        WorkflowPolicy {
            require_human_for_dangerous_tools: false,
            ..WorkflowPolicy::default()
        },
    );
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: item-producer-output-contract
task: Produce implementation inventory.
stages:
  - id: implementation_inventory
    kind: agent
    outputs: [items]
"#,
    )
    .unwrap();

    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &Runner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 1);
    assert_eq!(
        finished
            .stages
            .get("implementation_inventory")
            .unwrap()
            .status,
        StageStatus::Failed
    );
    assert!(
        finished
            .stages
            .get("implementation_inventory")
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("emitted no parseable items or completed_items")
    );
}

fn stage<'a>(spec: &'a WorkflowSpec, id: &str) -> &'a archon_workflow::StageSpec {
    spec.stages.iter().find(|stage| stage.id == id).unwrap()
}

fn strings(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_string)
        .collect()
}
