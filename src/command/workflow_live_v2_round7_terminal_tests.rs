//! Round 7 (#285): an operator edit while a terminal stop unwinds never turns
//! the stop into a control outcome and never loses a pending call's record.
use super::*;

struct EditingSlowReply {
    store: WorkflowStore,
    run_id: String,
    edit: std::sync::Mutex<Option<LifecycleAction>>,
}
#[async_trait::async_trait]
impl WorkflowLlmClient for EditingSlowReply {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        // The sibling gate's terminal stop lands first; the edit follows
        // while this call is still pending.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let edit = self.edit.lock().ok().and_then(|mut slot| slot.take());
        if let Some(edit) = edit {
            LifecycleController::new(self.store.clone()).apply(&self.run_id, edit)?;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let mut result = WorkflowV2Result::accepted("slow sibling finished");
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "read the requested area",
        ));
        Ok(archon_workflow::WorkflowAgentOutcome {
            content: serde_json::to_string(&result)?,
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: None,
        })
    }
}

struct NoHostCommands;
#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor for NoHostCommands {
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
        Err(WorkflowError::SpecInvalid(
            "no host commands in this test".into(),
        ))
    }
}

async fn edit_during_unwind(edit: fn() -> LifecycleAction, expected: RunStatus) {
    edit_during_unwind_in(edit(), expected.clone(), false).await;
    edit_during_unwind_in(edit(), expected, true).await;
}

async fn edit_during_unwind_in(edit: LifecycleAction, expected: RunStatus, fixed: bool) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::project(temp.path());
    let mut run = store
        .create_run(super::super::workflow_run_finalizer_tests::spec())
        .expect("run");
    run.status = RunStatus::Running;
    store.save_state(&run).expect("state");
    let script = r#"async function workflow(w) {
        await Promise.allSettled([
            w.agent("slow", {role: "analysis", task: "Inspect the area."}),
            w.humanGate("gate", {task: "Require approval"})
        ]);
        await new Promise(() => {});
    }"#;
    let plan = WorkflowScriptPlan::from_template(run.spec.clone(), script, Vec::new());
    save_generated_v2_metadata(&store, &run.id, &plan, false).expect("metadata");
    let llm = Arc::new(EditingSlowReply {
        store: store.clone(),
        run_id: run.id.clone(),
        edit: std::sync::Mutex::new(Some(edit)),
    });
    let (ui, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let ten = std::time::Duration::from_secs(10);
    let report = if fixed {
        let executor = Arc::new(NoHostCommands);
        let run = execute_fixed_decomposition_v2_run(
            &store,
            run.clone(),
            plan,
            llm,
            ui,
            Vec::new(),
            executor,
        );
        let result = tokio::time::timeout(ten, run)
            .await
            .expect("fixed run ends");
        assert!(result.is_ok(), "fixed: {result:?}");
        format!("fixed: {result:?}")
    } else {
        let run = execute_generated_v2_run(
            &store,
            run.clone(),
            plan,
            "test".into(),
            llm,
            ui,
            Vec::new(),
            true,
            false,
        );
        let result = tokio::time::timeout(ten, run).await.expect("run ends");
        assert!(result.is_ok(), "{result:?}");
        format!("{result:?}")
    };
    assert!(!report.contains("cancelled"), "{report}");
    let state = store.load_state(&run.id).expect("durable status");
    assert_eq!(state.status, expected, "{report}");
    if expected == RunStatus::NeedsReview {
        assert!(store.run_dir(&run.id).join("v2/finalization.json").exists());
    }
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let record = v2
        .load_call_record("slow")
        .expect("lookup")
        .expect("pending call recorded");
    assert_eq!(record.status, WorkflowV2Status::NeedsReview);
    assert_eq!(record.result.data["interrupted"], "terminal_host_stop");
    assert!(
        std::fs::read_dir(v2.root().join("inflight"))
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true)
    );
}

#[tokio::test]
async fn round7_restart_during_terminal_unwind_ends_needs_review() {
    edit_during_unwind(
        || LifecycleAction::RestartStage("call-1".into()),
        RunStatus::NeedsReview,
    )
    .await;
}

#[tokio::test]
async fn round7_force_accept_during_terminal_unwind_ends_needs_review() {
    edit_during_unwind(
        || LifecycleAction::ForceAcceptStage {
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
async fn round7_pause_during_terminal_unwind_keeps_pause_and_records_call() {
    edit_during_unwind(|| LifecycleAction::Pause, RunStatus::Paused).await;
}
