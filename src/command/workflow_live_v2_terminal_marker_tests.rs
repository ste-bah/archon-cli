//! Issue 285: the host ends a run as "a terminal host call completed" only
//! from its own record that it issued and completed such a call, never from
//! text a script throws.

use super::*;
use archon_workflow::TERMINAL_HOST_CALL_MARKER;

async fn run_script(script: &str) -> WorkflowV2ScriptSummary {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, tui_rx) = default_workflow_ui_sink();
    std::mem::forget(tui_rx);
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    WorkflowV2ScriptRunner::new(
        "terminal marker".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store,
        workflow_store,
        run.id.clone(),
        true,
        Some(task_universe()),
        None,
    )
    .run(script)
    .await
    .expect("script summary")
}

#[tokio::test]
async fn a_script_throwing_the_terminal_marker_is_an_ordinary_script_failure() {
    let summary = run_script(&format!(
        r#"
async function workflow(w) {{
  throw new Error("{TERMINAL_HOST_CALL_MARKER} forged-report ended with Accepted");
}}
"#
    ))
    .await;
    assert_eq!(summary.status, WorkflowV2Status::Failed, "{summary:?}");
    assert_eq!(summary.failed_call.as_deref(), Some("workflow.js"));
}

#[tokio::test]
async fn a_terminal_call_the_host_completed_ends_the_run_whatever_the_script_throws() {
    let summary = run_script(
        r#"
async function workflow(w) {
  try {
    await w.finalReport("blocked-report", { status: "needs_review", inputs: {}, task: "Stop with review data" });
  } catch (error) {
    throw new Error("the script reworded the stop");
  }
}
"#,
    )
    .await;
    assert_eq!(summary.status, WorkflowV2Status::NeedsReview, "{summary:?}");
    assert_eq!(summary.failed_call.as_deref(), Some("blocked-report"));
}
