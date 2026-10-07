//! Issue 337 round 6 (review finding 1): an unpublished gate refusal that a
//! host-taken pause covers replays while what its gate reads is what it was
//! AT THE PAUSE. The run's own progress after the refusal (later
//! publications, a sibling landing while the gate ran, a call the resumed
//! run makes live before it reaches the refusal) never voids the replay. An
//! operator's change to the task root after the pause always does.

use super::*;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor;

/// A task root as one version counter: the freeze gate refuses (and, when
/// `sibling_lands`, a sibling body lands while it runs); the body gate
/// publishes a new version (and fails to dispatch once when
/// `body_dispatch_fails`). Every gate reads the whole task root.
struct TaskRootGate {
    root: Arc<AtomicUsize>,
    ran: Arc<std::sync::Mutex<Vec<String>>>,
    sibling_lands: bool,
    body_dispatch_fails: Arc<AtomicBool>,
}

fn outcome(
    command_id: &str,
    exit_code: i32,
    stdout: String,
    published: bool,
) -> archon_workflow::HostCommandResult {
    archon_workflow::HostCommandResult {
        exit_code: Some(exit_code),
        stdout_bytes: stdout.len() as u64,
        stdout,
        stderr: String::new(),
        stderr_bytes: 0,
        timed_out: false,
        interrupted: false,
        stdout_truncated: false,
        stderr_truncated: false,
        gate_envelope: Some(archon_workflow::GateEnvelopeV1 {
            schema_version: archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION,
            report: serde_json::json!(if published { "passed" } else { "refused" }),
            policy_findings: Vec::new(),
            operational_error: None,
        }),
        publication_receipt: published.then(|| archon_workflow::PublicationReceiptV1 {
            schema_version: archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION,
            call_id: format!("host-command:{command_id}:fixed"),
            command_id: command_id.to_string(),
            entries: Vec::new(),
            committed_at: "2026-10-07T00:00:00Z".into(),
        }),
        subjects: Vec::new(),
        postcondition: published.then(|| archon_workflow::CommandPostconditionEvaluation {
            satisfied: true,
            summary: "fixture postcondition".into(),
        }),
    }
}

#[async_trait::async_trait]
impl WorkflowHostCommandExecutor for TaskRootGate {
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!("host-command:{}:fixed", request.command_id))
    }

    fn record_is_reusable(
        &self,
        record: &archon_workflow::WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        Ok(record.status == WorkflowV2Status::Accepted)
    }

    fn judged_inputs(
        &self,
        _request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<Option<String>> {
        Ok(Some(format!(
            "task-root-v{}",
            self.root.load(Ordering::SeqCst)
        )))
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
        if request.command_id == "land-task-body" {
            if self.body_dispatch_fails.swap(false, Ordering::SeqCst) {
                return Err(WorkflowError::StageFailed(
                    "the body gate could not be reached".into(),
                ));
            }
            self.root.fetch_add(1, Ordering::SeqCst);
            return Ok(outcome(
                &request.command_id,
                0,
                format!("landed {answer}"),
                true,
            ));
        }
        if self.sibling_lands {
            self.root.fetch_add(1, Ordering::SeqCst);
        }
        Ok(outcome(
            &request.command_id,
            1,
            format!("refusal {answer}"),
            false,
        ))
    }
}

/// No agent call is made by these scripts.
struct NoAgent;

#[async_trait::async_trait]
impl WorkflowLlmClient for NoAgent {
    async fn continue_agent(
        &self,
        _call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        panic!("no agent call")
    }

    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        panic!("no agent call")
    }

    async fn run_agent(
        &self,
        _request: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        panic!("no agent call")
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    store: WorkflowStore,
    run_id: String,
    root: Arc<AtomicUsize>,
    ran: Arc<std::sync::Mutex<Vec<String>>>,
    body_dispatch_fails: Arc<AtomicBool>,
    sibling_lands: bool,
}

impl Fixture {
    fn new(sibling_lands: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(temp.path().join("workflows"));
        let run_id = store.create_run(test_spec()).expect("run").id;
        Self {
            _temp: temp,
            store,
            run_id,
            root: Arc::default(),
            ran: Arc::default(),
            body_dispatch_fails: Arc::default(),
            sibling_lands,
        }
    }

    fn runner(&self, crash: bool) -> (WorkflowV2ScriptRunner, impl Sized) {
        let spec = test_spec();
        let (ui_sink, ui) = default_workflow_ui_sink();
        let client = LiveV2AgentClient::new(
            Arc::new(NoAgent),
            ui_sink,
            Vec::new(),
            self.run_id.clone(),
            None,
            Some(1_500),
        );
        let runner = WorkflowV2ScriptRunner::new(
            "judged resume".to_string(),
            test_runtime(&spec),
            WorkflowV2AgentAdapter::new(),
            client,
            WorkflowV2ResultStore::new(self.store.run_dir(&self.run_id).join("v2")),
            self.store.clone(),
            self.run_id.clone(),
            true,
            None,
            Some(serde_json::json!({ "crash": crash })),
        )
        .with_host_command_executor(Arc::new(TaskRootGate {
            root: self.root.clone(),
            ran: self.ran.clone(),
            sibling_lands: self.sibling_lands,
            body_dispatch_fails: self.body_dispatch_fails.clone(),
        }))
        .with_raw_outcomes(true);
        (runner, ui)
    }

