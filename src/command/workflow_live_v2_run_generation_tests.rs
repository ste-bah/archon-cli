use super::*;

struct ResumeBeforeReply {
    store: WorkflowStore,
    run_id: String,
    status: WorkflowV2Status,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for ResumeBeforeReply {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        let lifecycle = LifecycleController::new(self.store.clone());
        lifecycle.apply(&self.run_id, LifecycleAction::Pause)?;
        lifecycle.apply(&self.run_id, LifecycleAction::Resume)?;
        let result = WorkflowV2Result {
            status: self.status,
            summary: "reply from the obsolete executor".into(),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Inspection,
                "read",
            )],
            ..WorkflowV2Result::default()
        };
        Ok(archon_workflow::WorkflowAgentOutcome {
            content: serde_json::to_string(&result).unwrap(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: None,
        })
    }
}

#[tokio::test]
async fn round3_obsolete_generated_executor_cannot_finalize_resumed_run() {
    for status in [WorkflowV2Status::Accepted, WorkflowV2Status::Failed] {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(temp.path());
        let run = store
            .create_run(super::super::workflow_run_finalizer_tests::spec())
            .unwrap();
        let script = r#"async function workflow(w) {
            await w.agent("inspect-one", {role: "analysis", task: "Inspect the area and report."});
            return {};
        }"#;
        let plan = WorkflowScriptPlan::from_template(run.spec.clone(), script, Vec::new());
        save_generated_v2_metadata(&store, &run.id, &plan, false).unwrap();
        let llm = Arc::new(ResumeBeforeReply {
            store: store.clone(),
            run_id: run.id.clone(),
            status,
        });
        let (ui, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
        let result = execute_generated_v2_run(
            &store,
            run.clone(),
            plan,
            "test".into(),
            llm,
            ui,
            Vec::new(),
            true,
            false,
        )
        .await;
        let current = store.load_state(&run.id).unwrap();
        assert_eq!(current.status, RunStatus::Running, "{status:?}: {result:?}");
        assert_eq!(current.generation, run.generation + 2);
        assert!(
            !store.run_dir(&run.id).join("v2/finalization.json").exists(),
            "{result:?}"
        );
        let events = std::fs::read_to_string(store.events_path(&run.id)).unwrap();
        assert!(!events.contains("terminal_status"), "{events}");
    }
}

/// The operator acts once, while the first dispatch of the call is in flight.
struct ActionBeforeReply {
    store: WorkflowStore,
    run_id: String,
    action: LifecycleAction,
    dispatches: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for ActionBeforeReply {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        let dispatch = self
            .dispatches
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if dispatch == 0 {
            LifecycleController::new(self.store.clone())
                .apply(&self.run_id, self.action.clone())?;
        }
        let result = WorkflowV2Result {
            status: WorkflowV2Status::Accepted,
            summary: format!("inspection completed by dispatch {dispatch}"),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Inspection,
                "read the requested area",
            )],
            ..WorkflowV2Result::default()
        };
        Ok(archon_workflow::WorkflowAgentOutcome {
            content: serde_json::to_string(&result)?,
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: None,
        })
    }
}

async fn assert_running_action_finalizes(action: LifecycleAction) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let mut spec = super::super::workflow_run_finalizer_tests::spec();
    let mut second = spec.stages[0].clone();
    second.id = "call-2".into();
    spec.stages.push(second);
    let mut run = store.create_run(spec).unwrap();
    run.status = RunStatus::Running;
    run.items.insert(
        "item-1".into(),
        archon_workflow::run::ItemState {
            id: "item-1".into(),
            stage_id: "call-1".into(),
            status: archon_workflow::StageStatus::Pending,
            artifact: None,
            error: None,
        },
    );
    store.save_state(&run).unwrap();
    let script = r#"async function workflow(w) {
        await w.agent("inspect-one", {role: "analysis", task: "Inspect the area and report."});
        return {};
    }"#;
    let plan = WorkflowScriptPlan::from_template(run.spec.clone(), script, Vec::new());
    save_generated_v2_metadata(&store, &run.id, &plan, false).unwrap();
    let llm = Arc::new(ActionBeforeReply {
        store: store.clone(),
        run_id: run.id.clone(),
        action,
        dispatches: Default::default(),
    });
    let (ui, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let result = execute_generated_v2_run(
        &store,
        run.clone(),
        plan,
        "test".into(),
        llm.clone(),
        ui,
        Vec::new(),
        true,
        false,
    )
    .await;
    assert!(result.is_ok(), "{result:?}");
    // Round 5: the call in flight across the edit is fenced. Its pre-edit
    // result is discarded and the owning executor dispatches it again.
    assert_eq!(
        llm.dispatches.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the pre-edit result must not be published"
    );
    let record = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"))
        .load_call_record("inspect-one")
        .unwrap()
        .expect("the re-dispatched call is recorded");
    assert_eq!(record.result.summary, "inspection completed by dispatch 1");
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        RunStatus::Completed,
        "{result:?}"
    );
    assert!(store.run_dir(&run.id).join("v2/finalization.json").exists());
    assert!(
        std::fs::read_to_string(store.events_path(&run.id))
            .unwrap()
            .contains("terminal_status")
    );
}

#[tokio::test]
async fn round5_restart_stage_fences_the_call_and_keeps_the_executor_owner() {
    assert_running_action_finalizes(LifecycleAction::RestartStage("call-1".into())).await;
}

#[tokio::test]
async fn round5_restart_item_fences_the_call_and_keeps_the_executor_owner() {
    assert_running_action_finalizes(LifecycleAction::RestartItem {
        stage_id: "call-1".into(),
        item_id: "item-1".into(),
    })
    .await;
}

#[tokio::test]
async fn round5_force_accept_stage_fences_the_call_and_keeps_the_executor_owner() {
    assert_running_action_finalizes(LifecycleAction::ForceAcceptStage {
        stage_id: "call-1".into(),
        forced_by: "operator".into(),
        rationale: "reviewed".into(),
        source: "test".into(),
    })
    .await;
}
