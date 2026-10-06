//! Issue 335: a workflow.js runtime error after recorded work pauses the run
//! with the JavaScript error as evidence; a resume re-runs the script and
//! reuses the calls it recorded complete. A deterministic crash that recurs
//! at the same point pauses again, naming the recurrence. A source that
//! cannot be evaluated at all still fails the run.

use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use super::workflow_live_v2_script_control_tests::create_run;

/// Inspects area one, then area two, and throws on an unexpected verdict
/// from area two: a script defect a call's result triggers.
const CRASH_AFTER_WORK_SCRIPT: &str = r#"
async function workflow(w) {
  const one = await w.agent("inspect-one", { role: "analysis", task: "Inspect area one and report." });
  const two = await w.agent("inspect-two", { role: "analysis", task: "Inspect area two and report." });
  if (two.status !== "accepted") {
    throw new Error("unexpected verdict from inspect-two: " + two.status);
  }
  return { one: one.status, two: two.status };
}
"#;

/// Throws at the same point on every execution.
const DETERMINISTIC_CRASH_SCRIPT: &str = r#"
async function workflow(w) {
  const one = await w.agent("inspect-one", { role: "analysis", task: "Inspect area one and report." });
  throw new Error("deterministic defect after inspect-one: " + one.status);
}
"#;

/// Answers every call accepted, except a call about area two while
/// `fail_area_two` is set; counts the calls it answered.
struct AreaLlm {
    fail_area_two: bool,
    answered: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl archon_workflow::WorkflowLlmClient for AreaLlm {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.answered.fetch_add(1, Ordering::SeqCst);
        let about_two = serde_json::to_string(&messages)
            .unwrap_or_default()
            .contains("Inspect area two");
        let result = if self.fail_area_two && about_two {
            let mut result = WorkflowV2Result::accepted("area two could not be read");
            result.status = WorkflowV2Status::Failed;
            result
        } else {
            let mut result = WorkflowV2Result::accepted("inspected the area");
            result.evidence.push(WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Inspection,
                "read the area",
            ));
            result
        };
        Ok(archon_workflow::WorkflowAgentOutcome {
            content: serde_json::to_string(&result).expect("result json"),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: None,
        })
    }
}

fn runner(
    store: &WorkflowStore,
    run_id: &str,
    fail_area_two: bool,
    answered: Arc<AtomicUsize>,
) -> (WorkflowV2ScriptRunner, impl Sized) {
    let (ui_sink, ui) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let llm = Arc::new(AreaLlm {
        fail_area_two,
        answered,
    });
    let client = LiveV2AgentClient::new(llm, ui_sink, Vec::new(), run_id.to_string(), None, None);
    let runner = WorkflowV2ScriptRunner::new(
        "script error pause probe".to_string(),
        WorkflowV2ScriptRuntime {
            target_repository_root: None,
            generated_config: archon_core::config::GeneratedWorkflowConfig::default(),
        },
        WorkflowV2AgentAdapter::new(),
        client,
        WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2")),
        store.clone(),
        run_id.to_string(),
        true,
        None,
        None,
    );
    (runner, ui)
}

/// The last script-error pause event of `run_id`.
fn last_script_error_pause(store: &WorkflowStore, run_id: &str) -> serde_json::Value {
    let events = std::fs::read_to_string(store.events_path(run_id)).expect("events");
    // Every line must parse, so parse them all before the search from the end.
    let parsed: Vec<serde_json::Value> = events
        .lines()
        .map(|line| serde_json::from_str(line).expect("event json"))
        .collect();
    parsed
        .into_iter()
        .rfind(|event| event["detail"]["event"] == "script_error_pause")
        .unwrap_or_else(|| panic!("no script error pause evidence: {events}"))
}

