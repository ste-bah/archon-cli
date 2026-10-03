//! Issue #255 end to end through the script host and the real fixed executor:
//! a host-command timeout pauses the fixed run (it used to fail it through
//! `stage_failed workflow.js`), and a resume runs the interrupted call again.
//! Also the R7 continuation: a run that already FAILED that way, paused by
//! the operator, re-runs its failed call on resume.

use super::*;

use crate::command::workflow_host_command_exec::{
    FixedHostCommandExecutor, HostCommandProcessAdapter,
};
use crate::command::workflow_host_command_supervisor::{
    HostCommandControl, SupervisedProcessOutput,
};

const TIME_OUT: usize = 0;
const LEGACY_TIMEOUT_ERROR: usize = 1;
const GENUINE_EXIT_1: usize = 2;

const SCRIPT: &str = r#"async function workflow(w) { return await w.hostCommand("task-set-lint", { stdin: null }); }"#;

struct ModeProcess {
    mode: AtomicUsize,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl HostCommandProcessAdapter for ModeProcess {
    async fn execute(
        &self,
        request: crate::command::workflow_host_command_catalog::ResolvedHostCommand,
        _control: HostCommandControl,
    ) -> archon_workflow::WorkflowResult<SupervisedProcessOutput> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let output = |exit_code, timed_out| SupervisedProcessOutput {
            exit_code,
            timed_out,
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_bytes: 0,
            stderr_bytes: 0,
        };
        match self.mode.load(Ordering::SeqCst) {
            TIME_OUT => Ok(output(None, true)),
            // What the supervisor returned before Issue #255.
            LEGACY_TIMEOUT_ERROR => Err(WorkflowError::StageFailed(format!(
                "host command '{}' timed out after {}s",
                request.command_id, request.timeout_secs
            ))),
            _ => Ok(output(Some(1), false)),
        }
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    store: WorkflowStore,
    run_id: String,
    log_path: std::path::PathBuf,
    process: Arc<ModeProcess>,
    executor: Arc<FixedHostCommandExecutor>,
    call_id: String,
}

fn fixture(mode: usize) -> Fixture {
    use crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor;
    let temp = tempfile::tempdir().expect("tempdir");
    let mut context = crate::command::workflow_host_command_exec_tests::context(temp.path());
    let store = WorkflowStore::project(&context.project_root);
    let run = store.create_run(test_spec()).expect("run");
    let log_path = context.task_root.join(".decompose.log");
    super::workflow_live_v2_script_fixed_progress_tests::seed_fixed_progress_state(
        &store, &run.id, &log_path,
    );
    context.run_staging_root = store.run_dir(&run.id).join("host-command-staging");
    let process = Arc::new(ModeProcess {
        mode: AtomicUsize::new(mode),
        calls: AtomicUsize::new(0),
    });
    let executor = Arc::new(FixedHostCommandExecutor::with_process(
        crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev-1")
            .unwrap(),
        context,
        store.run_dir(&run.id),
        process.clone(),
    ));
    let call_id = executor
        .call_identity(&archon_workflow::HostCommandRequest::new("task-set-lint", None).unwrap())
        .unwrap();
    Fixture {
        _temp: temp,
        store,
        run_id: run.id,
        log_path,
        process,
        executor,
        call_id,
    }
}

async fn run_script(fixture: &Fixture) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let spec = test_spec();
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        ui_sink,
        Vec::new(),
        fixture.run_id.clone(),
        None,
        None,
    );
    WorkflowV2ScriptRunner::new(
        "host command operational".into(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2")),
        fixture.store.clone(),
        fixture.run_id.clone(),
        true,
        None,
        None,
    )
    .with_host_command_executor(fixture.executor.clone())
    .run(SCRIPT)
    .await
}

fn record(fixture: &Fixture) -> WorkflowV2CallRecord {
    WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2"))
        .load_call_record(&fixture.call_id)
        .expect("record lookup")
        .expect("the host call left a record")
}

fn resume(fixture: &Fixture) {
    archon_workflow::LifecycleController::new(fixture.store.clone())
        .apply(&fixture.run_id, archon_workflow::LifecycleAction::Resume)
        .expect("resume");
}

#[tokio::test]
async fn a_host_command_timeout_pauses_the_fixed_run_and_resume_re_runs_the_call() {
    let fixture = fixture(TIME_OUT);

    let error = run_script(&fixture).await.expect_err("the run stops");

    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "a timeout pauses, it does not fail: {error:?}"
    );
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 2);
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(run.status, RunStatus::Paused);
    assert_eq!(run.stages[&fixture.call_id].status, StageStatus::Paused);
    let interrupted = record(&fixture);
    assert_eq!(interrupted.result.data["interrupted"], "paused");
    assert!(!is_reusable_status(interrupted.status));
    let error_text = interrupted.result.data["error"].as_str().unwrap();
    assert!(error_text.contains("operational limit"), "{error_text}");
    let events = std::fs::read_to_string(fixture.store.events_path(&fixture.run_id)).unwrap();
    assert!(
        events.contains("host_command_operational_retry"),
        "{events}"
    );
    assert!(
        events.contains("host_command_operational_pause"),
        "{events}"
    );
    assert!(!events.contains("\"call_id\":\"workflow.js\""), "{events}");
    let log = std::fs::read_to_string(&fixture.log_path).unwrap();
    assert!(
        log.contains("transition=host_command_operational_pause"),
        "{log}"
    );

    resume(&fixture);
    fixture.process.mode.store(GENUINE_EXIT_1, Ordering::SeqCst);
    let _ = run_script(&fixture).await;

    assert_eq!(
        fixture.process.calls.load(Ordering::SeqCst),
        3,
        "the interrupted call runs again on resume, it is not reused"
    );
    let rerun = record(&fixture);
    assert_eq!(rerun.attempt, interrupted.attempt + 1);
    assert_ne!(rerun.result.data["interrupted"], "paused");
    assert_eq!(rerun.result.data["exitCode"], 1);
}

#[tokio::test]
async fn a_failed_fixed_run_paused_by_the_operator_re_runs_its_failed_host_call() {
    // The R7 shape: the pre-#255 timeout error recorded the call `failed`,
    // and the finalizer marked the run and the call's stage failed.
    let fixture = fixture(LEGACY_TIMEOUT_ERROR);
    let _ = run_script(&fixture).await;
    let failed = record(&fixture);
    assert_eq!(failed.status, WorkflowV2Status::Failed);
    assert!(
        failed.result.summary.contains("timed out after"),
        "{}",
        failed.result.summary
    );
    let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
    run.status = RunStatus::Failed;
    run.stages.get_mut(&fixture.call_id).unwrap().status = StageStatus::Failed;
    fixture.store.save_state(&run).unwrap();

    // `workflow pause` has no status precondition.
    archon_workflow::LifecycleController::new(fixture.store.clone())
        .apply(&fixture.run_id, archon_workflow::LifecycleAction::Pause)
        .expect("pause a failed run");
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Paused
    );
    resume(&fixture);
    let resumed = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(resumed.status, RunStatus::Running);
    // Resume leaves a failed stage as it is; the call record, not the stage,
    // decides re-execution, and a failed record is never reused.
    assert_eq!(resumed.stages[&fixture.call_id].status, StageStatus::Failed);

    fixture.process.mode.store(GENUINE_EXIT_1, Ordering::SeqCst);
    let _ = run_script(&fixture).await;

    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 2);
    let rerun = record(&fixture);
    assert_eq!(rerun.attempt, failed.attempt + 1);
    assert_eq!(rerun.result.data["exitCode"], 1);
}
