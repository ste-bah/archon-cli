//! Issue 291: executor A (started under generation g) is still running when a
//! resume hands the run to executor B. From then on A dispatches no call,
//! writes no call record, checkpoint or event, and changes no run state; it
//! stops with a stale-session refusal. Each write path is also fenced on its
//! own, under the run lock, so a takeover after A's entry check still finds
//! the fence.
use super::workflow_live_v2_reuse_verify_lineage_tests::CountingAcceptedLlm;
use super::workflow_live_v2_script_pause_tests::{new_run, resume, runner, set_status};
use super::*;
use archon_workflow::{LifecycleAction, LifecycleController, RunStatus};

/// A script that catches every control error and keeps calling.
const STALE_SCRIPT: &str = r#"
async function workflow(w) {
  const refusals = [];
  try { await w.checkpoint("first", {}); } catch (error) { refusals.push(String(error.kind)); }
  try { await w.checkpoint("second", {}); } catch (error) { refusals.push(String(error.kind)); }
  try { await w.agent("third", { role: "analysis", task: "never dispatched" }); }
  catch (error) { refusals.push(String(error.kind)); }
  return { refusals };
}
"#;

fn take_over(store: &WorkflowStore, run_id: &str) {
    LifecycleController::new(store.clone())
        .apply(run_id, LifecycleAction::Pause)
        .unwrap();
    resume(store, run_id);
}

fn host_of(runner: WorkflowV2ScriptRunner) -> WorkflowScriptHost {
    WorkflowScriptHost {
        scaffold_hash: String::new(),
        host_occurrences: Default::default(),
        envelope_shape: ScriptEnvelopeShape::Compat,
        runner,
        accumulator: Arc::new(tokio::sync::Mutex::new(Default::default())),
        tool_host: Default::default(),
        tool_budget: Arc::new(std::sync::Mutex::new(Default::default())),
    }
}

/// A host for executor A, bound to the generation it started under, after
/// B took the run over; with the run files as B left them.
fn stale_host(llm: Arc<dyn WorkflowLlmClient>) -> StaleFixture {
    let (temp, store, run_id) = new_run();
    set_status(&store, &run_id, RunStatus::Running);
    let bound = store.load_state(&run_id).unwrap().generation;
    let (runner, rx) = runner(&store, &run_id, llm, None, None);
    runner.v2_store.bind_session_executor(bound);
    take_over(&store, &run_id);
    let owner = store.load_state(&run_id).unwrap();
    assert!(owner.executor_generation.is_some_and(|g| g > bound));
    let before = snapshot(&store, &run_id);
    StaleFixture {
        _temp: temp,
        _rx: rx,
        host: host_of(runner),
        store,
        run_id,
        before,
    }
}

struct StaleFixture {
    _temp: tempfile::TempDir,
    _rx: Box<dyn std::any::Any>,
    host: WorkflowScriptHost,
    store: WorkflowStore,
    run_id: String,
    before: Snapshot,
}

/// The bytes of state.json and events.jsonl, and every file under v2/.
type Snapshot = (Vec<u8>, Vec<u8>, Vec<(String, Vec<u8>)>);

/// state.json, events.jsonl and every file under v2/.
fn snapshot(store: &WorkflowStore, run_id: &str) -> Snapshot {
    let mut v2 = Vec::new();
    let root = store.run_dir(run_id).join("v2");
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                v2.push((path.display().to_string(), std::fs::read(&path).unwrap()));
            }
        }
    }
    v2.sort();
    (
        std::fs::read(store.state_path(run_id)).unwrap(),
        std::fs::read(store.events_path(run_id)).unwrap_or_default(),
        v2,
    )
}

fn assert_stale(outcome: &archon_workflow::WorkflowResult<impl std::fmt::Debug>) {
    assert!(
        matches!(outcome, Err(WorkflowError::ControlCancelled(message))
            if message.contains("no longer owns") && message.contains("stale session")),
        "a stale session is refused by name: {outcome:?}"
    );
}

