//! Issue 261: `w.pause(id, evidence)` pauses the run once per id, with an
//! evidence event, and a resumed run passes the pause it was resumed past.
//!
//! The last two tests drive the real fixed decomposition script through the
//! real script host: a body whose findings never improve pauses the run, and
//! a resume replays the recorded attempts, passes the pause and continues.

use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use super::*;

const PAUSING_SCRIPT: &str = r#"
async function workflow(w) {
  await w.checkpoint("before-pause", { note: "before" });
  const answer = await w.pause("pause-subject-1", {
    subject: "subject", reason: "no_progress", last_findings: ["defect alpha"],
  });
  await w.checkpoint("after-pause", { resumed: answer.resumed === true });
  return { resumed: answer.resumed === true };
}
"#;

/// Two branches stall at once: the first pause transitions the run, the
/// second finds it paused and joins that pause.
const TWO_PAUSES_SCRIPT: &str = r#"
async function workflow(w) {
  const settled = await Promise.allSettled([
    w.pause("pause-a-1", { subject: "a", reason: "no_progress" }),
    w.pause("pause-b-1", { subject: "b", reason: "no_progress" }),
  ]);
  const stopped = settled.find((result) => result.status === "rejected");
  if (stopped) throw stopped.reason;
  await w.checkpoint("after-pause", {});
  return {};
}
"#;

fn new_run() -> (tempfile::TempDir, WorkflowStore, String) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(test_spec()).unwrap();
    (temp, store, run.id)
}

fn set_status(store: &WorkflowStore, run_id: &str, status: archon_workflow::RunStatus) {
    let mut run = store.load_state(run_id).unwrap();
    run.status = status;
    store.save_state(&run).unwrap();
}