    /// The script crashes after its calls; the run pauses; it is resumed.
    async fn crash(&self, script: &str) {
        let (runner, _ui) = self.runner(true);
        let error = runner.run(script).await.unwrap_err();
        assert!(
            matches!(error, WorkflowError::ControlPaused(_)),
            "{error:?}"
        );
        archon_workflow::LifecycleController::new(self.store.clone())
            .apply(&self.run_id, archon_workflow::LifecycleAction::Resume)
            .expect("resume");
    }

    async fn finish(&self, script: &str) -> serde_json::Value {
        let (runner, _ui) = self.runner(false);
        let summary = runner.run(script).await.expect("resumed run");
        let raw = summary.script_result.as_deref().expect("script result");
        match serde_json::from_str::<serde_json::Value>(raw).expect("result json") {
            serde_json::Value::String(text) => serde_json::from_str(&text).expect("returned json"),
            value => value,
        }
    }

    fn ran(&self) -> Vec<String> {
        self.ran.lock().unwrap().clone()
    }
}

/// The freeze gate refuses, then the run goes on and publishes a body.
const REFUSAL_THEN_PROGRESS: &str = r#"
async function workflow(w) {
  const refused = await w.hostCommand("freeze-acceptance", { stdin: "candidate" });
  const landed = await w.hostCommand("land-task-body", { stdin: "body" });
  if (args.crash) throw new Error("the run crashed after later progress");
  return JSON.stringify({ refused: refused.stdout, landed: landed.stdout });
}
"#;

/// The run published after the refusal, then crashed. The task root at the
/// resume is the one the pause left: the refusal replays, never re-asked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refusal_replays_after_the_run_published_later_work() {
    let fixture = Fixture::new(false);
    fixture.crash(REFUSAL_THEN_PROGRESS).await;
    assert_eq!(fixture.ran(), ["freeze-acceptance", "land-task-body"]);

    let result = fixture.finish(REFUSAL_THEN_PROGRESS).await;

    assert_eq!(
        fixture.ran(),
        ["freeze-acceptance", "land-task-body"],
        "the judge is never asked again about a task root the run itself moved on"
    );
    assert_eq!(result["refused"], "refusal 1", "{result}");
    assert_eq!(result["landed"], "landed 2", "{result}");
}

/// The operator changed the task root after the pause -- here back to the
/// content the refusal judged. The task root is not the one the pause left,
/// so the refusal is asked again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refusal_is_asked_again_after_the_operator_changed_the_task_root() {
    let fixture = Fixture::new(false);
    fixture.crash(REFUSAL_THEN_PROGRESS).await;
    fixture.root.store(0, Ordering::SeqCst);

    let result = fixture.finish(REFUSAL_THEN_PROGRESS).await;

    assert_eq!(
        fixture.ran(),
        ["freeze-acceptance", "land-task-body", "freeze-acceptance"],
        "a changed task root is a new question"
    );
    assert_eq!(result["refused"], "refusal 3", "{result}");
}

/// A sibling body landed while the freeze gate ran, so the task root before
/// and after that call differ. The pause still records the task root it
/// left, and the refusal replays.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refusal_replays_after_a_sibling_landed_while_its_gate_ran() {
    let fixture = Fixture::new(true);
    fixture.crash(REFUSAL_THEN_PROGRESS).await;
    assert_eq!(fixture.root.load(Ordering::SeqCst), 2);

    let result = fixture.finish(REFUSAL_THEN_PROGRESS).await;

    assert_eq!(fixture.ran(), ["freeze-acceptance", "land-task-body"]);
    assert_eq!(result["refused"], "refusal 1", "{result}");
}

/// The body gate could not be reached (no verdict, not covered), then the
/// freeze gate refused, then the run crashed.
const FAULT_THEN_REFUSAL: &str = r#"
async function workflow(w) {
  const landed = await w.hostCommand("land-task-body", { stdin: "body" });
  const refused = await w.hostCommand("freeze-acceptance", { stdin: "candidate" });
  if (args.crash) throw new Error("the run crashed after the refusal");
  return JSON.stringify({ refused: refused.stdout, landed: landed.stdout });
}
"#;

/// The resumed run asks the body gate again and it publishes BEFORE the run
/// reaches the covered refusal. The reference is the task root when the
/// resumed run started, so that progress never voids the refusal.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_call_of_the_resumed_run_never_voids_a_later_covered_refusal() {
    let fixture = Fixture::new(false);
    fixture.body_dispatch_fails.store(true, Ordering::SeqCst);
    fixture.crash(FAULT_THEN_REFUSAL).await;
    assert_eq!(fixture.ran(), ["land-task-body", "freeze-acceptance"]);
    assert_eq!(fixture.root.load(Ordering::SeqCst), 0);

    let result = fixture.finish(FAULT_THEN_REFUSAL).await;

    assert_eq!(
        fixture.ran(),
        ["land-task-body", "freeze-acceptance", "land-task-body"],
        "the fault is asked again; the refusal replays"
    );
    assert_eq!(result["landed"], "landed 3", "{result}");
    assert_eq!(result["refused"], "refusal 2", "{result}");
}
