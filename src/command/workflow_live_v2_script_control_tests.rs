//! Issue-253: run control (pause, cancel) reaches a workflow script as a
//! typed outcome, and the run's outcome retains a host-observed stop: a pause
//! or cancel the operator applied outranks every error the script raised
//! while it unwound -- an exhausted sibling at a lower index, or a sibling's
//! untyped "cancelled" error induced by the pause itself.

use super::*;

use std::sync::atomic::{AtomicBool, Ordering};

/// A model request that does not come back until the test is long over:
/// only run control ends the call.
pub(super) struct StuckLlm {
    pub(super) entered: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl archon_workflow::WorkflowLlmClient for StuckLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.entered.store(true, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(90)).await;
        Err(WorkflowError::StageFailed(
            "the stuck request returned".into(),
        ))
    }
}

/// The script catches what `w.agent` threw and re-throws a description of
/// it, so the test can read what the script itself was able to test.
const TYPED_PROBE_SCRIPT: &str = r#"
async function workflow(w) {
  try {
    await w.agent("inspect-one", { role: "analysis", task: "Inspect the area and report." });
  } catch (error) {
    throw new Error("observed " + JSON.stringify({
      name: error && error.name,
      code: error && error.code,
      kind: error && error.kind,
      typed: typeof WorkflowControlError === "function" && error instanceof WorkflowControlError
    }));
  }
  return {};
}
"#;

/// The body pool's reduction (`authorBodies`): every started call settles,
/// then the lowest-index rejection is raised. Index 0 rejects with `{FIRST}`;
/// index 2 is a real host call that run control stops.
const POOL_PROBE_SCRIPT: &str = r#"
async function workflow(w) {
  const settle = (promise) => promise.then(
    (value) => ({ status: "fulfilled", value }),
    (reason) => ({ status: "rejected", reason }));
  const settled = await Promise.all([
    settle(Promise.resolve().then(() => { throw new Error({FIRST}); })),
    settle(Promise.resolve({ status: "accepted" })),
    settle(w.agent("body-two", { role: "analysis", task: "Inspect the area and report." })),
  ]);
  const failure = settled.find((result) => result.status === "rejected");
  if (failure) throw failure.reason;
  return {};
}
"#;

pub(super) fn create_run(store: &WorkflowStore) -> archon_workflow::WorkflowRun {
    store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
            name: "run-control-test".to_string(),
            task: "test".to_string(),
            target_repository_root: None,
            max_parallelism: 4,
            max_agents: 4,
            stages: Vec::new(),
            permissions: std::collections::BTreeMap::new(),
            learning_hooks: Vec::new(),
        })
        .expect("run")
}

/// Runs `script` against a model that never answers, applies `action` once
/// the call is in flight, and returns what the runner reported.
async fn run_with_control(
    script: &str,
    action: archon_workflow::LifecycleAction,
) -> (
    tempfile::TempDir,
    WorkflowStore,
    String,
    archon_workflow::WorkflowResult<WorkflowV2ScriptSummary>,
) {
    run_control_probe(script, action, false).await
}

async fn run_control_probe(
    script: &str,
    action: archon_workflow::LifecycleAction,
    resume_during_unwind: bool,
) -> (
    tempfile::TempDir,
    WorkflowStore,
    String,
    archon_workflow::WorkflowResult<WorkflowV2ScriptSummary>,
) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = create_run(&store);
    let entered = Arc::new(AtomicBool::new(false));
    let mut controller = {
        let (store, run_id, entered) = (store.clone(), run.id.clone(), entered.clone());
        tokio::spawn(async move {
            while !entered.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            archon_workflow::LifecycleController::new(store)
                .apply(&run_id, action)
                .expect("run control while the call is in flight");
        })
    };
    let (ui_sink, _ui) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(StuckLlm { entered }),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "run control probe".to_string(),
        WorkflowV2ScriptRuntime {
            target_repository_root: None,
            generated_config: archon_core::config::GeneratedWorkflowConfig::default(),
        },
        WorkflowV2AgentAdapter::new(),
        client,
        WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2")),
        store.clone(),
        run.id.clone(),
        true,
        None,
        None,
    );
    let runner = if resume_during_unwind {
        runner.with_host_command_executor(Arc::new(ResumeOnUnwind {
            store: store.clone(),
            run_id: run.id.clone(),
        }))
    } else {
        runner
    };
    let outcome = tokio::time::timeout(Duration::from_secs(300), runner.run(script))
        .await
        .expect("run control ends the in-flight call");
    match tokio::time::timeout(Duration::from_secs(5), &mut controller).await {
        Ok(result) => result.expect("controller"),
        Err(_) => {
            controller.abort();
            panic!(
                "controller never observed an in-flight model call; runner outcome: {outcome:?}"
            );
        }
    }
    (temp, store, run.id, outcome)
}

