use archon_workflow::{
    LifecycleAction, LifecycleController, RunStatus, StageRunOutput, StageRunRequest,
    WorkflowExecutor, WorkflowPolicy, WorkflowSpec, WorkflowStageRunner, WorkflowStore,
};

fn permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

#[tokio::test]
async fn restarted_blocked_stage_reports_no_runnable_stage() {
    struct FailedDependencyRunner;

    impl archon_workflow::WriteBoundaryProbe for FailedDependencyRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for FailedDependencyRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            match request.stage_id.as_str() {
                "compile" => Ok(StageRunOutput::markdown(r#"{"status":"failed"}"#)),
                _ => Ok(StageRunOutput::markdown(
                    r#"{"status":"accepted","target_files":["src/lib.rs"],"verification":[{"command":"noop","exit_status":0}],"residual_gaps":[]}"#,
                )),
            }
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: blocked-restart
task: A restarted dependent stage must not leave a fake running workflow.
stages:
  - id: compile
    kind: agent
  - id: post-tests
    kind: agent
    depends_on: [compile]
"#,
    )
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &FailedDependencyRunner)
        .await
        .unwrap();
    assert_eq!(report.failed, 1);

    LifecycleController::new(store.clone())
        .apply(
            &run.id,
            LifecycleAction::RestartStage("post-tests".to_string()),
        )
        .unwrap();
    let restarted = store.load_state(&run.id).unwrap();
    assert_eq!(restarted.status, RunStatus::Running);

    let err = executor
        .execute_with_runner(restarted, &FailedDependencyRunner)
        .await
        .expect_err("blocked resume should fail explicitly");
    assert!(
        err.to_string().contains("no runnable stage")
            || err
                .to_string()
                .contains("pending stages but no runnable stage"),
        "unexpected error: {err}"
    );
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(finished.status, RunStatus::Failed);
}
