//! Issue 329: a script that catches every run control refusal and calls again
//! does not spin. The first refusal is sticky: the host refuses every later
//! call of the session at once, nothing more reaches the run, and the
//! executor exits with the control outcome.
use super::workflow_live_v2_reuse_verify_lineage_tests::CountingAcceptedLlm;
use super::workflow_live_v2_script_pause_tests::{
    new_run, pause_events, resume, runner, set_status,
};
use super::*;
use archon_workflow::{LifecycleAction, LifecycleController, RunStatus};

/// Catches every refusal and calls again, without end.
const SPIN_ON_CHECKPOINT: &str = r#"
async function workflow(w) {
  for (let n = 0; ; n += 1) {
    try { await w.checkpoint("spin-" + n, {}); } catch (error) {}
  }
}
"#;

/// Asks for a new pause after every refusal, without end.
const SPIN_ON_PAUSE: &str = r#"
async function workflow(w) {
  for (let n = 0; ; n += 1) {
    try { await w.pause("spin-pause-" + n, { subject: "spin", reason: "no_progress" }); }
    catch (error) {}
  }
}
"#;

/// The bound on how long the executor may take to exit once the script
/// spins: the post-stop budget plus slack. Before the fix, it never exits.
const EXIT_BOUND: std::time::Duration = std::time::Duration::from_secs(20);

/// The bytes of state.json and events.jsonl, and every file under v2/.
type Snapshot = (Vec<u8>, Vec<u8>, Vec<String>);

/// state.json, events.jsonl and every file under v2/.
fn snapshot(store: &WorkflowStore, run_id: &str) -> Snapshot {
    let mut v2 = Vec::new();
    let mut stack = vec![store.run_dir(run_id).join("v2")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                v2.push(path.display().to_string());
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

#[tokio::test]
async fn a_stale_executor_that_catches_and_retries_is_refused_at_once_and_exits() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, RunStatus::Running);
    let launched = store.load_state(&run_id).unwrap().generation;
    let llm = Arc::new(CountingAcceptedLlm {
        calls: AtomicUsize::new(0),
    });
    let (runner, _rx) = runner(&store, &run_id, llm.clone(), None, None);
    runner.v2_store.bind_session_executor(launched);
    let taken: Arc<std::sync::Mutex<Option<Snapshot>>> = Arc::default();
    let (hook_store, hook_id, hook_taken) = (store.clone(), run_id.clone(), taken.clone());
    // Executor A's first call is in flight when a resume gives the run to B.
    crate::command::workflow_live::workflow_live_v2::workflow_live_v2_fixed_persistence::publication_hook::install(
        store.run_dir(&run_id),
        Box::new(move || {
            LifecycleController::new(hook_store.clone())
                .apply(&hook_id, LifecycleAction::Pause)
                .unwrap();
            resume(&hook_store, &hook_id);
            *hook_taken.lock().unwrap() = Some(snapshot(&hook_store, &hook_id));
        }),
    );

    let outcome = tokio::time::timeout(EXIT_BOUND, runner.run(SPIN_ON_CHECKPOINT))
        .await
        .expect("the executor exits: the spinning script is stopped");
    let before = taken.lock().unwrap().clone().expect("the takeover ran");

    assert!(
        matches!(&outcome, Err(WorkflowError::ControlCancelled(message))
            if message.contains("no longer owns") && message.contains("stale session")),
        "the stale refusal is the outcome: {outcome:?}"
    );
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0, "nothing dispatched");
    assert_eq!(snapshot(&store, &run_id), before, "nothing reached the run");
    assert_eq!(
        store.load_state(&run_id).unwrap().status,
        RunStatus::Running,
        "the newer executor keeps the run"
    );
}

#[tokio::test]
async fn a_script_that_catches_its_pause_and_asks_again_takes_one_pause_and_exits() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, RunStatus::Running);
    let before = store.load_state(&run_id).unwrap().generation;
    let (runner, _rx) = runner(&store, &run_id, Arc::new(PanicLlm), None, None);

    let outcome = tokio::time::timeout(EXIT_BOUND, runner.run(SPIN_ON_PAUSE))
        .await
        .expect("the executor exits: the spinning script is stopped");

    assert!(
        matches!(&outcome, Err(WorkflowError::ControlPaused(message))
            if message.contains("spin-pause-0")),
        "the first pause is the outcome: {outcome:?}"
    );
    let run = store.load_state(&run_id).unwrap();
    assert_eq!(run.status, RunStatus::Paused);
    assert_eq!(run.generation, before + 1, "one pause transition");
    // Bounded: exactly one pause request reached the run. Before the fix,
    // every later request joined the pause with its own event and record.
    let paused = pause_events(&store, &run_id);
    assert_eq!(paused.len(), 1, "{paused:?}");
    assert_eq!(paused[0].detail["pause_id"], "spin-pause-0");
    let records = std::fs::read_dir(store.run_dir(&run_id).join("v2/script-pauses"))
        .unwrap()
        .count();
    assert_eq!(records, 1, "one pause record");
}

/// A refusal keeps its kind for every call the script issues after it
/// reached the script, and such a call reads and writes nothing. A call
/// issued with it (a sibling in the same pool) still runs.
#[tokio::test]
async fn every_call_issued_after_a_control_refusal_is_refused_with_that_refusal() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, RunStatus::Running);
    let (runner, _rx) = runner(&store, &run_id, Arc::new(PanicLlm), None, None);
    let generation = store.load_state(&run_id).unwrap().generation;
    runner.v2_store.bind_session_executor(generation);
    let host = WorkflowScriptHost {
        scaffold_hash: String::new(),
        host_occurrences: Default::default(),
        envelope_shape: ScriptEnvelopeShape::Compat,
        runner,
        accumulator: Arc::new(tokio::sync::Mutex::new(Default::default())),
        tool_host: Default::default(),
        tool_budget: Arc::new(std::sync::Mutex::new(Default::default())),
    };
    let pause = |id: &str| serde_json::json!({"id": id, "options": {"evidence": {}}}).to_string();
    let method = || archon_workflow::v2::script::SCRIPT_PAUSE_METHOD.to_string();
    // The script issues two pause requests together; the first pauses.
    let issued = std::sync::atomic::AtomicU64::new(2);
    let first = host
        .execute_issued(method(), pause("pause-1"), Some(1))
        .await;
    assert!(
        matches!(&first, Err(WorkflowError::ControlPaused(_))),
        "{first:?}"
    );
    host.note_delivered(&first, &issued).await;
    let sibling = host
        .execute_issued(method(), pause("pause-2"), Some(2))
        .await;
    assert!(
        matches!(&sibling, Err(WorkflowError::ControlPaused(message)) if message.contains("joining")),
        "the sibling issued with it joins the pause: {sibling:?}"
    );
    assert_eq!(pause_events(&store, &run_id).len(), 2);
    // The operator resumes; the session whose script saw the pause stops.
    resume(&store, &run_id);
    let before = snapshot(&store, &run_id);
    for (order, (method, id)) in [(3, ("checkpoint", "after-1")), (4, ("agent", "after-2"))] {
        let later = host
            .execute_issued(
                method.into(),
                serde_json::json!({"id": id, "options": {"role": "analysis", "task": "t"}})
                    .to_string(),
                Some(order),
            )
            .await;
        assert!(
            matches!(&later, Err(WorkflowError::ControlPaused(message))
                if message.contains("already received that run control refusal")),
            "{later:?}"
        );
    }
    assert_eq!(snapshot(&store, &run_id), before, "nothing reached the run");
    assert!(host.accumulator.lock().await.session_stopped());
}
