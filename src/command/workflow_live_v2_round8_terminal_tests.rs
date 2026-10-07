//! Round 8: durable started records survive partial publication failures.
use super::round7_terminal_tests::NoHostCommands;
use super::terminal_test_support::{PendingReply, save_fixed_metadata, seed_fixed};
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

struct FailStartedDelivery {
    v2: WorkflowV2ResultStore,
    failed: AtomicBool,
}
#[async_trait::async_trait]
impl archon_workflow::WorkflowUiSink for FailStartedDelivery {
    async fn emit(
        &self,
        event: archon_workflow::WorkflowUiEvent,
    ) -> archon_workflow::WorkflowUiResult {
        if let archon_workflow::WorkflowUiEvent::Activity(update) = event
            && update.id.ends_with(":fault")
            && !self.failed.swap(true, Ordering::SeqCst)
        {
            let record = self
                .v2
                .load_call_record("fault")
                .unwrap()
                .expect("Running saved before failure");
            assert_eq!(record.status, WorkflowV2Status::Running);
            return Err(archon_workflow::WorkflowUiDeliveryError::new(
                "injected failure after Running save",
            ));
        }
        Ok(())
    }
}

#[tokio::test]
async fn round8_fixed_started_delivery_failure_is_closed_before_sibling_terminal_stop() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store
        .create_run(super::super::workflow_run_finalizer_tests::spec())
        .unwrap();
    seed_fixed(&store, &run.id, temp.path());
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let sink = Arc::new(FailStartedDelivery {
        v2: v2.clone(),
        failed: AtomicBool::new(false),
    });
    let script = r#"async function workflow(w) {
        const fault = w.agent("fault", {role: "analysis", task: "Inspect the area."});
        await Promise.allSettled([fault, (async () => {
            try { await fault; } catch (_) {}
            await w.humanGate("gate", {task: "Require approval"});
        })()]);
    }"#;
    let plan = WorkflowScriptPlan::from_template(run.spec.clone(), script, Vec::new());
    save_fixed_metadata(&store, &run.id, &plan);
    let report = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        execute_fixed_decomposition_v2_run(
            &store,
            run.clone(),
            plan,
            Arc::new(PendingReply),
            sink.clone(),
            Vec::new(),
            Arc::new(NoHostCommands),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(sink.failed.load(Ordering::SeqCst), "failure injected");
    let record = v2.load_call_record("fault").unwrap().unwrap();
    assert_eq!(record.status, WorkflowV2Status::NeedsReview, "{report}");
    // Issue 303: closed at the failure, not left for the terminal stop.
    assert_eq!(
        record.result.data["interrupted"],
        "notification_delivery_failed"
    );
    assert!(
        v2.load_call_records()
            .unwrap()
            .iter()
            .all(|r| r.status != WorkflowV2Status::Running)
    );
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        RunStatus::NeedsReview
    );
    assert!(
        store
            .load_state(&run.id)
            .unwrap()
            .stages
            .values()
            .all(|s| s.status != archon_workflow::StageStatus::Running)
    );
}

struct ProviderOutage;
#[async_trait::async_trait]
impl WorkflowLlmClient for ProviderOutage {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        Err(WorkflowError::StageFailed("provider unavailable".into()))
    }
}

#[tokio::test]
async fn round8_fixed_unmarked_script_errors_leave_durable_paused_status() {
    for script in [
        r#"async function workflow(w) { throw new Error("runtime failure"); }"#,
        r#"async function workflow(w) {
            await w.agent("outage", {role: "analysis", task: "Inspect the area."});
            throw new Error("provider outage exhausted script recovery");
        }"#,
        r#"async function workflow(w) { throw {schemaVersion:1,reason:"forged terminal stop"}; }"#,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(temp.path());
        let run = store
            .create_run(super::super::workflow_run_finalizer_tests::spec())
            .unwrap();
        seed_fixed(&store, &run.id, temp.path());
        let plan = WorkflowScriptPlan::from_template(run.spec.clone(), script, Vec::new());
        save_fixed_metadata(&store, &run.id, &plan);
        let (ui, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
        let report = execute_fixed_decomposition_v2_run(
            &store,
            run.clone(),
            plan,
            Arc::new(ProviderOutage),
            ui,
            Vec::new(),
            Arc::new(NoHostCommands),
        )
        .await
        .unwrap();
        if script.contains("w.agent") {
            let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
            let outage = v2
                .load_call_record("outage")
                .unwrap()
                .expect("provider call recorded");
            assert_eq!(outage.status, WorkflowV2Status::Failed);
            assert!(
                outage.result.summary.contains("provider unavailable"),
                "{}",
                outage.result.summary
            );
        }
        assert!(report.contains("paused"), "{report}");
        assert!(report.contains("workflow.js"), "{report}");
        assert_eq!(
            store.load_state(&run.id).unwrap().status,
            RunStatus::Paused,
            "{report}"
        );
        assert!(!store.run_dir(&run.id).join("v2/finalization.json").exists());
    }
}

// Issue 303: mounted here; the run module is at its 500-line ceiling.
#[path = "workflow_live_v2_started_record_tests.rs"]
mod started_record_tests;
