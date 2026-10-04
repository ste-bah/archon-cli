//! Round 7 (#253): a host command lands as a whole. A lifecycle edit in the
//! publication window never discards its record or runs it again.
use super::*;
use archon_workflow::{LifecycleAction, LifecycleController, RunStatus};

async fn host_command_edit(
    action: fn(&str) -> LifecycleAction,
) -> (
    usize,
    Option<WorkflowV2Status>,
    archon_workflow::WorkflowResult<()>,
) {
    let temp = tempfile::tempdir().expect("tempdir");
    // A spec with a stage, so restart and force-accept apply.
    let spec =
        crate::command::workflow_live::workflow_live_v2::workflow_run_finalizer_tests::spec();
    let stage = "call-1".to_string();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let mut run = workflow_store.create_run(spec.clone()).expect("run");
    run.status = RunStatus::Running;
    workflow_store.save_state(&run).expect("state");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let executor = Arc::new(FakeHostCommandExecutor {
        calls: AtomicUsize::new(0),
    });
    let controller = LifecycleController::new(workflow_store.clone());
    let id = run.id.clone();
    let edit = action(&stage);
    crate::command::workflow_live::workflow_live_v2::workflow_live_v2_fixed_persistence::publication_hook::install(
        workflow_store.run_dir(&id),
        Box::new(move || {
            controller.apply(&id, edit).expect("edit in publication window");
        }),
    );
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "host command publication".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        None,
        None,
    )
    .with_host_command_executor(executor.clone());
    let result = runner
        .run(r#"async function workflow(w) { return await w.hostCommand("task-set-lint", { stdin: null }); }"#)
        .await
        .map(|_| ());
    let status = v2_store
        .load_call_record("host-command:task-set-lint:fixed")
        .expect("record lookup")
        .map(|record| record.status);
    (executor.calls.load(Ordering::SeqCst), status, result)
}

#[tokio::test]
async fn round7_restart_at_host_command_publication_publishes_once() {
    let (calls, status, result) =
        host_command_edit(|stage| LifecycleAction::RestartStage(stage.into())).await;
    assert_eq!(calls, 1, "a landed host command must never run again");
    assert_eq!(status, Some(WorkflowV2Status::Accepted), "{result:?}");
}

#[tokio::test]
async fn round7_force_accept_at_host_command_publication_publishes_once() {
    let (calls, status, result) = host_command_edit(|stage| LifecycleAction::ForceAcceptStage {
        stage_id: stage.into(),
        forced_by: "operator".into(),
        rationale: "reviewed".into(),
        source: "test".into(),
    })
    .await;
    assert_eq!(calls, 1, "a landed host command must never run again");
    assert_eq!(status, Some(WorkflowV2Status::Accepted), "{result:?}");
}

#[tokio::test]
async fn round7_pause_at_host_command_publication_saves_record_then_pauses() {
    let (calls, status, result) = host_command_edit(|_| LifecycleAction::Pause).await;
    assert_eq!(calls, 1);
    assert_eq!(
        status,
        Some(WorkflowV2Status::Accepted),
        "the landed record is saved before the pause is raised"
    );
    assert!(
        matches!(result, Err(WorkflowError::ControlPaused(_))),
        "{result:?}"
    );
}
