//! Issue 304: real replay and an operator fence delivered before JS settles.
use super::*;

async fn replay(script: &str) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store
        .create_run(super::super::super::workflow_run_finalizer_tests::spec())
        .unwrap();
    seed_fixed(&store, &run.id, temp.path());
    let plan = WorkflowScriptPlan::from_template(run.spec.clone(), script, Vec::new());
    save_fixed_metadata(&store, &run.id, &plan);
    let (ui, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    execute_fixed_decomposition_v2_run(
        &store,
        run.clone(),
        plan.clone(),
        Arc::new(ProviderOutage),
        ui.clone(),
        Vec::new(),
        Arc::new(NoHostCommands),
    )
    .await
    .unwrap();
    // Issue 337: an unplanned workflow.js throw pauses the run resumably and
    // commits no terminal finalization.
    let path = store.run_dir(&run.id).join("v2/finalization.json");
    assert!(
        !path.exists(),
        "an unplanned throw must not finalize the run"
    );
    assert_eq!(store.load_state(&run.id).unwrap().status, RunStatus::Paused);
    LifecycleController::new(store.clone())
        .apply(&run.id, LifecycleAction::Resume)
        .unwrap();
    let resumed = store.load_state(&run.id).unwrap();
    assert!(resumed.generation > run.generation);
    assert_eq!(resumed.status, RunStatus::Running);
    let report = execute_fixed_decomposition_v2_run(
        &store,
        resumed,
        plan,
        Arc::new(ProviderOutage),
        ui,
        Vec::new(),
        Arc::new(NoHostCommands),
    )
    .await
    .unwrap();
    let calls = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"))
        .load_call_records()
        .unwrap();
    assert!(
        calls
            .iter()
            .any(|call| call.call.method == WorkflowV2HostMethod::Agent
                && call.status == WorkflowV2Status::Failed
                && call.attempt >= 2),
        "provider failure must be dispatched again after resume"
    );
    assert!(report.contains("paused"), "{report}");
    assert!(
        !path.exists(),
        "the replayed throw must not finalize the run"
    );
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        RunStatus::Paused,
        "replay must synchronize durable state: {report}"
    );
}

#[tokio::test]
async fn issue304_replay_failed_after_provider_failure() {
    replay(r#"async function workflow(w) { await w.agent("outage", {role:"analysis", task:"Inspect"}); throw new Error("exhausted recovery"); }"#).await;
}
#[tokio::test]
async fn issue304_replay_failed_after_caught_provider_failure() {
    replay(r#"async function workflow(w) { try { await w.agent("outage", {role:"analysis", task:"Inspect"}); } catch (_) {} throw new Error("exhausted recovery"); }"#).await;
}
#[tokio::test]
async fn issue304_replay_failed_after_multiple_provider_failures() {
    replay(r#"async function workflow(w) { await Promise.allSettled([w.agent("outage-a", {role:"analysis", task:"Inspect"}), w.agent("outage-b", {role:"analysis", task:"Inspect"})]); throw new Error("exhausted recovery"); }"#).await;
}

struct InFlightFence {
    store: WorkflowStore,
    run: String,
    action: LifecycleAction,
    entered: Arc<tokio::sync::Notify>,
    stopped: Arc<tokio::sync::Notify>,
}
#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor for InFlightFence {
    fn call_identity(
        &self,
        _: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok("inflight-fence".into())
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
        generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        self.entered.notify_one();
        self.stopped.notified().await;
        let before = self.store.load_state(&self.run).unwrap();
        assert_eq!(generation, Some(before.generation));
        LifecycleController::new(self.store.clone())
            .apply(&self.run, self.action.clone())
            .unwrap();
        let current = self.store.load_state(&self.run).unwrap();
        assert!(current.generation > before.generation);
        // The supervisor's generation fence: delivered while the host call
        // remains in flight and while the script awaits both siblings.
        Err(WorkflowError::ControlCancelled(format!(
            "host command generation {} no longer matches {}",
            before.generation, current.generation
        )))
    }
}

async fn inflight(action: LifecycleAction, expected: RunStatus) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store
        .create_run(super::super::super::workflow_run_finalizer_tests::spec())
        .unwrap();
    seed_fixed(&store, &run.id, temp.path());
    let entered = Arc::new(tokio::sync::Notify::new());
    let stopped = Arc::new(tokio::sync::Notify::new());
    let signal = stopped.clone();
    super::super::terminal_test_support::on_stop(
        store.run_dir(&run.id),
        Box::new(move || signal.notify_one()),
    );
    struct WaitUntilEntered(Arc<tokio::sync::Notify>);
    #[async_trait::async_trait]
    impl WorkflowLlmClient for WaitUntilEntered {
        async fn send_message(
            &self,
            _: Vec<serde_json::Value>,
            _: Vec<serde_json::Value>,
            _: Vec<serde_json::Value>,
            _: &str,
        ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
            self.0.notified().await;
            let mut result = WorkflowV2Result::accepted("ready");
            result
                .evidence
                .push(archon_workflow::WorkflowV2Evidence::new(
                    archon_workflow::WorkflowV2EvidenceKind::Inspection,
                    "command entry observed",
                ));
            Ok(archon_workflow::WorkflowAgentOutcome {
                content: serde_json::to_string(&result).unwrap(),
                tool_uses: Vec::new(),
                tokens_in: 1,
                tokens_out: 1,
                stop_reason: None,
            })
        }
    }
    let script = r#"async function workflow(w) {
        const inflight = w.hostCommand("task-set-lint", {});
        await w.agent("barrier", {role:"analysis", task:"Wait for command entry"});
        await Promise.allSettled([inflight, w.humanGate("gate", {task:"Require approval"})]);
    }"#;
    let plan = WorkflowScriptPlan::from_template(run.spec.clone(), script, Vec::new());
    save_fixed_metadata(&store, &run.id, &plan);
    let executor = Arc::new(InFlightFence {
        store: store.clone(),
        run: run.id.clone(),
        action,
        entered: entered.clone(),
        stopped,
    });
    let (ui, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let report = execute_fixed_decomposition_v2_run(
        &store,
        run.clone(),
        plan,
        Arc::new(WaitUntilEntered(entered)),
        ui,
        Vec::new(),
        executor,
    )
    .await
    .unwrap();
    assert!(!report.contains("cancelled"), "{report}");
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        expected,
        "{report}"
    );
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let call = v2
        .load_call_records()
        .unwrap()
        .into_iter()
        .find(|record| record.call.method == WorkflowV2HostMethod::HostCommand)
        .expect("in-flight call closed");
    assert_eq!(call.result.data["interrupted"], "terminal_host_stop");
}

#[tokio::test]
async fn issue304_restart_while_terminal_sibling_inflight() {
    inflight(
        LifecycleAction::RestartStage("call-1".into()),
        RunStatus::NeedsReview,
    )
    .await;
}
#[tokio::test]
async fn issue304_force_accept_while_terminal_sibling_inflight() {
    inflight(
        LifecycleAction::ForceAcceptStage {
            stage_id: "call-1".into(),
            forced_by: "operator".into(),
            rationale: "reviewed".into(),
            source: "test".into(),
        },
        RunStatus::NeedsReview,
    )
    .await;
}
#[tokio::test]
async fn issue304_pause_while_terminal_sibling_inflight() {
    inflight(LifecycleAction::Pause, RunStatus::Paused).await;
    // Also prove an edit that keeps the run running cannot forge cancellation.
    inflight(
        LifecycleAction::RestartStage("call-1".into()),
        RunStatus::NeedsReview,
    )
    .await;
}