fn record(run_id: &str, id: &str) -> WorkflowV2CallRecord {
    WorkflowV2CallRecord::new(
        run_id.to_string(),
        WorkflowV2HostCall {
            id: id.into(),
            method: WorkflowV2HostMethod::Checkpoint,
            write_mode: None,
            options: Default::default(),
        },
        1,
        "hash".into(),
        WorkflowV2Result::accepted("done"),
        Vec::new(),
    )
}

/// The repro: A's script catches the control error and keeps calling after
/// B took over. Nothing A does after the takeover reaches the run.
#[tokio::test]
async fn a_stale_executor_dispatches_and_writes_nothing_after_a_resume() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, RunStatus::Running);
    let llm = Arc::new(CountingAcceptedLlm {
        calls: AtomicUsize::new(0),
    });
    let (runner, _rx) = runner(&store, &run_id, llm.clone(), None, None);
    let taken: Arc<std::sync::Mutex<Option<Snapshot>>> = Arc::default();
    let (hook_store, hook_id, hook_taken) = (store.clone(), run_id.clone(), taken.clone());
    // B takes the run over while A's first call is in flight.
    crate::command::workflow_live::workflow_live_v2::workflow_live_v2_fixed_persistence::publication_hook::install(
        store.run_dir(&run_id),
        Box::new(move || {
            take_over(&hook_store, &hook_id);
            *hook_taken.lock().unwrap() = Some(snapshot(&hook_store, &hook_id));
        }),
    );

    let outcome = runner.run(STALE_SCRIPT).await;

    let taken = taken.lock().unwrap().clone().expect("the takeover ran");
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0, "no agent dispatched");
    let after = snapshot(&store, &run_id);
    assert_eq!(after.0, taken.0, "state.json unchanged after the takeover");
    assert_eq!(
        String::from_utf8_lossy(&after.1),
        String::from_utf8_lossy(&taken.1),
        "no event appended after the takeover"
    );
    assert_eq!(after.2, taken.2, "no v2 file written after the takeover");
    let owner = store.load_state(&run_id).unwrap();
    assert_eq!(owner.status, RunStatus::Running, "B keeps the run");
    assert_stale(&outcome);
}

#[tokio::test]
async fn a_stale_executor_call_is_refused_before_it_dispatches() {
    let llm = Arc::new(CountingAcceptedLlm {
        calls: AtomicUsize::new(0),
    });
    let fixture = stale_host(llm.clone());
    for (method, id) in [("checkpoint", "late-checkpoint"), ("agent", "late-agent")] {
        let outcome = fixture
            .host
            .execute(
                method.into(),
                serde_json::json!({"id": id, "options": {"role": "analysis", "task": "t"}})
                    .to_string(),
            )
            .await;
        assert_stale(&outcome);
    }
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0);
    assert_eq!(snapshot(&fixture.store, &fixture.run_id), fixture.before);
}

#[tokio::test]
async fn a_stale_fixed_executor_reads_no_generation_as_its_own() {
    let fixture = stale_host(Arc::new(PanicLlm));
    let fixed = fixture
        .store
        .run_dir(&fixture.run_id)
        .join(crate::command::workflow_decompose_state::FIXED_STATE_PATH);
    std::fs::create_dir_all(fixed.parent().unwrap()).unwrap();
    std::fs::write(&fixed, b"{}").unwrap();
    assert_stale(&fixture.host.fixed_execution_generation());
}

#[tokio::test]
async fn a_stale_executor_publishes_no_call_record() {
    let fixture = stale_host(Arc::new(PanicLlm));
    let late = record(&fixture.run_id, "late-publish");
    let published = fixture
        .host
        .persist_generation_owned_call_and_emit(
            &late,
            crate::command::workflow_decompose_state::FixedCallProjectionKind::Executed,
            None,
        )
        .await;
    assert_stale(&published);
    // A landed write call, its generation sampled after the takeover.
    let current = fixture
        .store
        .load_state(&fixture.run_id)
        .unwrap()
        .generation;
    let landed = crate::command::workflow_live::workflow_live_v2::workflow_live_v2_fixed_persistence::persist_dispatched_call(
        &fixture.store,
        &fixture.run_id,
        &fixture.host.runner.v2_store,
        &record(&fixture.run_id, "late-landed"),
        Some(current),
        false,
    )
    .map(|_| ());
    assert_stale(&landed);
    assert_eq!(snapshot(&fixture.store, &fixture.run_id), fixture.before);
}

