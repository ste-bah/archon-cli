//! Issue 265: the ordinal-drift reuse of a completed task's record holds a
//! remediation call to the same freshness checks as every other reuse path.
//! It answers only the question the record answered, and never answers a
//! remediation call with an implement record.

use super::workflow_live_v2_reuse_content_key_tests::reuse_test_store;
use super::*;

const SCRIPT: &str = r#"
async function workflow(w) {
  const options = { task: "Post-review remediation for TASK-A-001. Findings (verbatim): " + args.findings };
  if (args.remediation) {
    options.remediationContract = { version: 1, stage: "remediate", taskId: "TASK-A-001",
      round: 1, maxRounds: 1, sourceReduceCallIds: ["r"] };
  }
  await w.agent(args.label, options);
  return "done";
}
"#;

fn remediation(label: &str, findings: &str) -> serde_json::Value {
    serde_json::json!({ "label": label, "findings": findings, "remediation": true })
}

async fn session(
    llm: Arc<dyn WorkflowLlmClient>,
    workflow_store: &WorkflowStore,
    run: &archon_workflow::WorkflowRun,
    root: &std::path::Path,
    args: serde_json::Value,
    completed: &[&str],
) -> WorkflowV2ScriptSummary {
    let (ui_sink, tui_rx) = default_workflow_ui_sink();
    std::mem::forget(tui_rx);
    let client = LiveV2AgentClient::new(llm, ui_sink, Vec::new(), run.id.clone(), None, None);
    WorkflowV2ScriptRunner::new(
        "ordinal drift".to_string(),
        test_runtime(&test_spec()),
        WorkflowV2AgentAdapter::new(),
        client,
        WorkflowV2ResultStore::new(root.to_path_buf()),
        workflow_store.clone(),
        run.id.clone(),
        true,
        None,
        Some(args),
    )
    .with_frontier_resume(true)
    .with_resume_completed_ids(completed.iter().map(|id| id.to_string()).collect())
    .run(SCRIPT)
    .await
    .expect("session")
}

fn accepted() -> Arc<dyn WorkflowLlmClient> {
    Arc::new(SlowAcceptedLlm {
        delay: Duration::ZERO,
    })
}

#[tokio::test]
async fn a_shifted_remediation_call_with_other_findings_runs_again() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (workflow_store, run) = reuse_test_store(&temp);
    let root = workflow_store.run_dir(&run.id).join("v2");
    let first = session(
        accepted(),
        &workflow_store,
        &run,
        &root,
        remediation("remediate-task-a-001-r1-5", "[F1]"),
        &[],
    )
    .await;
    assert_eq!(first.executed, 1, "{first:?}");
    // The same question under a shifted ordinal is answered from the record.
    let same = session(
        Arc::new(PanicLlm),
        &workflow_store,
        &run,
        &root,
        remediation("remediate-task-a-001-r1-7", "[F1]"),
        &["TASK-A-001"],
    )
    .await;
    assert_eq!(same.reused, 1, "{same:?}");
    // Other findings are another question: the call runs.
    let other = session(
        accepted(),
        &workflow_store,
        &run,
        &root,
        remediation("remediate-task-a-001-r1-9", "[F2]"),
        &["TASK-A-001"],
    )
    .await;
    assert_eq!((other.executed, other.reused), (1, 0), "{other:?}");
}

#[tokio::test]
async fn an_implement_record_never_answers_a_remediation_call() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (workflow_store, run) = reuse_test_store(&temp);
    let root = workflow_store.run_dir(&run.id).join("v2");
    let implement = serde_json::json!({
        "label": "implement-task-a-001-3", "findings": "[F1]", "remediation": false,
    });
    let first = session(accepted(), &workflow_store, &run, &root, implement, &[]).await;
    assert_eq!(first.executed, 1, "{first:?}");
    let fix = session(
        accepted(),
        &workflow_store,
        &run,
        &root,
        remediation("remediate-task-a-001-r1-5", "[F1]"),
        &["TASK-A-001"],
    )
    .await;
    assert_eq!((fix.executed, fix.reused), (1, 0), "{fix:?}");
}