fn stored_status(store: &WorkflowStore, run_id: &str) -> archon_workflow::RunStatus {
    store.load_state(run_id).expect("state").status
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pause_reaches_the_script_as_a_typed_control_error() {
    let (_temp, store, run_id, outcome) =
        run_with_control(TYPED_PROBE_SCRIPT, archon_workflow::LifecycleAction::Pause).await;
    let error = outcome.expect_err("a paused run reports the pause");
    let WorkflowError::ControlPaused(message) = &error else {
        panic!("a paused run reports the pause: {error:?}");
    };
    assert!(
        message.contains(
            r#""name":"WorkflowControlError","code":"workflow_control","kind":"pause","typed":true"#
        ),
        "the script saw a typed pause: {message}"
    );
    assert_eq!(
        stored_status(&store, &run_id),
        archon_workflow::RunStatus::Paused
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancel_reaches_the_script_as_a_typed_control_error() {
    let (_temp, store, run_id, outcome) =
        run_with_control(TYPED_PROBE_SCRIPT, archon_workflow::LifecycleAction::Cancel).await;
    let error = outcome.expect_err("a cancelled run reports the cancel");
    let WorkflowError::ControlCancelled(message) = &error else {
        panic!("a cancelled run reports the cancel: {error:?}");
    };
    assert!(
        message.contains(r#""code":"workflow_control","kind":"cancel","typed":true"#),
        "the script saw a typed cancel: {message}"
    );
    assert_eq!(
        stored_status(&store, &run_id),
        archon_workflow::RunStatus::Cancelled
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pause_outranks_an_exhausted_body_at_a_lower_index() {
    let script =
        POOL_PROBE_SCRIPT.replace("{FIRST}", r#""body-zero exhausted 10 candidate attempts""#);
    let (_temp, store, run_id, outcome) =
        run_with_control(&script, archon_workflow::LifecycleAction::Pause).await;
    let error = outcome.expect_err("the pause outranks the exhausted body");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "the pause outranks the exhausted body: {error:?}"
    );
    assert_eq!(
        stored_status(&store, &run_id),
        archon_workflow::RunStatus::Paused
    );
    let record = WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"))
        .load_call_record("body-two")
        .expect("record lookup")
        .expect("the stopped call leaves a record");
    assert_eq!(record.result.data["interrupted"], "paused");
    assert!(!is_reusable_status(record.status));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pause_outranks_a_sibling_cancel_error_it_induced() {
    // What a sibling whose persistence lost generation ownership to the pause
    // raised into the script before the typed outcome existed.
    let script = POOL_PROBE_SCRIPT.replace(
        "{FIRST}",
        r#""workflow cancelled by run control: fixed executor generation 1 cannot persist call body-zero; current generation is 2""#,
    );
    let (_temp, store, run_id, outcome) =
        run_with_control(&script, archon_workflow::LifecycleAction::Pause).await;
    let error = outcome.expect_err("the run reports the pause");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "the stored state is Paused, so the run reports paused: {error:?}"
    );
    assert_eq!(
        stored_status(&store, &run_id),
        archon_workflow::RunStatus::Paused
    );
}

/// Invoked by JS only after it has caught the host's typed control error.
/// Identity resolution precedes the host's control poll, allowing the test
/// to resume at precisely that boundary without sleeps or timing guesses.
struct ResumeOnUnwind {
    store: WorkflowStore,
    run_id: String,
}

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor for ResumeOnUnwind {
    fn call_identity(
        &self,
        _: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        archon_workflow::LifecycleController::new(self.store.clone())
            .apply(&self.run_id, archon_workflow::LifecycleAction::Resume)?;
        Err(WorkflowError::PolicyDenied("unwind barrier reached".into()))
    }

    fn record_is_reusable(
        &self,
        _: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        unreachable!("identity barrier rejects before reuse")
    }

    async fn execute(
        &self,
        _: archon_workflow::HostCommandRequest,
        _: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        unreachable!("identity barrier rejects before dispatch")
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round3_host_observed_stop_survives_resume_during_unwind() {
    for action in [
        archon_workflow::LifecycleAction::Pause,
        archon_workflow::LifecycleAction::Cancel,
    ] {
        for finish in [
            "throw stopped;",
            "throw new Error('lower-index body exhausted');",
            "return {};",
        ] {
            let script = format!(
                r#"
async function workflow(w) {{
  try {{ await w.agent("inspect-one", {{role: "analysis", task: "Inspect the area and report."}}); }}
  catch (stopped) {{
    if (!(stopped instanceof WorkflowControlError)) throw new Error('missing typed control');
    try {{ await w.hostCommand("task-set-lint", {{stdin: null}}); }} catch (_) {{}}
    {finish}
  }}
}}
"#
            );
            let (_temp, store, run_id, outcome) =
                run_control_probe(&script, action.clone(), true).await;
            assert_eq!(
                stored_status(&store, &run_id),
                archon_workflow::RunStatus::Running
            );
            let expected_pause = matches!(action, archon_workflow::LifecycleAction::Pause);
            assert!(
                matches!(&outcome, Err(WorkflowError::ControlPaused(_))) && expected_pause
                    || matches!(&outcome, Err(WorkflowError::ControlCancelled(_)))
                        && !expected_pause,
                "{action:?}, {finish}: {outcome:?}"
            );
            let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap();
            assert!(!events.contains("script_stopped"), "{events}");
        }
    }
}
