//! Issue 261: `w.pause(id, evidence)` pauses the run once per id, with an
//! evidence event, and a resumed run passes the pause it was resumed past.
//!
//! The last two tests drive the real fixed decomposition script through the
//! real script host: a body whose findings never improve pauses the run, and
//! a resume replays the recorded attempts, passes the pause and continues.

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

pub(super) fn new_run() -> (tempfile::TempDir, WorkflowStore, String) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(test_spec()).unwrap();
    (temp, store, run.id)
}

pub(super) fn set_status(store: &WorkflowStore, run_id: &str, status: archon_workflow::RunStatus) {
    let mut run = store.load_state(run_id).unwrap();
    run.status = status;
    store.save_state(&run).unwrap();
}

pub(super) fn runner(
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
pub(super) async fn run_script(
    store: &WorkflowStore,
    run_id: &str,
    script: &str,
) -> Result<WorkflowV2ScriptSummary, WorkflowError> {
    let (runner, _rx) = runner(store, run_id, Arc::new(PanicLlm), None, None);
    runner.run(script).await
}

pub(super) fn events(store: &WorkflowStore, run_id: &str) -> Vec<archon_workflow::WorkflowEvent> {
    std::fs::read_to_string(store.events_path(run_id))
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

pub(super) fn pause_events(
    store: &WorkflowStore,
    run_id: &str,
) -> Vec<archon_workflow::WorkflowEvent> {
    events(store, run_id)
        .into_iter()
        .filter(|event| event.detail["event"] == "script_pause")
        .collect()
}

pub(super) fn resume(store: &WorkflowStore, run_id: &str) {
    archon_workflow::LifecycleController::new(store.clone())
        .apply(run_id, archon_workflow::LifecycleAction::Resume)
        .unwrap();
}

pub(super) fn record_exists(store: &WorkflowStore, run_id: &str, call_id: &str) -> bool {
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

/// A pause the run already took passes only a run that executes: when a
/// sibling has just paused the run, the credited pause stops too.
#[tokio::test]
async fn a_taken_pause_does_not_pass_a_run_that_is_paused_again() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    run_script(
        &store,
        &run_id,
        r#"async function workflow(w) { await w.pause("pause-a-1", { subject: "a" }); return {}; }"#,
    )
    .await
    .expect_err("first run pauses");
    resume(&store, &run_id);

    let outcome = run_script(
        &store,
        &run_id,
        r#"
async function workflow(w) {
  const settled = await Promise.allSettled([
    w.pause("pause-b-1", { subject: "b" }),
    w.pause("pause-a-1", { subject: "a" }),
  ]);
  if (settled[1].status === "fulfilled") throw new Error("credited pause passed a paused run");
  throw settled[0].reason;
}
"#,
    )
    .await;

    assert!(
        matches!(&outcome, Err(WorkflowError::ControlPaused(_))),
        "{outcome:?}"
    );
}

/// A restart that invalidates work a pause covered voids that pause: the
/// redone work reaching the same point pauses again, never passes on a
/// credit earned by work that no longer stands.
#[tokio::test]
async fn a_restart_of_covered_work_voids_the_pause_taken_after_it() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    run_script(&store, &run_id, PAUSING_SCRIPT)
        .await
        .expect_err("first run pauses");
    WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"))
        .invalidate_call_and_dependents(&[], "before-pause")
        .unwrap();
    resume(&store, &run_id);

    let outcome = run_script(&store, &run_id, PAUSING_SCRIPT).await;

    assert!(
        matches!(&outcome, Err(WorkflowError::ControlPaused(_))),
        "the redone work pauses again: {outcome:?}"
    );
    assert_eq!(pause_events(&store, &run_id).len(), 2);
    assert!(!record_exists(&store, &run_id, "after-pause"));
}

#[path = "workflow_live_v2_script_pause_fixed_tests.rs"]
mod fixed;

#[path = "workflow_live_v2_script_pause_lock_tests.rs"]
mod lock_credit;