#[tokio::test]
async fn a_stale_executor_restores_no_history_record() {
    let fixture = stale_host(Arc::new(PanicLlm));
    let restored =
        fixture
            .host
            .restore_reused_record(&record(&fixture.run_id, "late-restore"), true, None);
    assert_stale(&restored);
    assert_eq!(snapshot(&fixture.store, &fixture.run_id), fixture.before);
}

#[tokio::test]
async fn a_stale_executor_saves_no_interruption_record() {
    let fixture = stale_host(Arc::new(PanicLlm));
    let late = record(&fixture.run_id, "late-interrupt");
    let execution = WorkflowV2CallExecution {
        call: late.call.clone(),
        input: serde_json::json!({}),
        depends_on: Vec::new(),
    };
    let saved = fixture
        .host
        .save_interrupted_call_record(
            &execution,
            "undelivered",
            &WorkflowError::NotificationDelivery("ui gone".into()),
            std::time::Duration::from_secs(1),
            1,
            "hash",
            None,
            None,
        )
        .await;
    assert_stale(&saved);
    assert_eq!(snapshot(&fixture.store, &fixture.run_id), fixture.before);
}

#[tokio::test]
async fn issue291_lifecycle_marker_text_without_host_stop_is_failed() {
    for error in [
        WorkflowError::StageFailed(TERMINAL_HOST_CALL_MARKER.into()),
        WorkflowError::SpecInvalid(format!("provider echoed {TERMINAL_HOST_CALL_MARKER}")),
        WorkflowError::TerminalHostCall("unrecorded typed stop".into()),
    ] {
        let (_temp, store, run_id) = new_run();
        set_status(&store, &run_id, RunStatus::Running);
        let (mut runner, _rx) = runner(&store, &run_id, Arc::new(PanicLlm), None, None);
        runner.initialize_repository_audit().await.unwrap();
        let host = host_of(runner);
        let summary = host.finish_lifecycle(Err(error)).await.unwrap();
        assert_eq!(summary.status, WorkflowV2Status::Failed);
        assert_eq!(summary.failed_call.as_deref(), Some("workflow.js"));
    }
}

#[tokio::test]
async fn issue291_lifecycle_recorded_host_stop_survives_unrelated_error_text() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, RunStatus::Running);
    let (mut runner, _rx) = runner(&store, &run_id, Arc::new(PanicLlm), None, None);
    runner.initialize_repository_audit().await.unwrap();
    let host = host_of(runner);
    let mut stop = record(&run_id, "gate");
    stop.status = WorkflowV2Status::NeedsReview;
    stop.result.status = WorkflowV2Status::NeedsReview;
    host.mark_terminal(&stop, "gate/result.json".into(), "review".into())
        .await;
    let summary = host
        .finish_lifecycle(Err(WorkflowError::StageFailed(
            "unrelated driver error".into(),
        )))
        .await
        .unwrap();
    assert_eq!(summary.status, WorkflowV2Status::NeedsReview);
    assert_eq!(summary.failed_call.as_deref(), Some("gate"));
}

#[tokio::test]
async fn issue291_stale_lifecycle_cannot_decide_terminal_status() {
    let fixture = stale_host(Arc::new(PanicLlm));
    for outcome in [
        Err(WorkflowError::StageFailed("failure".into())),
        Err(WorkflowError::StageFailed(TERMINAL_HOST_CALL_MARKER.into())),
        Ok(()),
    ] {
        assert_stale(&fixture.host.finish_lifecycle(outcome).await);
    }
    assert_eq!(snapshot(&fixture.store, &fixture.run_id), fixture.before);
}

#[path = "workflow_live_v2_review_race_tests.rs"]
mod review_races;

#[path = "workflow_live_v2_round3_audit_tests.rs"]
mod round3_audit;
#[path = "workflow_live_v2_round3_gate_tests.rs"]
mod round3_gates;
#[path = "workflow_live_v2_spawn_fence_tests.rs"]
mod spawned_admission;
