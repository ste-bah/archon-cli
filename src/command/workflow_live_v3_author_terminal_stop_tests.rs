//! Issue 293: a terminal host stop in an authored (v3) run ends in its own
//! stored state with its evidence. The executed-run checks judge only a
//! script that returned: a stopped one is never Failed/SpecInvalid, and a
//! missing result is never a pass.
use super::*;
use archon_workflow::{RunStatus, WorkflowAgentOutcome, WorkflowLlmClient};

/// Replies to every agent call with one fixed result status.
struct FixedStatusAgent(&'static str);

#[async_trait::async_trait]
impl WorkflowLlmClient for FixedStatusAgent {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let content = serde_json::json!({"status": self.0, "summary": "reviewed the plan",
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
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(spec.clone()).expect("run");
    let authored_path = store.run_dir(&run.id).join("authored-workflow.js");
    std::fs::write(&authored_path, script).expect("persist authored script");
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let agent = Arc::new(FixedStatusAgent(status));
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

#[tokio::test]
async fn a_cancelled_call_ends_the_run_cancelled() {
    let stopped = run_persisted(AUTHORED_DEMO_SCRIPT, "cancelled").await;
    assert_eq!(stopped.summary.script_result, None);
    assert_eq!(
        stopped.summary.failed_call.as_deref(),
        Some("demo-review-1")
    );
    let (status, summary) = stopped.finalize().await;
    assert_eq!(status, RunStatus::Cancelled);
    assert_eq!(summary.status, WorkflowV2Status::Cancelled);
}
