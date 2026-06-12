use archon_workflow::{
    StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor, WorkflowPolicy, WorkflowSpec,
    WorkflowStageRunner, WorkflowStore,
};

struct PartialFailureRunner;

impl archon_workflow::WriteBoundaryProbe for PartialFailureRunner {}
#[async_trait::async_trait]
impl WorkflowStageRunner for PartialFailureRunner {
    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        if request.stage_id == "discover" {
            return Ok(StageRunOutput::markdown(
                r#"{"items":[{"path":"src/a.rs"},{"path":"src/b.rs"}]}"#,
            ));
        }
        if request.stage_id == "review-1" {
            return Err(archon_workflow::WorkflowError::StageFailed(
                "provider error".into(),
            ));
        }
        Ok(StageRunOutput::markdown("reviewed"))
    }
}

#[tokio::test]
async fn partial_fanout_failure_fails_stage() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/a.rs"), "fn a() {}").unwrap();
    std::fs::write(temp.path().join("src/b.rs"), "fn b() {}").unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(audit_spec()).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &PartialFailureRunner)
        .await
        .unwrap();
    assert_eq!(report.failed, 1);

    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(
        finished.stages.get("review").unwrap().status,
        StageStatus::Failed
    );
}

fn audit_spec() -> WorkflowSpec {
    WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: partial-fanout-failure
task: Audit src/a.rs src/b.rs
stages:
  - id: discover
    kind: agent
    outputs: [items]
  - id: review
    kind: fanout
    foreach: ${discover.items}
    depends_on: [discover]
"#,
    )
    .unwrap()
}
