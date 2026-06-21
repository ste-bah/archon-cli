use archon_workflow::{
    StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor, WorkflowPolicy, WorkflowSpec,
    WorkflowStageRunner, WorkflowStore,
};

fn permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

#[tokio::test]
async fn read_only_review_fanout_accepts_structured_review_json() {
    struct ReviewRunner;

    impl archon_workflow::WriteBoundaryProbe for ReviewRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for ReviewRunner {
        async fn run_stage(
            &self,
            _request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            Ok(StageRunOutput::markdown(
                r#"{"status":"accepted","findings":[],"summary":"read-only review"}"#,
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: read-only-review-json
task: Review structured evidence.
stages:
  - id: review
    kind: fanout
    input:
      items:
        - task: inspect only
"#,
    )
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &ReviewRunner)
        .await
        .unwrap();
    assert_eq!(report.failed, 0);

    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(
        finished.stages.get("review").unwrap().status,
        StageStatus::Accepted
    );
}

#[tokio::test]
async fn read_only_audit_fanout_collects_gaps_without_treating_suggested_commands_as_execution() {
    struct AuditRunner;

    impl archon_workflow::WriteBoundaryProbe for AuditRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for AuditRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "discover" {
                return Ok(StageRunOutput::markdown(
                    r#"{"items":[{"id":"TASK-TDL-100","task":"audit only"}]}"#,
                ));
            }
            Ok(StageRunOutput::markdown(
                r#"{
                    "item_id": "TASK-TDL-100",
                    "status": "audit_complete_read_only",
                    "evidence_found": [{"area": "tests", "evidence": "focused tests exist"}],
                    "gaps": [{"id": "GAP-1", "severity": "high", "description": "needs implementation"}],
                    "commands_suitable_for_focused_verification": [
                        {
                            "command": "cargo test -p demo missing_filter -- --nocapture",
                            "purpose": "suggested command, not executed evidence",
                            "note": "if this matched zero tests later, the verification stage must fail"
                        }
                    ],
                    "target_files": ["src/lib.rs"]
                }"#,
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: read-only-audit-gaps
task: Audit before planning.
stages:
  - id: discover
    kind: agent
    outputs: [items]
  - id: read-audit
    kind: fanout
    foreach: "${discover.items}"
    depends_on: [discover]
  - id: planning-gate
    kind: quality_gate
    depends_on: [read-audit]
"#,
    )
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &AuditRunner)
        .await
        .unwrap();
    assert_eq!(report.failed, 0);

    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(
        finished.stages.get("read-audit").unwrap().status,
        StageStatus::Accepted
    );
    assert_eq!(
        finished.stages.get("planning-gate").unwrap().status,
        StageStatus::Accepted
    );
    assert!(
        finished
            .stages
            .get("read-audit")
            .unwrap()
            .artifacts
            .iter()
            .all(|artifact| artifact.accepted),
        "read-only audit artifacts are collected evidence for downstream reduction"
    );
}

#[tokio::test]
async fn empty_non_recovery_implementation_fanout_fails_even_with_downstream_recovery() {
    struct EmptyImplementationRunner;

    impl archon_workflow::WriteBoundaryProbe for EmptyImplementationRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for EmptyImplementationRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            match request.stage_id.as_str() {
                "inventory" => Ok(StageRunOutput::markdown(r#"{"items":[]}"#)),
                "remediation-inventory" => Ok(StageRunOutput::markdown(r#"{"items":[]}"#)),
                _ => Ok(StageRunOutput::markdown("status: completed")),
            }
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: empty-implementation-is-structural
task: Do not accept empty implementation waves.
stages:
  - id: inventory
    kind: agent
    agent: planner
    outputs: [items]
  - id: implement
    kind: fanout
    foreach: "${inventory.items}"
    item_kind: implementation
    allow_empty_items: true
    depends_on: [inventory]
  - id: remediation-inventory
    kind: agent
    agent: critic
    outputs: [items]
    depends_on: [implement]
"#,
    )
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &EmptyImplementationRunner)
        .await
        .unwrap();
    assert_eq!(report.failed, 1);

    let finished = store.load_state(&run.id).unwrap();
    let stage = finished.stages.get("implement").unwrap();
    assert_eq!(stage.status, StageStatus::Failed);
    assert!(
        stage
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("resolved zero items")
    );
}
