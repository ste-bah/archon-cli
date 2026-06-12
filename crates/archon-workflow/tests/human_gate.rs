use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use archon_workflow::{
    LifecycleAction, LifecycleController, RunStatus, StageRunOutput, StageRunRequest, StageStatus,
    WorkflowExecutor, WorkflowPolicy, WorkflowResult, WorkflowSpec, WorkflowStageRunner,
    WorkflowStore,
};

#[test]
fn human_gate_pauses_without_accepting_or_writing_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(gate_spec()).unwrap();
    let report = executor.execute(run.clone()).unwrap();

    assert_eq!(report.completed, 0);
    assert_eq!(report.failed, 0);
    let paused = store.load_state(&run.id).unwrap();
    assert_eq!(paused.status, RunStatus::Paused);
    assert_eq!(
        paused.stages.get("approve").unwrap().status,
        StageStatus::Paused
    );
    assert!(paused.stages.get("approve").unwrap().artifacts.is_empty());
    assert_eq!(
        paused.stages.get("after").unwrap().status,
        StageStatus::Pending
    );
    assert!(
        std::fs::read_to_string(store.events_path(&run.id))
            .unwrap()
            .contains("human_gate")
    );
}

#[tokio::test]
async fn live_human_gate_skips_runner_until_explicit_approval() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(gate_spec()).unwrap();
    let runner = CountingRunner::default();
    executor
        .execute_with_runner(run.clone(), &runner)
        .await
        .unwrap();

    assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
    let paused = store.load_state(&run.id).unwrap();
    assert_eq!(paused.status, RunStatus::Paused);
    assert_eq!(
        paused.stages.get("approve").unwrap().status,
        StageStatus::Paused
    );

    let approved = LifecycleController::new(store.clone())
        .apply(
            &run.id,
            LifecycleAction::ForceAcceptStage {
                stage_id: "approve".into(),
                forced_by: "unit-test".into(),
                rationale: "human gate approved".into(),
                source: "test".into(),
            },
        )
        .unwrap();
    let resumed = executor
        .execute_with_runner(approved, &runner)
        .await
        .unwrap();

    assert_eq!(resumed.failed, 0);
    assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(finished.status, RunStatus::Completed);
    assert_eq!(
        finished.stages.get("approve").unwrap().status,
        StageStatus::ForcedAccepted
    );
    assert_eq!(
        finished.stages.get("after").unwrap().status,
        StageStatus::Accepted
    );
}

#[derive(Default)]
struct CountingRunner {
    calls: Arc<AtomicUsize>,
}

impl archon_workflow::WriteBoundaryProbe for CountingRunner {}
#[async_trait::async_trait]
impl WorkflowStageRunner for CountingRunner {
    async fn run_stage(&self, _: StageRunRequest) -> WorkflowResult<StageRunOutput> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(StageRunOutput::markdown("approved downstream work"))
    }
}

fn gate_spec() -> WorkflowSpec {
    WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: human-gate-test
task: verify human gates pause
stages:
  - id: approve
    kind: human_gate
    task: Require explicit approval.
  - id: after
    kind: agent
    task: Run only after approval.
    depends_on: [approve]
"#,
    )
    .unwrap()
}
