use archon_workflow::{
    RunStatus, StageRunOutput, StageRunRequest, WorkflowError, WorkflowExecutor, WorkflowPolicy,
    WorkflowResult, WorkflowSpec, WorkflowStageRunner, WorkflowStore,
};

struct FailingRunner;

#[async_trait::async_trait]
impl WorkflowStageRunner for FailingRunner {
    async fn run_stage(&self, _request: StageRunRequest) -> WorkflowResult<StageRunOutput> {
        Err(WorkflowError::StageFailed("provider unavailable".into()))
    }
}

#[tokio::test]
async fn failed_live_agent_records_normalized_failure_output() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: failed-live-agent
task: fail cleanly
stages:
  - id: inspect
    kind: agent
    agent: tester
"#,
    )
    .unwrap();
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(spec).unwrap();
    let run_id = run.id.clone();

    let report = executor
        .execute_with_runner(run, &FailingRunner)
        .await
        .unwrap();
    assert_eq!(report.failed, 1);
    assert_eq!(store.load_state(&run_id).unwrap().status, RunStatus::Failed);

    let output_path = store
        .run_dir(&run_id)
        .join("agent-outputs")
        .join("inspect")
        .join("inspect.json");
    let output = std::fs::read_to_string(output_path).unwrap();
    assert!(output.contains("provider unavailable"));
    assert!(output.contains("\"accepted\": false"));
}
