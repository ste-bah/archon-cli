//! Issue 337: the pause a fixed script's host takes on an unplanned error
//! covers the VERDICTS the run recorded, exactly as a `w.pause` covers its
//! attempts. A resume replays each covered verdict verbatim -- an
//! unpublished gate refusal too -- while its own slot still holds that
//! attempt and the call asks with the input it was recorded with: re-asking
//! would let a judge replace its authoritative answer. A failed dispatch
//! (a provider or host fault) carries no verdict and is asked again. One
//! covered call that changed never voids the replay of the others.

use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor;

/// Answers every gate with an unpublished refusal (exit 1, no receipt) and
/// logs the command it ran.
struct RefusingGate {
    ran: Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl WorkflowHostCommandExecutor for RefusingGate {
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!("host-command:{}:fixed", request.command_id))
    }

    fn record_is_reusable(
        &self,
        _record: &archon_workflow::WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        Ok(true)
    }

    /// Every gate here judges the same, unchanging content.
    fn judged_inputs(
        &self,
        _request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<Option<String>> {
        Ok(Some("unchanged task root".into()))
    }

    async fn execute(
        &self,
        request: archon_workflow::HostCommandRequest,
        _expected_generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        let answer = {
            let mut ran = self.ran.lock().unwrap();
            ran.push(request.command_id.clone());
            ran.len()
        };
        Ok(archon_workflow::HostCommandResult {
            exit_code: Some(1),
            stdout: format!("refusal {answer}"),
            stderr: String::new(),
            stdout_bytes: 9,
            stderr_bytes: 0,
            timed_out: false,
            interrupted: false,
            stdout_truncated: false,
            stderr_truncated: false,
            gate_envelope: Some(archon_workflow::GateEnvelopeV1 {
                schema_version: archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION,
                report: serde_json::json!("refused"),
                policy_findings: Vec::new(),
                operational_error: None,
            }),
            publication_receipt: None,
            subjects: Vec::new(),
            postcondition: None,
        })
    }
}

/// Fails every raw author call, as a provider refusal does.
struct FailingAuthor {
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for FailingAuthor {
    async fn continue_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.run_agent(call).await
    }

    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        panic!("a raw author call uses run_agent")
    }

    async fn run_agent(
        &self,
        _request: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(WorkflowError::StageFailed(
            "provider refused the author call".into(),
        ))
    }
}

struct Counters {
    gate: Arc<std::sync::Mutex<Vec<String>>>,
    author: Arc<AtomicUsize>,
}

impl Counters {
    fn ran(&self) -> Vec<String> {
        self.gate.lock().unwrap().clone()
    }
}

fn fixed_runner(
    store: &WorkflowStore,
    run_id: &str,
    counters: &Counters,
    args: serde_json::Value,
) -> (WorkflowV2ScriptRunner, impl Sized) {
    let spec = test_spec();
    let (ui_sink, ui) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(FailingAuthor {
            calls: counters.author.clone(),
        }),
        ui_sink,
        Vec::new(),
        run_id.to_string(),
        None,
        Some(1_500),
    )
    .with_fixed_raw_tool_policy(vec!["Read".into()]);
    let runner = WorkflowV2ScriptRunner::new(
        "crash replay".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2")),
        store.clone(),
        run_id.to_string(),
        true,
        None,
        Some(args),
    )
    .with_host_command_executor(Arc::new(RefusingGate {
        ran: counters.gate.clone(),
    }))
    .with_raw_outcomes(true);
    (runner, ui)
}

/// The gate answers, a raw author call fails, then the script crashes.
/// Two gates answer, a raw author call fails to dispatch, then the script
/// crashes.
const GATES_AUTHOR_CRASH: &str = r#"
async function workflow(w) {
  const first = await w.hostCommand("task-set-lint", { stdin: args.first });
  const second = await w.hostCommand("task-set-gate", { stdin: null });
  const authored = await w.agent("acceptance-author-1", {
    task: "Author the contract", tier: "planner", resultMode: "rawOutcome"
  });
  if (args.crash) throw new Error("progress delivery failed after the gates answered");
  return JSON.stringify({ first: first.stdout, second: second.stdout, author: authored.status });
}
"#;

fn new_run(store: &WorkflowStore) -> String {
    store.create_run(test_spec()).expect("run").id
}

fn resume(store: &WorkflowStore, run_id: &str) {
    archon_workflow::LifecycleController::new(store.clone())
        .apply(run_id, archon_workflow::LifecycleAction::Resume)
        .expect("resume");
}

fn args(crash: bool, first: &str) -> serde_json::Value {
    serde_json::json!({"crash": crash, "first": first})
}

