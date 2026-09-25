//! Review remediation replays by content across a shifted prelude ordinal:
//! the host path (`remediation_replay`) end to end, with an LLM that panics
//! if the replay is missed.

use super::workflow_live_v2_reuse_content_key_tests::reuse_test_store;
use super::*;

fn runner_with(
    llm: Arc<dyn WorkflowLlmClient>,
    workflow_store: &WorkflowStore,
    run: &archon_workflow::WorkflowRun,
    v2_store: &WorkflowV2ResultStore,
    args: serde_json::Value,
) -> WorkflowV2ScriptRunner {
    let (ui_sink, tui_rx) = default_workflow_ui_sink();
    std::mem::forget(tui_rx);
    let client = LiveV2AgentClient::new(llm, ui_sink, Vec::new(), run.id.clone(), None, None);
    WorkflowV2ScriptRunner::new(
        "remediation drift".to_string(),
        test_runtime(&test_spec()),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store.clone(),
        run.id.clone(),
        true,
        None,
        Some(args),
    )
}

/// One remediation call, named the way the prelude names it: a label that
/// carries the task and round, then the global ordinal. A fix-stage call:
/// a verdict replays only with its fix (`workflow_live_v2_reuse_verify_lineage_tests`).
const VERIFY_SCRIPT: &str = r#"
async function workflow(w) {
  const id = "review-verify-task-a-" + args.round + "-" + args.ordinal;
  await w.agent(id, {
    task: "Verify the fix for TASK-A (" + id + "): " + args.findings,
    remediationContract: { version: 1, stage: "remediate", taskId: "TASK-A", round: args.round, maxRounds: 2, sourceReduceCallIds: ["r"] },
  });
  return "done";
}
"#;

fn args(round: u64, ordinal: u64, findings: &str) -> serde_json::Value {
    serde_json::json!({ "round": round, "ordinal": ordinal, "findings": findings })
}

#[tokio::test]
async fn a_remediation_call_under_a_shifted_ordinal_replays_the_same_work() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (workflow_store, run) = reuse_test_store(&temp);
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let accepted: Arc<dyn WorkflowLlmClient> = Arc::new(SlowAcceptedLlm {
        delay: Duration::ZERO,
    });
    let first = runner_with(
        accepted,
        &workflow_store,
        &run,
        &v2_store,
        args(1, 32, "[f1]"),
    )
    .run(VERIFY_SCRIPT)
    .await
    .expect("first run");
    assert_eq!(first.executed, 1, "{first:?}");

    // An earlier task now takes two calls fewer: the same verifier arrives
    // as ordinal 30. PanicLlm fails the test if it is dispatched. A resume is
    // a new session: a new store over the same run directory.
    let resumed_store = WorkflowV2ResultStore::new(v2_store.root().to_path_buf());
    let resumed = runner_with(
        Arc::new(PanicLlm),
        &workflow_store,
        &run,
        &resumed_store,
        args(1, 30, "[f1]"),
    )
    .with_frontier_resume(true)
    .run(VERIFY_SCRIPT)
    .await
    .expect("resumed run");
    assert_eq!(resumed.reused, 1, "{resumed:?}");
    assert_eq!(resumed.executed, 0);
}

#[tokio::test]
async fn a_shifted_remediation_call_with_other_findings_or_round_runs_again() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (workflow_store, run) = reuse_test_store(&temp);
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let accepted: Arc<dyn WorkflowLlmClient> = Arc::new(SlowAcceptedLlm {
        delay: Duration::ZERO,
    });
    runner_with(
        accepted.clone(),
        &workflow_store,
        &run,
        &v2_store,
        args(1, 32, "[f1]"),
    )
    .run(VERIFY_SCRIPT)
    .await
    .expect("first run");
    for (round, findings) in [(1, "[f2]"), (2, "[f1]")] {
        let resumed_store = WorkflowV2ResultStore::new(v2_store.root().to_path_buf());
        let resumed = runner_with(
            accepted.clone(),
            &workflow_store,
            &run,
            &resumed_store,
            args(round, 30, findings),
        )
        .with_frontier_resume(true)
        .run(VERIFY_SCRIPT)
        .await
        .expect("resumed run");
        assert_eq!(resumed.reused, 0, "round {round} {findings}: {resumed:?}");
        assert_eq!(resumed.executed, 1);
    }
}

#[tokio::test]
async fn the_same_remediation_asked_twice_in_one_session_runs_twice() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (workflow_store, run) = reuse_test_store(&temp);
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let accepted: Arc<dyn WorkflowLlmClient> = Arc::new(SlowAcceptedLlm {
        delay: Duration::ZERO,
    });
    for ordinal in [32, 36] {
        let summary = runner_with(
            accepted.clone(),
            &workflow_store,
            &run,
            &v2_store,
            args(1, ordinal, "[f1]"),
        )
        .with_frontier_resume(true)
        .run(VERIFY_SCRIPT)
        .await
        .expect("run");
        assert_eq!(summary.executed, 1, "ordinal {ordinal}: {summary:?}");
    }
}
