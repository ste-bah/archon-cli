//! Issue 364: a starved script thread, and a provider silent for a whole
//! no-progress window, pause the run with their evidence; a healthy long host
//! call is never cut.
use super::workflow_live_v2_script_pause_tests::{events, new_run, set_status};
use super::workflow_live_v2_script_starvation::{
    STARVATION_RECORDS, STARVATION_WINDOW, WATCHDOG_PAUSED_RUNS, write_record,
};
use super::*;

/// A slow agent call in flight, one completed checkpoint, then `tail`.
fn starving_script(tail: &str) -> String {
    format!(
        r#"
async function workflow(w) {{
  const slow = w.agent("slow-agent", {{ role: "analysis", task: "Return an accepted result slowly" }});
  await w.checkpoint("before-tail", {{ note: "the slow call is in flight" }});
  {tail}
}}
"#
    )
}

fn runner_with(
    store: &WorkflowStore,
    run_id: &str,
    llm: Arc<dyn WorkflowLlmClient>,
) -> (WorkflowV2ScriptRunner, Box<dyn std::any::Any>) {
    let spec = test_spec();
    let (ui_sink, rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(llm, ui_sink, Vec::new(), run_id.into(), None, None);
    let runner = WorkflowV2ScriptRunner::new(
        "starvation".into(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2")),
        store.clone(),
        run_id.into(),
        true,
        None,
        None,
    );
    (runner, Box::new(rx))
}

/// Runs `script` with the CPU watchdog paused when `watchdog_paused`, as it
/// is while a host call is in flight; `None` if the run did not end in 30 s.
/// The run has its own thread and runtime, so a run that never ends (the
/// failure before the fix) fails this test instead of hanging the process.
fn run_bounded(
    store: &WorkflowStore,
    run_id: &str,
    llm: Arc<dyn WorkflowLlmClient>,
    script: String,
    watchdog_paused: bool,
) -> Option<Result<WorkflowV2ScriptSummary, WorkflowError>> {
    if watchdog_paused {
        WATCHDOG_PAUSED_RUNS
            .lock()
            .unwrap()
            .insert(run_id.to_string());
    }
    // The UI receiver stays open on this thread while the run does.
    let (runner, _ui) = runner_with(store, run_id, llm);
    let (done, outcome) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("runtime");
        let _ = done.send(runtime.block_on(runner.run(&script)));
    });
    let outcome = outcome.recv_timeout(Duration::from_secs(30)).ok();
    WATCHDOG_PAUSED_RUNS.lock().unwrap().remove(run_id);
    outcome
}

fn slow_llm(delay: Duration) -> Arc<dyn WorkflowLlmClient> {
    Arc::new(SlowAcceptedLlm { delay })
}

/// Every starvation record of the run, in the order they were written.
fn starvation_records(store: &WorkflowStore, run_id: &str) -> Vec<serde_json::Value> {
    let Ok(entries) = std::fs::read_dir(store.run_dir(run_id).join(STARVATION_RECORDS)) else {
        return Vec::new();
    };
    let mut paths = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .iter()
        .filter_map(|path| serde_json::from_slice(&std::fs::read(path).ok()?).ok())
        .collect()
}

/// The latest starvation record of the run.
fn starvation_record(store: &WorkflowStore, run_id: &str) -> Option<serde_json::Value> {
    starvation_records(store, run_id).pop()
}