async fn crash(store: &WorkflowStore, run_id: &str, counters: &Counters) {
    let (runner, _ui) = fixed_runner(store, run_id, counters, args(true, "candidate one"));
    let error = runner.run(GATES_AUTHOR_CRASH).await.unwrap_err();
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    resume(store, run_id);
}

async fn finish(
    store: &WorkflowStore,
    run_id: &str,
    counters: &Counters,
    first: &str,
) -> serde_json::Value {
    let (runner, _ui) = fixed_runner(store, run_id, counters, args(false, first));
    let summary = runner.run(GATES_AUTHOR_CRASH).await.expect("resumed run");
    script_result(&summary)
}

/// The object the script returned (its result is the JSON of the string).
fn script_result(summary: &WorkflowV2ScriptSummary) -> serde_json::Value {
    let raw = summary.script_result.as_deref().expect("script result");
    match serde_json::from_str::<serde_json::Value>(raw).expect("result json") {
        serde_json::Value::String(text) => serde_json::from_str(&text).expect("returned json"),
        value => value,
    }
}

fn counters() -> Counters {
    Counters {
        gate: Arc::default(),
        author: Arc::default(),
    }
}

fn setup() -> (tempfile::TempDir, WorkflowStore, String, Counters) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run_id = new_run(&store);
    (temp, store, run_id, counters())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_crash_resume_replays_the_refusals_and_reasks_the_failed_dispatch() {
    let (_temp, store, run_id, counters) = setup();
    crash(&store, &run_id, &counters).await;
    assert_eq!(counters.ran(), ["task-set-lint", "task-set-gate"]);
    let authored = counters.author.load(Ordering::SeqCst);
    assert!(authored >= 1);

    let result = finish(&store, &run_id, &counters, "candidate one").await;

    assert_eq!(
        counters.ran(),
        ["task-set-lint", "task-set-gate"],
        "each judge answered once; its refusal is replayed, never re-asked"
    );
    assert_eq!(result["first"], "refusal 1", "{result}");
    assert_eq!(result["second"], "refusal 2", "{result}");
    assert!(
        counters.author.load(Ordering::SeqCst) > authored,
        "a failed dispatch carries no verdict: it is asked again"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_changed_first_call_never_voids_the_replay_of_a_later_one() {
    let (_temp, store, run_id, counters) = setup();
    crash(&store, &run_id, &counters).await;

    // The first gate is asked with another candidate: a new question, run
    // live. The second gate, asked the same, still replays.
    let result = finish(&store, &run_id, &counters, "candidate two").await;

    assert_eq!(
        counters.ran(),
        ["task-set-lint", "task-set-gate", "task-set-lint"],
        "only the changed call runs again"
    );
    assert_eq!(result["first"], "refusal 3", "{result}");
    assert_eq!(result["second"], "refusal 2", "{result}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_invalidated_call_runs_again_and_the_others_still_replay() {
    let (_temp, store, run_id, counters) = setup();
    crash(&store, &run_id, &counters).await;
    // A restart invalidated the second refusal only.
    WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"))
        .invalidate_call_and_dependents(&[], "host-command:task-set-gate:fixed")
        .unwrap();

    let result = finish(&store, &run_id, &counters, "candidate one").await;

    assert_eq!(
        counters.ran(),
        ["task-set-lint", "task-set-gate", "task-set-gate"],
        "an invalidated answer is asked again; the first is replayed"
    );
    assert_eq!(result["first"], "refusal 1", "{result}");
    assert_eq!(result["second"], "refusal 3", "{result}");
}

const STOP_ON_REFUSAL: &str = r#"
async function workflow(w) {
  const gate = await w.hostCommand("task-set-lint", { stdin: null });
  if (gate.exitCode !== 0) {
    await __archonHost("terminalStop", JSON.stringify({ schemaVersion: 1, reason: "refused: " + gate.stdout }));
  }
  return "accepted";
}
"#;

/// Round 3 (review finding 2): the stop was recorded, then the executor died
/// before its finalization (or the finalization failed). Stale-owner recovery
/// moved the run on and paused it. A resume replays the verdict that decided
/// the stop and reaches the same stop; the judge is never asked again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_whose_finalization_was_lost_replays_its_verdict_on_resume() {
    let (_temp, store, run_id, counters) = setup();
    let (runner, _ui) = fixed_runner(&store, &run_id, &counters, serde_json::json!({}));
    let summary = runner.run(STOP_ON_REFUSAL).await.expect("deliberate stop");
    assert_eq!(summary.status, WorkflowV2Status::Failed);
    assert_eq!(summary.script_error.as_deref(), Some("refused: refusal 1"));
    // No finalization: the process died. Stale-owner recovery's transition.
    let mut run = store.load_state(&run_id).unwrap();
    run.generation += 1;
    run.status = archon_workflow::RunStatus::Paused;
    store.save_state(&run).unwrap();
    resume(&store, &run_id);

    let (runner, _ui) = fixed_runner(&store, &run_id, &counters, serde_json::json!({}));
    let summary = runner.run(STOP_ON_REFUSAL).await.expect("the same stop");

    assert_eq!(counters.ran(), ["task-set-lint"], "the verdict is replayed");
    assert_eq!(summary.status, WorkflowV2Status::Failed);
    assert_eq!(summary.script_error.as_deref(), Some("refused: refusal 1"));
}

const REFUSAL_AUTHOR_FINAL_REPORT: &str = r#"
async function workflow(w) {
  const gate = await w.hostCommand("task-set-lint", { stdin: null });
  const authored = await w.agent("acceptance-author-1", {
    task: "Author the contract", tier: "planner", resultMode: "rawOutcome"
  });
  await w.finalReport("stopped", {
    status: "needs_review", inputs: { gate: gate.stdout, author: authored.status }, task: "Stop for review"
  });
}
"#;

/// Round 5 (review findings 2 and 5): a final report's stop that cannot be
/// persisted pauses the run with the coverage of the verdicts it holds. The
/// resume after the store is repaired replays the gate's refusal that decided
/// the stop (never re-asked), asks the failed author dispatch again (it holds
/// no verdict), and reaches the same stop, persisted this time.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unpersisted_stop_pause_replays_the_deciding_verdict_on_resume() {
    let (_temp, store, run_id, counters) = setup();
    let held = store.run_dir(&run_id).join("v2/terminal-stop.json");
    std::fs::create_dir_all(held.join("held")).unwrap();
    let (runner, _ui) = fixed_runner(&store, &run_id, &counters, serde_json::json!({}));
    let paused = runner.run(REFUSAL_AUTHOR_FINAL_REPORT).await;
    assert!(
        matches!(paused, Err(WorkflowError::ControlPaused(_))),
        "{paused:?}"
    );
    let coverage = std::fs::read_dir(store.run_dir(&run_id).join("v2/script-pauses"))
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("host-terminal-stop-unpersisted-g")
        })
        .count();
    assert_eq!(coverage, 1, "the pause records what it covers");
    let authored = counters.author.load(Ordering::SeqCst);
    // The pause keeps an unreadable stop record aside; the store is repaired.
    if held.exists() {
        std::fs::remove_dir_all(&held).unwrap();
    }
    resume(&store, &run_id);

    let (runner, _ui) = fixed_runner(&store, &run_id, &counters, serde_json::json!({}));
    let summary = runner
        .run(REFUSAL_AUTHOR_FINAL_REPORT)
        .await
        .expect("the same stop");

    assert_eq!(counters.ran(), ["task-set-lint"], "the refusal is replayed");
    assert!(
        counters.author.load(Ordering::SeqCst) > authored,
        "a failed dispatch carries no verdict: it is asked again"
    );
    assert_eq!(summary.status, WorkflowV2Status::NeedsReview);
    assert_eq!(summary.failed_call.as_deref(), Some("stopped"));
    assert!(held.is_file(), "the stop is persisted on the resume");
    assert_ne!(
        store.load_state(&run_id).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
}