fn runner(
    store: &WorkflowStore,
    run_id: &str,
    llm: Arc<dyn WorkflowLlmClient>,
    host: Option<Arc<dyn crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor>>,
    args: Option<serde_json::Value>,
) -> (
    WorkflowV2ScriptRunner,
    // The UI channel's receiver, held so the sink stays open while the run does.
    Box<dyn std::any::Any>,
) {
    let spec = test_spec();
    let (ui_sink, rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(llm, ui_sink, Vec::new(), run_id.into(), None, Some(1_500))
        .with_fixed_raw_tool_policy(vec!["Read".into()]);
    let runner = WorkflowV2ScriptRunner::new(
        "pause".into(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2")),
        store.clone(),
        run_id.into(),
        true,
        None,
        args,
    )
    .with_raw_outcomes(true);
    let runner = match host {
        Some(host) => runner.with_host_command_executor(host),
        None => runner,
    };
    (runner, Box::new(rx))
}

/// Runs `script` to its end, keeping the UI channel open throughout.
async fn run_script(
    store: &WorkflowStore,
    run_id: &str,
    script: &str,
) -> Result<WorkflowV2ScriptSummary, WorkflowError> {
    let (runner, _rx) = runner(store, run_id, Arc::new(PanicLlm), None, None);
    runner.run(script).await
}

fn events(store: &WorkflowStore, run_id: &str) -> Vec<archon_workflow::WorkflowEvent> {
    std::fs::read_to_string(store.events_path(run_id))
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn pause_events(store: &WorkflowStore, run_id: &str) -> Vec<archon_workflow::WorkflowEvent> {
    events(store, run_id)
        .into_iter()
        .filter(|event| event.detail["event"] == "script_pause")
        .collect()
}

fn resume(store: &WorkflowStore, run_id: &str) {
    archon_workflow::LifecycleController::new(store.clone())
        .apply(run_id, archon_workflow::LifecycleAction::Resume)
        .unwrap();
}

fn record_exists(store: &WorkflowStore, run_id: &str, call_id: &str) -> bool {
    WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"))
        .load_call_record(call_id)
        .unwrap()
        .is_some()
}

#[tokio::test]
async fn a_pause_request_pauses_the_run_with_its_evidence() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    let before = store.load_state(&run_id).unwrap().generation;

    let error = run_script(&store, &run_id, PAUSING_SCRIPT)
        .await
        .expect_err("the run must stop on the pause");

    assert!(
        matches!(&error, WorkflowError::ControlPaused(message) if message.contains("pause-subject-1")),
        "{error:?}"
    );
    let run = store.load_state(&run_id).unwrap();
    assert_eq!(run.status, archon_workflow::RunStatus::Paused);
    assert_eq!(run.generation, before + 1, "the same transition as a pause");
    let paused = pause_events(&store, &run_id);
    assert_eq!(paused.len(), 1, "{paused:?}");
    assert_eq!(paused[0].kind, archon_workflow::WorkflowEventKind::Paused);
    let detail = &paused[0].detail;
    assert_eq!(detail["pause_id"], "pause-subject-1", "{detail}");
    assert_eq!(detail["evidence"]["subject"], "subject", "{detail}");
    assert_eq!(
        detail["evidence"]["last_findings"][0], "defect alpha",
        "{detail}"
    );
    assert!(
        detail["resume"].as_str().is_some_and(
            |text| text.contains(&format!("archon workflow resume --live --yes {run_id}"))
        ),
        "{detail}"
    );
    assert!(record_exists(&store, &run_id, "before-pause"));
    assert!(
        !record_exists(&store, &run_id, "after-pause"),
        "nothing runs past a pause"
    );
}

#[tokio::test]
async fn a_resumed_run_passes_the_pause_it_was_resumed_past_and_continues() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    run_script(&store, &run_id, PAUSING_SCRIPT)
        .await
        .expect_err("first run pauses");
    resume(&store, &run_id);

    let summary = run_script(&store, &run_id, PAUSING_SCRIPT)
        .await
        .expect("the resumed run passes the pause");

    assert_eq!(summary.status, WorkflowV2Status::Accepted, "{summary:?}");
    assert_eq!(
        summary.script_result.as_deref(),
        Some(r#"{"resumed":true}"#)
    );
    assert!(record_exists(&store, &run_id, "after-pause"));
    assert_eq!(
        pause_events(&store, &run_id).len(),
        1,
        "a pause is taken once per id"
    );
    assert_eq!(
        store.load_state(&run_id).unwrap().status,
        archon_workflow::RunStatus::Running
    );
}

#[tokio::test]
async fn a_pause_requested_while_a_sibling_already_paused_the_run_joins_that_pause() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    let before = store.load_state(&run_id).unwrap().generation;

    let error = run_script(&store, &run_id, TWO_PAUSES_SCRIPT)
        .await
        .expect_err("the run stops on the pause");

    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    assert_eq!(
        store.load_state(&run_id).unwrap().generation,
        before + 1,
        "one transition, however many branches asked"
    );
    let paused = pause_events(&store, &run_id);
    assert_eq!(paused.len(), 2, "{paused:?}");
    assert_eq!(paused[0].kind, archon_workflow::WorkflowEventKind::Paused);
    assert_eq!(paused[0].detail["joined"], false);
    assert_eq!(
        paused[1].kind,
        archon_workflow::WorkflowEventKind::StageStalled
    );
    assert_eq!(paused[1].detail["joined"], true);
    assert_eq!(paused[1].detail["evidence"]["subject"], "b");
    // Both were taken: the resume passes them instead of pausing again.
    resume(&store, &run_id);
    run_script(&store, &run_id, TWO_PAUSES_SCRIPT)
        .await
        .expect("both pauses were taken");
    assert!(record_exists(&store, &run_id, "after-pause"));
    assert_eq!(pause_events(&store, &run_id).len(), 2);
}

// --- the fixed decomposition script through the real host -------------------

/// Answers every author; content is unique per provider call, so each body
/// candidate is its own host-command identity.
struct CountingLlm {
    calls: AtomicUsize,
    ids: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for CountingLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        panic!("fixed authors must use raw run_agent")
    }

    async fn run_agent(
        &self,
        request: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let ordinal = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        self.ids.lock().unwrap().push(request.task.clone());
        let content = if request.task.contains("Author ONLY entry AC-X-001") {
            serde_json::json!({"id": "AC-X-001"}).to_string()
        } else {
            format!("# candidate {ordinal}")
        };
        Ok(WorkflowAgentOutcome {
            content,
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }
}

/// Every gate is clean except the body gate, which reports the same finding
/// until `fixed` is set: the operator's or a fix's change.
struct StallingHost {
    fixed: AtomicBool,
}

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor for StallingHost {
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!(
            "stall-{}-{}",
            request.command_id,
            archon_workflow::task_set_contract::content_digest(
                request.stdin.as_deref().unwrap_or_default().as_bytes()
            )
        ))
    }

    fn record_is_reusable(
        &self,
        _record: &archon_workflow::WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        Ok(true)
    }

    async fn execute(
        &self,
        request: archon_workflow::HostCommandRequest,
        _expected_generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        let findings =
            if request.command_id == "land-task-body" && !self.fixed.load(Ordering::SeqCst) {
                vec![archon_workflow::GatePolicyFinding {
                    text: "deliverable `src/a.rs` carries no observation".into(),
                    subject: "TASK-X-010".into(),
                    source_path: None,
                    remediation_scope: archon_workflow::RemediationScope::Body,
                }]
            } else {
                Vec::new()
            };
        let subjects = if request.command_id == "freeze-skeleton" {
            vec![archon_workflow::HostCommandSubject {
                task_id: "TASK-X-010".into(),
                file_name: "TASK-X-010.md".into(),
            }]
        } else {
            Vec::new()
        };
        let call_id = self.call_identity(&request)?;
        Ok(archon_workflow::HostCommandResult {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            stdout_bytes: 0,
            stderr_bytes: 0,
            timed_out: false,
            interrupted: false,
            stdout_truncated: false,
            stderr_truncated: false,
            gate_envelope: Some(archon_workflow::GateEnvelopeV1 {
                schema_version: archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION,
                report: serde_json::json!("judged"),
                policy_findings: findings,
                operational_error: None,
            }),
            publication_receipt: Some(archon_workflow::PublicationReceiptV1 {
                schema_version: archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION,
                call_id,
                command_id: request.command_id,
                entries: Vec::new(),
                committed_at: "2026-10-03T00:00:00Z".into(),
            }),
            subjects,
            postcondition: Some(archon_workflow::CommandPostconditionEvaluation {
                satisfied: true,
                summary: "fixture postcondition".into(),
            }),
        })
    }
}

