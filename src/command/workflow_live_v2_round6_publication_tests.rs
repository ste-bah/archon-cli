//! Lifecycle edits in the window after the dispatch check, before publication.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct ReplyCounter(AtomicUsize);
#[async_trait::async_trait]
impl WorkflowLlmClient for ReplyCounter {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        let dispatch = self.0.fetch_add(1, Ordering::SeqCst);
        let mut result = WorkflowV2Result::accepted(format!("dispatch {dispatch}"));
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

async fn publication_edit(action: LifecycleAction, expected: RunStatus, dispatches: usize) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::project(temp.path());
    let mut run = store
        .create_run(super::super::workflow_run_finalizer_tests::spec())
        .expect("run");
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
    store.save_state(&run).expect("state");
    let script = r#"async function workflow(w) {
        await w.agent("inspect-one", {role: "analysis", task: "Inspect the area."});
        return {};
    }"#;
    let plan = WorkflowScriptPlan::from_template(run.spec.clone(), script, Vec::new());
    save_generated_v2_metadata(&store, &run.id, &plan, false).expect("metadata");
    let controller = LifecycleController::new(store.clone());
    let id = run.id.clone();
    super::super::workflow_live_v2_fixed_persistence::publication_hook::install(
        store.run_dir(&id),
        Box::new(move || {
            controller
                .apply(&id, action)
                .expect("edit in publication window");
        }),
    );
    let llm = Arc::new(ReplyCounter(AtomicUsize::new(0)));
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
    assert_eq!(
        store.load_state(&run.id).expect("durable state").status,
        expected
    );
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    if dispatches == 2 {
        assert_eq!(
            llm.0.load(Ordering::SeqCst),
            2,
            "stale result must redispatch"
        );
        let record = v2
            .load_call_record("inspect-one")
            .expect("lookup")
            .expect("record");
        assert_eq!(record.result.summary, "dispatch 1");
        assert_eq!(v2.load_call_records().expect("records").len(), 1);
        assert!(
            v2.load_checkpoint()
                .expect("checkpoint")
                .expect("checkpoint exists")
                .completed_call_ids
                .contains(&"inspect-one".to_string())
        );
        let history = v2.root().join("results/superseded");
        assert!(
            !history.exists(),
            "pre-edit result must never reach history"
        );
    } else {
        let record = v2.load_call_record("inspect-one").expect("lookup");
        assert!(
            record.is_none(),
            "pre-stop answer must not be published: {record:?}"
        );
    }
}

#[tokio::test]
async fn round6_restart_at_publication_redispatches() {
    publication_edit(
        LifecycleAction::RestartStage("call-1".into()),
        RunStatus::Completed,
        2,
    )
    .await;
}
#[tokio::test]
async fn round6_item_restart_at_publication_redispatches() {
    publication_edit(
        LifecycleAction::RestartItem {
            stage_id: "call-1".into(),
            item_id: "item-1".into(),
        },
        RunStatus::Completed,
        2,
    )
    .await;
}
#[tokio::test]
async fn round6_force_accept_at_publication_redispatches() {
    publication_edit(
        LifecycleAction::ForceAcceptStage {
            stage_id: "call-1".into(),
            forced_by: "operator".into(),
            rationale: "reviewed".into(),
            source: "test".into(),
        },
        RunStatus::Completed,
        2,
    )
    .await;
}
#[tokio::test]
async fn round6_pause_at_publication_preserves_operator_stop() {
    publication_edit(LifecycleAction::Pause, RunStatus::Paused, 1).await;
}
#[tokio::test]
async fn round6_cancel_at_publication_preserves_operator_stop() {
    publication_edit(LifecycleAction::Cancel, RunStatus::Cancelled, 1).await;
}
