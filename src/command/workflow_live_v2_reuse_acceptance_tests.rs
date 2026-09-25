//! An acceptance round is a measurement of the repository as it is NOW, so
//! the host never replays one from the store: a resume against a changed tree
//! must run the checks again.

use super::workflow_live_v2_reuse_content_key_tests::{reuse_test_runner, reuse_test_store};
use super::*;

const ACCEPTANCE_PROBE_SCRIPT: &str = r#"
async function workflow(w) {
  await w.tool("acceptance-contract-run-1", { tool: "acceptance-contract-run", round: 1, maxRounds: 1, checkIds: [] });
  return "done";
}
"#;

#[tokio::test]
async fn an_acceptance_round_is_never_served_from_the_store() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (workflow_store, run) = reuse_test_store(&temp);
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let args = serde_json::json!({});
    let first = reuse_test_runner(&workflow_store, &run, &v2_store, args.clone(), None)
        .run(ACCEPTANCE_PROBE_SCRIPT)
        .await
        .expect("first run");
    assert_eq!(first.executed, 1, "{first:?}");
    // Make the stored round look like a reusable pass: same input hash, same
    // scaffold, accepted. Only the call kind may stop the replay now.
    let mut record = v2_store
        .load_call_record("acceptance-contract-run-1")
        .expect("lookup")
        .expect("recorded");
    record.status = WorkflowV2Status::Accepted;
    record.result.status = WorkflowV2Status::Accepted;
    record.result.residual_gaps.clear();
    v2_store.save_call_record(&record).expect("save");
    let mut checkpoint = v2_store.load_checkpoint().unwrap().unwrap_or_default();
    checkpoint.mark_completed("acceptance-contract-run-1");
    v2_store.save_checkpoint(&checkpoint).unwrap();
    let resumed = reuse_test_runner(&workflow_store, &run, &v2_store, args, None)
        .with_frontier_resume(true)
        .run(ACCEPTANCE_PROBE_SCRIPT)
        .await
        .expect("resumed run");
    assert_eq!(
        resumed.reused, 0,
        "an acceptance round must re-run: {resumed:?}"
    );
    assert_eq!(resumed.executed, 1);
}
