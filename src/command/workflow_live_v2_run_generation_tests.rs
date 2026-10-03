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