/// Round 5: a final report's stop was persisted, then its finalization was
/// lost and stale-owner recovery paused the run. The resume replays the
/// covered final report, and the replay stops the script again: it never
/// goes on past the stop.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replayed_final_report_stop_stops_the_script_again() {
    let (_temp, store, run_id, counters) = setup();
    let (runner, _ui) = fixed_runner(&store, &run_id, &counters, serde_json::json!({}));
    let summary = runner
        .run(REFUSAL_AUTHOR_FINAL_REPORT)
        .await
        .expect("the stop");
    assert_eq!(summary.status, WorkflowV2Status::NeedsReview);
    // No finalization: the process died. Stale-owner recovery's transition.
    let mut run = store.load_state(&run_id).unwrap();
    run.generation += 1;
    run.status = archon_workflow::RunStatus::Paused;
    store.save_state(&run).unwrap();
    resume(&store, &run_id);

    let (runner, _ui) = fixed_runner(&store, &run_id, &counters, serde_json::json!({}));
    let summary = runner
        .run(REFUSAL_AUTHOR_FINAL_REPORT)
        .await
        .expect("the same stop");

    assert_eq!(counters.ran(), ["task-set-lint"], "the refusal is replayed");
    assert_eq!(summary.status, WorkflowV2Status::NeedsReview);
    assert_eq!(summary.failed_call.as_deref(), Some("stopped"));
}