fn resume(store: &WorkflowStore, run_id: &str) {
    archon_workflow::LifecycleController::new(store.clone())
        .apply(run_id, archon_workflow::LifecycleAction::Resume)
        .expect("resume");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_crash_after_completed_calls_pauses_and_the_resume_reuses_them() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = create_run(&store);

    // Session 1: inspect-one completes, inspect-two fails, the script throws.
    let answered = Arc::new(AtomicUsize::new(0));
    let (first, _ui) = runner(&store, &run.id, true, answered.clone());
    let error = first
        .run(CRASH_AFTER_WORK_SCRIPT)
        .await
        .expect_err("a script crash after recorded work pauses the run");
    let WorkflowError::ControlPaused(message) = &error else {
        panic!("a script crash pauses, never fails: {error:?}");
    };
    assert!(
        message.contains("unexpected verdict from inspect-two: failed"),
        "the pause carries the JavaScript error: {message}"
    );
    assert!(message.contains("paused, not failed"), "{message}");
    assert!(message.contains("workflow resume"), "{message}");
    let state = store.load_state(&run.id).expect("state");
    assert_eq!(state.status, archon_workflow::RunStatus::Paused);
    let pause = last_script_error_pause(&store, &run.id);
    assert_eq!(pause["kind"], "paused");
    assert_eq!(pause["detail"]["cause"], "script_error");
    assert_eq!(pause["detail"]["call_id"], "workflow.js");
    assert_eq!(pause["detail"]["calls_answered"], 2);
    assert_eq!(pause["detail"]["last_call"], "inspect-two");
    assert!(
        pause["detail"]["script_error"]
            .as_str()
            .is_some_and(|text| text.contains("unexpected verdict from inspect-two")),
        "{pause}"
    );
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let one = v2_store
        .load_call_record("inspect-one")
        .expect("lookup")
        .expect("inspect-one recorded");
    assert_eq!(one.status, WorkflowV2Status::Accepted);

    // Session 2: resumed. inspect-one answers from its record; only
    // inspect-two runs again, and the script completes.
    resume(&store, &run.id);
    let answered = Arc::new(AtomicUsize::new(0));
    let (second, _ui) = runner(&store, &run.id, false, answered.clone());
    let summary = second
        .run(CRASH_AFTER_WORK_SCRIPT)
        .await
        .expect("the resumed script completes");
    assert_eq!(summary.status, WorkflowV2Status::Accepted, "{summary:?}");
    assert_eq!(summary.reused, 1, "inspect-one is reused: {summary:?}");
    assert_eq!(summary.executed, 1, "only inspect-two runs: {summary:?}");
    assert_eq!(answered.load(Ordering::SeqCst), 1, "one dispatch on resume");
    assert!(summary.failed_call.is_none(), "{summary:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_same_crash_at_the_same_point_pauses_again_naming_the_recurrence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = create_run(&store);
    let answered = Arc::new(AtomicUsize::new(0));

    let (first, _ui) = runner(&store, &run.id, false, answered.clone());
    let error = first
        .run(DETERMINISTIC_CRASH_SCRIPT)
        .await
        .expect_err("the first crash pauses");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    assert_eq!(
        last_script_error_pause(&store, &run.id)["detail"]["occurrences"],
        1
    );

    resume(&store, &run.id);
    let (second, _ui) = runner(&store, &run.id, false, answered.clone());
    let error = second
        .run(DETERMINISTIC_CRASH_SCRIPT)
        .await
        .expect_err("the recurring crash pauses again, it never fails");
    let WorkflowError::ControlPaused(message) = &error else {
        panic!("a recurring crash pauses: {error:?}");
    };
    assert!(
        message.contains("a resume alone re-runs it unchanged"),
        "the operator is told a resume alone does not help: {message}"
    );
    assert!(message.contains("deterministic defect"), "{message}");
    assert_eq!(answered.load(Ordering::SeqCst), 1, "inspect-one reused");
    let pause = last_script_error_pause(&store, &run.id);
    assert_eq!(pause["detail"]["cause"], "recurring_script_error");
    assert_eq!(pause["detail"]["occurrences"], 2);
    assert_eq!(pause["detail"]["last_call"], "inspect-one");
    assert_eq!(
        store.load_state(&run.id).expect("state").status,
        archon_workflow::RunStatus::Paused
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_source_that_cannot_be_evaluated_still_fails_with_its_error() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = create_run(&store);
    let answered = Arc::new(AtomicUsize::new(0));
    let (runner, _ui) = runner(&store, &run.id, false, answered.clone());
    let summary = runner
        .run("async function workflow(w) { const broken = ; }")
        .await
        .expect("an unevaluable source is a failed outcome");
    assert_eq!(summary.status, WorkflowV2Status::Failed, "{summary:?}");
    assert_eq!(summary.failed_call.as_deref(), Some("workflow.js"));
    let script_error = summary.script_error.expect("the summary carries the error");
    assert!(script_error.contains("unexpected token"), "{script_error}");
    assert!(
        script_error.contains("a resume evaluates the same source"),
        "the failure says why a resume cannot help: {script_error}"
    );
    assert_eq!(answered.load(Ordering::SeqCst), 0);
    assert_ne!(
        store.load_state(&run.id).expect("state").status,
        archon_workflow::RunStatus::Paused
    );
}
