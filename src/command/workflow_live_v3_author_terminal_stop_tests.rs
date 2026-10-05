//! Issue 293: a terminal host stop in an authored (v3) run ends in its own
//! stored state with its evidence. The executed-run checks judge only a
//! script that returned: a stopped one is never Failed/SpecInvalid, and a
//! missing result is never a pass.
use super::*;
use archon_workflow::{RunStatus, WorkflowAgentOutcome, WorkflowLlmClient};

/// Replies to every agent call with one fixed result status; with a
/// second status, a call whose prompt names the plan review gets that one.
struct FixedStatusAgent(&'static str, Option<&'static str>);

#[async_trait::async_trait]
impl WorkflowLlmClient for FixedStatusAgent {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let review = serde_json::to_string(&messages)
            .unwrap_or_default()
            .contains("Review the plan");
        let status = self.1.filter(|_| review).unwrap_or(self.0);
        let content = serde_json::json!({"status": status, "summary": "reviewed the plan",
            "evidence": [{"kind": "inspection", "summary": "read the plan"}]});
        Ok(WorkflowAgentOutcome {
            content: content.to_string(),
            tool_uses: vec![],
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: Some("end_turn".into()),
        })
    }
}

struct Stopped {
    _temp: tempfile::TempDir,
    store: WorkflowStore,
    run_id: String,
    v2_store: WorkflowV2ResultStore,
    summary: WorkflowV2ScriptSummary,
}

/// Runs the persisted `script` (it passes the dry-run pre-flight) through
/// the authored lifecycle with an agent that replies `status`.
async fn run_persisted(script: &str, status: &'static str) -> Stopped {
    run_persisted_with(script, FixedStatusAgent(status, None)).await
}

/// [`run_persisted`] with `agent` answering the calls.
async fn run_persisted_with(script: &str, agent: FixedStatusAgent) -> Stopped {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(spec.clone()).expect("run");
    let authored_path = store.run_dir(&run.id).join("authored-workflow.js");
    std::fs::write(&authored_path, script).expect("persist authored script");
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let agent = Arc::new(agent);
    let client = LiveV2AgentClient::new(agent, ui_sink, Vec::new(), run.id.clone(), None, None);
    let runner = WorkflowV2ScriptRunner::new(
        "terminal stop".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        store.clone(),
        run.id.clone(),
        true,
        None,
        None,
    );
    let summary = runner
        .run_authored_script_lifecycle(authored_path)
        .await
        .expect("a terminal stop is an outcome, never a SpecInvalid error");
    Stopped {
        _temp: temp,
        store,
        run_id: run.id,
        v2_store,
        summary,
    }
}

impl Stopped {
    /// Commits the summary as the live caller does; returns the stored status.
    async fn finalize(self) -> (RunStatus, WorkflowV2ScriptSummary) {
        let summary = super::super::super::super::workflow_live_v3_run_end::finalize_run(
            &self.store,
            &self.run_id,
            archon_workflow::WorkflowRunKind::AuthoredTaskWorkflow,
            None,
            self.summary,
            &self.v2_store,
        )
        .await
        .expect("finalize");
        let status = self.store.load_state(&self.run_id).expect("state").status;
        (status, summary)
    }
}

fn gated(catch: bool) -> String {
    let gate = r#"await w.humanGate("approval-gate", { task: "Require approval" });"#;
    let gate = if catch {
        format!("try {{ {gate} }} catch (_) {{ return {{ accepted: [], blocked: [] }}; }}")
    } else {
        gate.to_string()
    };
    AUTHORED_DEMO_SCRIPT.replace(
        r#"await phase("Authored Phase");"#,
        &format!("await phase(\"Authored Phase\");\n  {gate}"),
    )
}

#[tokio::test]
async fn an_unsatisfied_human_gate_ends_needs_review_naming_the_gate() {
    let stopped = run_persisted(&gated(false), "accepted").await;
    assert_eq!(stopped.summary.script_result, None);
    assert_eq!(
        stopped.summary.failed_call.as_deref(),
        Some("approval-gate")
    );
    let (status, summary) = stopped.finalize().await;
    assert_eq!(status, RunStatus::NeedsReview);
    assert_eq!(summary.status, WorkflowV2Status::NeedsReview);
    assert_eq!(summary.failed_call.as_deref(), Some("approval-gate"));
    let next = summary.next_action.unwrap_or_default();
    assert!(next.contains("approval-gate"), "{next}");
}

#[tokio::test]
async fn a_script_that_catches_a_terminal_stop_is_held_by_the_stop() {
    // The script returns past the stop; the host's own acceptance stage
    // (REM-13) is then refused, so the stopped call stays the evidence.
    let stopped = run_persisted(&gated(true), "accepted").await;
    assert_eq!(
        stopped.summary.failed_call.as_deref(),
        Some("approval-gate")
    );
    let (status, summary) = stopped.finalize().await;
    assert_eq!(status, RunStatus::NeedsReview);
    assert_eq!(summary.failed_call.as_deref(), Some("approval-gate"));
}

/// Issue 335: a worker whose own result says 'cancelled' did not cancel the
/// run -- only run control (an operator cancel) does. Its call is a failed
/// call the script handles like any other, and the script runs on.
#[tokio::test]
async fn a_worker_reporting_cancelled_is_a_failed_call_not_a_run_cancel() {
    let stopped = run_persisted_with(
        AUTHORED_DEMO_SCRIPT,
        FixedStatusAgent("accepted", Some("cancelled")),
    )
    .await;
    assert_eq!(
        stopped.summary.failed_call, None,
        "no terminal host stop: {:?}",
        stopped.summary
    );
    assert!(
        stopped.summary.script_result.is_some(),
        "the script ran on and returned: {:?}",
        stopped.summary
    );
    let review = stopped
        .v2_store
        .load_call_record("demo-review-1")
        .expect("lookup")
        .expect("the review call is recorded");
    assert_eq!(review.status, WorkflowV2Status::Failed, "{review:?}");
    assert_eq!(review.result.data["worker_reported_status"], "cancelled");
    let (status, summary) = stopped.finalize().await;
    assert_ne!(status, RunStatus::Cancelled, "{summary:?}");
    assert_ne!(summary.status, WorkflowV2Status::Cancelled, "{summary:?}");
}