fn fixed_args(root: &std::path::Path) -> serde_json::Value {
    serde_json::json!({
        "projectRoot": root,
        "repositoryRoot": root,
        "prdPath": root.join("PRD.md"),
        "prdDigest": "a".repeat(64),
        "acceptanceCriteria": {"AC-X-001": "example criterion"},
        "taskRoot": root.join("tasks"),
        "gateMode": "enforce"
    })
}

async fn run_fixed(
    temp: &tempfile::TempDir,
    store: &WorkflowStore,
    run_id: &str,
    llm: &Arc<CountingLlm>,
    host: &Arc<StallingHost>,
) -> Result<WorkflowV2ScriptSummary, WorkflowError> {
    let (runner, _rx) = runner(
        store,
        run_id,
        llm.clone(),
        Some(host.clone()),
        Some(fixed_args(temp.path())),
    );
    runner
        .run(crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE)
        .await
}

fn fixture() -> (
    tempfile::TempDir,
    WorkflowStore,
    String,
    Arc<CountingLlm>,
    Arc<StallingHost>,
) {
    let (temp, store, run_id) = new_run();
    let llm = Arc::new(CountingLlm {
        calls: AtomicUsize::new(0),
        ids: Mutex::new(Vec::new()),
    });
    let host = Arc::new(StallingHost {
        fixed: AtomicBool::new(false),
    });
    (temp, store, run_id, llm, host)
}

#[tokio::test]
async fn a_stalled_body_pauses_the_fixed_run_and_a_resume_after_a_fix_accepts_it() {
    let (temp, store, run_id, llm, host) = fixture();

    let error = run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect_err("a body without progress pauses the run");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    assert_eq!(
        store.load_state(&run_id).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
    // Acceptance, skeleton, then one baseline body attempt and three without
    // progress.
    assert_eq!(llm.calls.load(Ordering::SeqCst), 6);
    let paused = pause_events(&store, &run_id);
    assert_eq!(paused.len(), 1, "{paused:?}");
    let detail = &paused[0].detail;
    assert_eq!(detail["pause_id"], "pause-body-TASK-X-010-1", "{detail}");
    assert_eq!(detail["evidence"]["subject"], "body-TASK-X-010", "{detail}");
    assert_eq!(detail["evidence"]["reason"], "no_progress", "{detail}");
    assert_eq!(detail["evidence"]["author_calls"], 4, "{detail}");
    assert!(
        detail["evidence"]["last_findings"][0]
            .as_str()
            .is_some_and(|text| text.contains("carries no observation")),
        "{detail}"
    );

    host.fixed.store(true, Ordering::SeqCst);
    resume(&store, &run_id);
    let summary = run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect("the resumed run continues");

    assert_eq!(summary.status, WorkflowV2Status::Accepted, "{summary:?}");
    assert_eq!(
        llm.calls.load(Ordering::SeqCst),
        7,
        "every recorded author call is reused; only the attempt after the pause is new"
    );
    assert_eq!(pause_events(&store, &run_id).len(), 1);
}

#[tokio::test]
async fn a_resume_with_nothing_changed_makes_new_attempts_before_it_pauses_again() {
    let (temp, store, run_id, llm, host) = fixture();
    run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect_err("first stall pauses");
    resume(&store, &run_id);

    let error = run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect_err("still no progress");

    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    assert_eq!(
        llm.calls.load(Ordering::SeqCst),
        9,
        "the resume grants a fresh window: three new attempts, not an immediate re-pause"
    );
    let paused = pause_events(&store, &run_id);
    assert_eq!(paused.len(), 2, "{paused:?}");
    assert_eq!(paused[1].detail["pause_id"], "pause-body-TASK-X-010-2");
    assert_eq!(paused[1].detail["evidence"]["author_calls"], 7);
}