/// Fails before the fix: every record went to one file, so a later
/// "suspected" or "recovered" record replaced the evidence of a cut.
#[test]
fn every_starvation_record_is_kept() {
    let (_tmp, store, run_id) = new_run();
    let record = |state: &str| archon_workflow::v2::script::script_thread_heartbeat::Starvation {
        detected_by: "monitor".into(),
        state: state.into(),
        no_progress_ms: 1,
        window_ms: 1,
        script_thread_cpu_ms: None,
        process_cpu_ms: None,
        in_flight: vec![serde_json::json!({"id": state})],
    };
    for state in ["cut", "suspected", "recovered"] {
        write_record(&store, &run_id, &record(state));
    }
    let states = starvation_records(&store, &run_id)
        .iter()
        .map(|record| record["state"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    assert_eq!(states, ["cut", "suspected", "recovered"]);
}

/// The run paused (never failed) on a starved thread, with the record.
fn assert_starvation_pause(
    store: &WorkflowStore,
    run_id: &str,
    outcome: Option<Result<WorkflowV2ScriptSummary, WorkflowError>>,
    detected_by: &str,
) {
    let outcome = outcome.expect("a starved script thread must not hang the run");
    assert!(
        matches!(&outcome, Err(WorkflowError::ControlPaused(message))
            if message.contains("script thread starved") && message.contains("slow-agent")),
        "a starved thread pauses, never fails: {outcome:?}"
    );
    let run = store.load_state(run_id).unwrap();
    assert_eq!(run.status, archon_workflow::RunStatus::Paused);
    let record = starvation_record(store, run_id).expect("the durable starvation record");
    assert_eq!(record["state"], "cut", "{record}");
    assert_eq!(record["detected_by"], detected_by, "{record}");
    assert!(
        record["in_flight"]
            .as_array()
            .is_some_and(|calls| calls.iter().any(|call| call["id"] == "slow-agent")),
        "the record names what was in flight: {record}"
    );
    let pause = events(store, run_id)
        .into_iter()
        .find(|event| event.detail["event"] == "script_error_pause")
        .expect("a pause event");
    assert_eq!(pause.detail["cause"], "script_thread_starved");
    // Kept on a recurrence too, whose `cause` is `recurring_script_error`.
    assert_eq!(pause.detail["stop_cause"], "script_thread_starved");
}

/// Fails before the fix: the run never ends (the watchdog is paused for the
/// in-flight call, and the job loop never lets the call be polled again).
#[test]
fn a_microtask_loop_during_an_in_flight_call_pauses_with_the_evidence() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    let script = starving_script("for (;;) { await null; }");
    let outcome = run_bounded(
        &store,
        &run_id,
        slow_llm(Duration::from_secs(60)),
        script,
        true,
    );
    assert_starvation_pause(&store, &run_id, outcome, "interrupt_handler");
}

/// Fails before the fix the same way: a synchronous loop with a call in flight.
#[test]
fn a_pure_busy_loop_during_an_in_flight_call_pauses_with_the_evidence() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    let script = starving_script("for (;;) {}");
    let outcome = run_bounded(
        &store,
        &run_id,
        slow_llm(Duration::from_secs(60)),
        script,
        true,
    );
    assert_starvation_pause(&store, &run_id, outcome, "interrupt_handler");
}

/// With the watchdog running, its cut names the starved calls too (before the
/// fix: a script-error pause that said nothing of what was in flight).
#[test]
fn a_watchdog_cut_with_calls_in_flight_is_recorded_as_starvation() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    let script = starving_script("for (;;) { await null; }");
    let outcome = run_bounded(
        &store,
        &run_id,
        slow_llm(Duration::from_secs(60)),
        script,
        false,
    );
    assert_starvation_pause(&store, &run_id, outcome, "cpu_watchdog");
}

/// A host call far longer than the no-progress window is never cut, even with
/// only the heartbeat watching: the runtime is idle in it.
#[test]
fn a_healthy_long_host_call_is_never_cut() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    let script = starving_script(
        r#"const answer = await slow;
  await w.checkpoint("after-slow", { summary: String(answer.summary) });"#,
    );
    let outcome = run_bounded(
        &store,
        &run_id,
        slow_llm(STARVATION_WINDOW * 4),
        script,
        true,
    )
    .expect("the run ends");
    let summary = outcome.expect("a healthy long call completes");
    assert_eq!(summary.status, WorkflowV2Status::Accepted);
    assert!(starvation_record(&store, &run_id).is_none());
}

/// Every agent call ends with the provider's transport stall.
struct StalledProviderLlm;

#[async_trait::async_trait]
impl WorkflowLlmClient for StalledProviderLlm {
    async fn continue_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.run_agent(call).await
    }

    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        Err(WorkflowError::StageFailed(format!(
            "subagent failed: {} subagent stream retry exhausted: no answer from the provider (last: resend could not open); prior conversation retained",
            archon_workflow::error::TRANSPORT_STALL_MARKER
        )))
    }
}

/// Fails before the fix: the stall became a failed call (and the session was
/// re-asked at once, twice, while the network was still down).
#[test]
fn a_provider_silent_for_a_whole_window_pauses_the_run() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    let script = r#"
async function workflow(w) {
  await w.agent("stalled-agent", { role: "analysis", task: "Answer once the provider is reachable" });
  return { finished: true };
}
"#;
    let outcome = run_bounded(
        &store,
        &run_id,
        Arc::new(StalledProviderLlm),
        script.to_string(),
        false,
    )
    .expect("the run ends");
    assert!(
        matches!(&outcome, Err(WorkflowError::ControlPaused(message))
            if message.contains("stalled-agent") && message.contains("no answer")),
        "a transport stall pauses, never fails: {outcome:?}"
    );
    let run = store.load_state(&run_id).unwrap();
    assert_eq!(run.status, archon_workflow::RunStatus::Paused);
    assert!(
        events(&store, &run_id)
            .iter()
            .any(|event| event.detail["event"] == "transport_stall_pause"),
        "the pause carries its evidence"
    );
}
