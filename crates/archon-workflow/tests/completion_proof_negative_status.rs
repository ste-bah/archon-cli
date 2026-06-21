use archon_workflow::{
    StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor, WorkflowPolicy, WorkflowSpec,
    WorkflowStageRunner, WorkflowStore,
};

struct NotAcceptedProofRunner;

impl archon_workflow::WriteBoundaryProbe for NotAcceptedProofRunner {}

#[async_trait::async_trait]
impl WorkflowStageRunner for NotAcceptedProofRunner {
    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        if request.stage_id == "discover" {
            return Ok(StageRunOutput::markdown(
                r#"{
                    "items": [],
                    "completed_items": [{
                        "task_ids": ["T001"],
                        "status": "not_accepted",
                        "verified": true,
                        "evidence": [{"path":"tasks/context/progress.md","summary":"negative accepted proof must not pass"}]
                    }]
                }"#,
            ));
        }
        Ok(StageRunOutput::markdown("unexpected item"))
    }
}

#[tokio::test]
async fn not_accepted_status_is_not_a_valid_empty_wave_proof() {
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
name: not-accepted-proof-status
task: Reject negative accepted statuses.
stages:
  - id: discover
    kind: agent
    outputs: [items]
  - id: implement
    kind: fanout
    item_kind: implementation
    foreach: ${discover.items}
    allow_empty_when_completed: true
    completion_task_ids: [T001]
    depends_on: [discover]
"#,
    )
    .unwrap();

    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &NotAcceptedProofRunner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 1);
    assert_eq!(
        finished.stages.get("discover").unwrap().status,
        StageStatus::Failed
    );
}
