//! Issue 337: a deliberate terminal stop stays terminal while a sibling host
//! command is still in flight. The sibling ends after the stop was recorded:
//! its operational pause and its publication are refused, so the run ends
//! with the deliberate verdict, not a resumable pause. An operator's pause
//! still outranks the stop.

use super::*;

use std::sync::atomic::{AtomicBool, Ordering};

use crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor;
use crate::command::workflow_host_command_operational::{
    OperationalAttempt, OperationalReport, pause_run, require_run_owned_locked,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sibling {
    /// Reaches an operational no-progress ending, as a gate whose every
    /// attempt timed out does.
    OperationalPause,
    /// Completes and publishes, as a gate that accepted its candidate does.
    Publication,
    /// An operator pauses the run while it is in flight.
    OperatorPause,
}

/// A host command still running when its sibling body stops the run.
struct SiblingAfterStop {
    store: WorkflowStore,
    run_id: String,
    mode: Sibling,
    published: Arc<AtomicBool>,
    /// The event that says the host recorded the stop.
    stop_event: &'static str,
}

impl SiblingAfterStop {
    /// Waits until the host recorded the deliberate stop.
    async fn stop_recorded(&self) -> archon_workflow::WorkflowResult<u64> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let events =
                std::fs::read_to_string(self.store.events_path(&self.run_id)).unwrap_or_default();
            if events.contains(self.stop_event) {
                return Ok(self.store.load_state(&self.run_id)?.generation);
            }
            if std::time::Instant::now() > deadline {
                return Err(WorkflowError::SpecInvalid(
                    "the deliberate stop was never recorded".into(),
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
}

#[async_trait::async_trait]
impl WorkflowHostCommandExecutor for SiblingAfterStop {
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
        Ok(false)
    }

    async fn execute(
        &self,
        request: archon_workflow::HostCommandRequest,
        _expected_generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        let generation = self.stop_recorded().await?;
        let call_id = format!("host-command:{}:fixed", request.command_id);
        match self.mode {
            Sibling::OperationalPause => {
                let attempts = [OperationalAttempt {
                    attempt: 1,
                    reason: "timeout",
                    elapsed_secs: 1,
                    progress: None,
                }];
                let report = OperationalReport {
                    run_id: &self.run_id,
                    call_id: &call_id,
                    command_id: &request.command_id,
                    limit_secs: 1,
                    attempts: &attempts,
                };
                Err(pause_run(
                    &self.store,
                    &self.store.run_dir(&self.run_id),
                    generation,
                    &report,
                    "no_progress",
                ))
            }
            Sibling::Publication => {
                // The parent publication's own check, under the run lock.
                self.store.with_run_lock(&self.run_id, |locked| {
                    require_run_owned_locked(locked, &self.run_id, generation)
                })?;
                self.published.store(true, Ordering::SeqCst);
                Err(WorkflowError::StageFailed(
                    "published after the deliberate stop".into(),
                ))
            }
            Sibling::OperatorPause => {
                archon_workflow::LifecycleController::new(self.store.clone())
                    .apply(&self.run_id, archon_workflow::LifecycleAction::Pause)?;
                Err(WorkflowError::ControlPaused(format!(
                    "host command '{}' paused while in flight",
                    request.command_id
                )))
            }
        }
    }
}

const STOP_WITH_SIBLING_IN_FLIGHT: &str = r#"
async function workflow(w) {
  const sibling = w.hostCommand("task-set-lint", { stdin: null });
  const stop = __archonHost("terminalStop", JSON.stringify({ schemaVersion: 1, reason: "deliberate gate refusal" }));
  const settled = await Promise.allSettled([sibling, stop]);
  throw settled[1].reason;
}
"#;

async fn race(
    mode: Sibling,
) -> (
    archon_workflow::WorkflowResult<WorkflowV2ScriptSummary>,
    archon_workflow::RunStatus,
    bool,
) {
    race_with(mode, STOP_WITH_SIBLING_IN_FLIGHT, "script_terminal_stop").await
}

async fn race_with(
    mode: Sibling,
    script: &str,
    stop_event: &'static str,
) -> (
    archon_workflow::WorkflowResult<WorkflowV2ScriptSummary>,
    archon_workflow::RunStatus,
    bool,
) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = test_spec();
    let run_id = store.create_run(spec.clone()).unwrap().id;
    let published = Arc::new(AtomicBool::new(false));
    let (ui_sink, _ui) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        ui_sink,
        Vec::new(),
        run_id.clone(),
        None,
        None,
    );
    let outcome = WorkflowV2ScriptRunner::new(
        "terminal race".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2")),
        store.clone(),
        run_id.clone(),
        true,
        None,
        None,
    )
    .with_host_command_executor(Arc::new(SiblingAfterStop {
        store: store.clone(),
        run_id: run_id.clone(),
        mode,
        published: published.clone(),
        stop_event,
    }))
    .with_raw_outcomes(true)
    .run(script)
    .await;
    let status = store.load_state(&run_id).unwrap().status;
    (outcome, status, published.load(Ordering::SeqCst))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sibling_in_flight_never_turns_a_deliberate_stop_into_a_pause() {
    for mode in [Sibling::OperationalPause, Sibling::Publication] {
        let (outcome, status, published) = race(mode).await;
        let summary = outcome.unwrap_or_else(|error| {
            panic!("{mode:?}: the deliberate stop is the outcome: {error:?}")
        });
        assert_eq!(summary.status, WorkflowV2Status::Failed, "{mode:?}");
        assert_eq!(
            summary.script_error.as_deref(),
            Some("deliberate gate refusal"),
            "{mode:?}"
        );
        assert_ne!(status, archon_workflow::RunStatus::Paused, "{mode:?}");
        assert!(!published, "{mode:?}: nothing publishes after the stop");
    }
    // An operator's pause still outranks the stop.
    let (outcome, status, _) = race(Sibling::OperatorPause).await;
    assert!(
        matches!(outcome, Err(WorkflowError::ControlPaused(_))),
        "{outcome:?}"
    );
    assert_eq!(status, archon_workflow::RunStatus::Paused);
}

const FINAL_REPORT_WITH_SIBLING_IN_FLIGHT: &str = r#"
async function workflow(w) {
  const sibling = w.hostCommand("task-set-lint", { stdin: null });
  const report = w.finalReport("stopped", { status: "needs_review", inputs: {}, task: "Stop for review" });
  await Promise.allSettled([sibling, report]);
}
"#;

/// Round 3 (review finding 3): a host terminal stop by a call (an unsatisfied
/// final report or human gate) is persisted through the same record, so a
/// sibling in flight cannot turn it into a resumable pause either.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sibling_in_flight_never_pauses_a_run_its_final_report_stopped() {
    for mode in [Sibling::OperationalPause, Sibling::Publication] {
        let (outcome, status, published) =
            race_with(mode, FINAL_REPORT_WITH_SIBLING_IN_FLIGHT, "script_stopped").await;
        let summary = outcome.unwrap_or_else(|error| {
            panic!("{mode:?}: the final report's stop is the outcome: {error:?}")
        });
        assert_eq!(summary.status, WorkflowV2Status::NeedsReview, "{mode:?}");
        assert_eq!(summary.failed_call.as_deref(), Some("stopped"), "{mode:?}");
        assert_ne!(status, archon_workflow::RunStatus::Paused, "{mode:?}");
        assert!(!published, "{mode:?}: nothing publishes after the stop");
    }
}

/// Round 4 (review finding 5): a call's terminal stop that cannot be
/// persisted fails safe, as `terminalStop` does: the run pauses with the
/// refusal as evidence instead of ending on an in-memory stop alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unpersisted_final_report_stop_pauses_with_its_refusal() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = test_spec();
    let run_id = store.create_run(spec.clone()).unwrap().id;
    // The stop record's path is taken by a directory: the write fails.
    std::fs::create_dir_all(store.run_dir(&run_id).join("v2/terminal-stop.json/held")).unwrap();
    let (ui_sink, _ui) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        ui_sink,
        Vec::new(),
        run_id.clone(),
        None,
        None,
    );
    let outcome = WorkflowV2ScriptRunner::new(
        "unpersisted stop".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2")),
        store.clone(),
        run_id.clone(),
        true,
        None,
        None,
    )
    .with_raw_outcomes(true)
    .run(r#"async function workflow(w) {
      await w.finalReport("stopped", { status: "needs_review", inputs: {}, task: "Stop for review" });
    }"#)
    .await;
    assert!(
        matches!(outcome, Err(WorkflowError::ControlPaused(_))),
        "{outcome:?}"
    );
    assert_eq!(
        store.load_state(&run_id).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
    let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap();
    assert!(events.contains("terminal_stop_unpersisted"), "{events}");
}
