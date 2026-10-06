//! Issue 337: the pause a fixed script's host takes on an unplanned error
//! covers the attempts the run recorded, exactly as a `w.pause` does. A
//! resume replays each covered answer verbatim, whatever its verdict -- an
//! unpublished gate refusal, a failed raw author call -- while its slot
//! still holds that attempt and the call asks with the input it was recorded
//! with. Re-asking would let a judge replace its authoritative answer.

use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor;

/// Answers every gate with an unpublished refusal: exit 1, no receipt.
struct RefusingGate {
    calls: Arc<AtomicUsize>,
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

    async fn execute(
        &self,
        _request: archon_workflow::HostCommandRequest,
        _expected_generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        let answer = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
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
    gate: Arc<AtomicUsize>,
    author: Arc<AtomicUsize>,
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
        calls: counters.gate.clone(),
    }))
    .with_raw_outcomes(true);
    (runner, ui)
}

/// The gate answers, a raw author call fails, then the script crashes.
const GATE_AUTHOR_CRASH: &str = r#"
async function workflow(w) {
  const gate = await w.hostCommand("task-set-lint", { stdin: null });
  const authored = await w.agent("acceptance-author-1", {
    task: args.task, tier: "planner", resultMode: "rawOutcome"
  });
  if (args.crash) throw new Error("progress delivery failed after the gate answered");
  return JSON.stringify({ gate: gate.stdout, author: authored.status });
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

async fn crash(store: &WorkflowStore, run_id: &str, counters: &Counters, task: &str) {
    let (runner, _ui) = fixed_runner(
        store,
        run_id,
        counters,
        serde_json::json!({"crash": true, "task": task}),
    );
    let error = runner.run(GATE_AUTHOR_CRASH).await.unwrap_err();
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    resume(store, run_id);
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_crash_resume_replays_the_unpublished_refusal_and_the_failed_author_call() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run_id = new_run(&store);
    let counters = counters();
    crash(&store, &run_id, &counters, "Author the contract").await;
    assert_eq!(counters.gate.load(Ordering::SeqCst), 1);
    let authored = counters.author.load(Ordering::SeqCst);

    let (runner, _ui) = fixed_runner(
        &store,
        &run_id,
        &counters,
        serde_json::json!({"crash": false, "task": "Author the contract"}),
    );
    let summary = runner.run(GATE_AUTHOR_CRASH).await.expect("resumed run");

    assert_eq!(
        counters.gate.load(Ordering::SeqCst),
        1,
        "the judge answered once; its refusal is replayed, never re-asked"
    );
    assert_eq!(
        counters.author.load(Ordering::SeqCst),
        authored,
        "the failed author call is replayed, never re-asked"
    );
    assert_eq!(summary.reused, 2, "{summary:?}");
    let result = script_result(&summary);
    assert_eq!(result["gate"], "refusal 1", "{result}");
    assert_eq!(result["author"], "failed", "{result}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_crash_resume_replays_nothing_asked_differently_or_invalidated() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run_id = new_run(&store);
    let counters = counters();
    crash(&store, &run_id, &counters, "Author the contract").await;
    let authored = counters.author.load(Ordering::SeqCst);

    // Asked with another input: the author runs live; the gate, asked the
    // same, is replayed. The script crashes again.
    crash(&store, &run_id, &counters, "Author a different contract").await;
    assert_eq!(
        counters.gate.load(Ordering::SeqCst),
        1,
        "the gate refusal is replayed"
    );
    assert!(
        counters.author.load(Ordering::SeqCst) > authored,
        "a changed input is a new question"
    );

    // A restart invalidated the refusal: it is asked again.
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"));
    v2_store
        .invalidate_call_and_dependents(&[], "host-command:task-set-lint:fixed")
        .unwrap();
    let (runner, _ui) = fixed_runner(
        &store,
        &run_id,
        &counters,
        serde_json::json!({"crash": false, "task": "Author a different contract"}),
    );
    let summary = runner.run(GATE_AUTHOR_CRASH).await.expect("resumed run");
    assert_eq!(
        counters.gate.load(Ordering::SeqCst),
        2,
        "an invalidated answer is never replayed"
    );
    let result = script_result(&summary);
    assert_eq!(result["gate"], "refusal 2", "{result}");
}
