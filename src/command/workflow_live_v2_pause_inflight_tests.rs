//! Issue-213 C5: an operator pause with agent calls in flight leaves every
//! one of them recorded as paused. Live (2026-10-13), a fixed decomposition
//! paused with two author calls in flight ended with both records still
//! `running` and both in-flight markers on disk: the next start then recorded
//! them as `host_process_ended`, the wrong cause.
use super::NoHostCommands;
use super::*;

fn inflight_markers(store: &WorkflowStore, run_id: &str) -> usize {
    std::fs::read_dir(store.run_dir(run_id).join("v2/inflight"))
        .map(|entries| entries.count())
        .unwrap_or(0)
}

/// The session's own run-control watcher ends an agent session the moment
/// the run is paused, before the host's control race ticks, and the stop
/// reaches the host as a transport failure -- the shape a live run showed
/// (`agent transport failed: workflow stage failed: subagent cancelled`).
struct StoppedBySessionControl {
    store: WorkflowStore,
    run_id: String,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for StoppedBySessionControl {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        loop {
            if self
                .store
                .load_state(&self.run_id)
                .is_ok_and(|run| run.status == RunStatus::Paused)
            {
                return Err(WorkflowError::StageFailed(
                    "agent transport failed: workflow stage failed: subagent cancelled".into(),
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
}

/// A trusted host command that refuses after a while, so the agent call the
/// script started first is in flight when the script then throws.
struct RefusesAfterAWhile;
#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor
    for RefusesAfterAWhile
{
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!("host-command:{}", request.command_id))
    }
    fn record_is_reusable(
        &self,
        _: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        Ok(false)
    }
    async fn execute(
        &self,
        _: archon_workflow::HostCommandRequest,
        _: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        Err(WorkflowError::SpecInvalid("host command refused".into()))
    }
}

/// A trusted host command that is still running when the run stops.
struct HangingHostCommand;
#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor
    for HangingHostCommand
{
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!("host-command:{}", request.command_id))
    }
    fn record_is_reusable(
        &self,
        _: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        Ok(false)
    }
    async fn execute(
        &self,
        _: archon_workflow::HostCommandRequest,
        _: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        std::future::pending().await
    }
}

enum Llm {
    Pending,
    SessionStops,
}

/// Runs `script` as a fixed decomposition, pauses it as an operator does once
/// `in_flight` markers exist, and returns the store and run id after the
/// executor ended.
async fn pause_with_calls_in_flight(
    script: &str,
    in_flight: Option<usize>,
    llm: Llm,
    executor: Arc<dyn crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor>,
) -> (tempfile::TempDir, WorkflowStore, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::project(temp.path());
    let mut run = store
        .create_run(super::super::super::workflow_run_finalizer_tests::spec())
        .expect("run");
    run.status = RunStatus::Running;
    store.save_state(&run).expect("state");
    super::super::terminal_test_support::seed_fixed(&store, &run.id, temp.path());
    let plan = WorkflowScriptPlan::from_template(run.spec.clone(), script, Vec::new());
    super::super::terminal_test_support::save_fixed_metadata(&store, &run.id, &plan);

    let operator_store = store.clone();
    let operator_run = run.id.clone();
    let operator = tokio::spawn(async move {
        // The operator pauses once the calls are in flight; `None`: never.
        let Some(in_flight) = in_flight else {
            return;
        };
        while inflight_markers(&operator_store, &operator_run) < in_flight {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        LifecycleController::new(operator_store.clone())
            .apply(&operator_run, LifecycleAction::Pause)
            .expect("operator pause");
    });
    let llm: Arc<dyn WorkflowLlmClient> = match llm {
        Llm::Pending => Arc::new(super::super::terminal_test_support::PendingReply),
        Llm::SessionStops => Arc::new(StoppedBySessionControl {
            store: store.clone(),
            run_id: run.id.clone(),
        }),
    };
    let (ui, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        execute_fixed_decomposition_v2_run(
            &store,
            run.clone(),
            plan,
            llm,
            ui,
            Vec::new(),
            executor,
        ),
    )
    .await
    .expect("the paused run ends");
    tokio::time::timeout(std::time::Duration::from_secs(5), operator)
        .await
        .expect("operator finished")
        .expect("operator task");
    assert!(outcome.is_ok(), "a pause is not an error: {outcome:?}");
    let run_id = run.id.clone();
    (temp, store, run_id)
}

/// Every call names the pause as its cause, and no marker is left behind.
fn assert_recorded_as_paused(store: &WorkflowStore, run_id: &str, calls: &[&str]) {
    assert_eq!(store.load_state(run_id).unwrap().status, RunStatus::Paused);
    let v2 = WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"));
    for call in calls {
        let record = v2
            .load_call_record(call)
            .expect("lookup")
            .unwrap_or_else(|| panic!("{call}: an interrupted call keeps a record"));
        assert_eq!(
            (record.status, record.result.data["interrupted"].as_str()),
            (WorkflowV2Status::NeedsReview, Some("paused")),
            "{call}: the pause is the recorded cause, never `running`: {}",
            record.result.summary
        );
        assert!(
            !record.is_reusable_for(&record.input_hash),
            "{call}: an interrupted record is never replayed as an answer"
        );
    }
    assert_eq!(
        inflight_markers(store, run_id),
        0,
        "no in-flight marker outlives a recorded pause"
    );
}

const TWO_AGENTS: &str = r#"async function workflow(w) {
    await Promise.all([
        w.agent("slow-a", {role: "analysis", task: "Inspect the first area."}),
        w.agent("slow-b", {role: "analysis", task: "Inspect the second area."})
    ]);
}"#;

/// The host's control race stops both calls with a typed pause.
#[tokio::test]
async fn an_operator_pause_records_every_call_in_flight_as_paused() {
    let (_temp, store, run_id) =
        pause_with_calls_in_flight(TWO_AGENTS, Some(2), Llm::Pending, Arc::new(NoHostCommands))
            .await;
    assert_recorded_as_paused(&store, &run_id, &["slow-a", "slow-b"]);
}

/// The live shape: each session's own watcher ends it first, as a transport
/// failure; the generation fence then refuses to publish that result. The
/// pause, not a lost result, is what each call records.
#[tokio::test]
async fn a_session_its_own_watcher_stopped_at_the_pause_is_recorded_as_paused() {
    let (_temp, store, run_id) = pause_with_calls_in_flight(
        TWO_AGENTS,
        Some(2),
        Llm::SessionStops,
        Arc::new(NoHostCommands),
    )
    .await;
    assert_recorded_as_paused(&store, &run_id, &["slow-a", "slow-b"]);
}

/// A sibling the control stop never reaches (a host command lands whole and
/// is not raced) is still in flight when the script ends; it is recorded with
/// the pause as its cause, not left `running` for the next start.
#[tokio::test]
async fn a_call_still_in_flight_when_the_script_ends_is_recorded_as_paused() {
    let script = r#"async function workflow(w) {
        await Promise.all([
            w.agent("slow-a", {role: "analysis", task: "Inspect the first area."}),
            w.hostCommand("hang-cmd", {stdin: "x"})
        ]);
    }"#;
    let (_temp, store, run_id) =
        pause_with_calls_in_flight(script, Some(2), Llm::Pending, Arc::new(HangingHostCommand))
            .await;
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"));
    let host = v2
        .load_call_records()
        .unwrap()
        .into_iter()
        .find(|record| record.call.method == archon_workflow::v2::WorkflowV2HostMethod::HostCommand)
        .expect("the host command keeps a record");
    assert_recorded_as_paused(&store, &run_id, &["slow-a", host.call.id.as_str()]);
}

/// A script error with a call in flight: the run pauses for the error, and
/// the call in flight is recorded with that pause, not left `running`.
#[tokio::test]
async fn a_script_error_pause_records_the_call_in_flight() {
    let script = r#"async function workflow(w) {
        const slow = w.agent("slow-a", {role: "analysis", task: "Inspect the first area."});
        await w.hostCommand("then-refuse", {stdin: "x"});
        throw new Error("the script stops after its host command");
    }"#;
    let (_temp, store, run_id) =
        pause_with_calls_in_flight(script, None, Llm::Pending, Arc::new(RefusesAfterAWhile)).await;
    assert_recorded_as_paused(&store, &run_id, &["slow-a"]);
}
