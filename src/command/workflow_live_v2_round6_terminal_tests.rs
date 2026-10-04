//! A terminal deadline must not tear down a sibling's pending host work.
use super::*;

struct SlowReply;
#[async_trait::async_trait]
impl WorkflowLlmClient for SlowReply {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        // Longer than the test terminal budget. The sibling gate settles
        // concurrently while this host future is suspended.
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

async fn durable_terminal(script: &str) -> (tempfile::TempDir, WorkflowStore, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::project(temp.path());
    let run = store
        .create_run(super::super::workflow_run_finalizer_tests::spec())
        .expect("run");
    let plan = WorkflowScriptPlan::from_template(run.spec.clone(), script, Vec::new());
    save_generated_v2_metadata(&store, &run.id, &plan, false).expect("metadata");
    let (ui, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        execute_generated_v2_run(
            &store,
            run.clone(),
            plan,
            "test".into(),
            Arc::new(SlowReply),
            ui,
            Vec::new(),
            true,
            false,
        ),
    )
    .await
    .expect("terminal stop ends the run");
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(
        store.load_state(&run.id).expect("durable status").status,
        RunStatus::NeedsReview
    );
    assert!(store.run_dir(&run.id).join("v2/finalization.json").exists());
    (temp, store, run.id)
}

#[tokio::test]
async fn round6_terminal_deadline_records_pending_host_interruption() {
    let (_temp, store, id) = durable_terminal(
        r#"async function workflow(w) {
        await Promise.allSettled([
            w.agent("slow", {role: "analysis", task: "Inspect the area."}),
            w.humanGate("gate", {task: "Require approval"})
        ]);
        await new Promise(() => {});
    }"#,
    )
    .await;
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&id).join("v2"));
    let record = v2
        .load_call_record("slow")
        .expect("lookup")
        .expect("sibling recorded");
    assert_eq!(record.status, WorkflowV2Status::NeedsReview);
    assert_eq!(record.result.data["interrupted"], "terminal_host_stop");
    assert!(
        record.agent_session_id.is_some(),
        "the dispatched session must be collected"
    );
    assert!(
        super::super::workflow_live_v2_client::call_sessions::peek_sessions(&id, "slow").is_empty()
    );
    let run = store.load_state(&id).expect("durable state");
    assert_eq!(
        run.stages["slow"].status,
        archon_workflow::StageStatus::NeedsReview
    );
    assert!(
        run.stages
            .values()
            .all(|stage| stage.status != archon_workflow::StageStatus::Running)
    );
    assert!(
        std::fs::read_dir(v2.root().join("inflight"))
            .expect("markers")
            .next()
            .is_none()
    );
}

#[tokio::test]
async fn round6_terminal_cpu_watchdog_records_pending_host_interruption() {
    let (_temp, store, id) = durable_terminal(
        r#"async function workflow(w) {
        const sibling = w.agent("slow", {role: "analysis", task: "Inspect the area."});
        try { await w.humanGate("gate", {task: "Require approval"}); } catch (_) {}
        while (true) {}
    }"#,
    )
    .await;
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&id).join("v2"));
    let record = v2
        .load_call_record("slow")
        .expect("lookup")
        .expect("sibling recorded");
    assert_eq!(record.result.data["interrupted"], "terminal_host_stop");
    assert!(
        record.agent_session_id.is_some(),
        "the dispatched session must be collected"
    );
    assert!(
        super::super::workflow_live_v2_client::call_sessions::peek_sessions(&id, "slow").is_empty()
    );
    assert_eq!(
        store.load_state(&id).expect("state").stages["slow"].status,
        archon_workflow::StageStatus::NeedsReview
    );
}
