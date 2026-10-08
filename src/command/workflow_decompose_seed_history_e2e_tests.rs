//! L2 (Issue 360 review): a seeded run replays no attempt recorded before its
//! seed. Host commands are keyed by content and their occurrence slots start
//! again in every execution, so a seeded gate whose candidate is byte for
//! byte an old superseded gate's would otherwise answer from that old
//! record, an outage included.
use super::*;

fn gate_record(run: &Run, id: &str) -> archon_workflow::WorkflowV2CallRecord {
    WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"))
        .load_call_record(id)
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn a_seeded_gate_identical_to_an_old_superseded_one_is_judged_now() {
    let run = Run::new();
    run.authors.strong.store(true, Ordering::SeqCst);
    run.judge.outage.store(true, Ordering::SeqCst);
    let error = run
        .run(&earlier_script(), run.args())
        .await
        .expect_err("the outages pause");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    let outages = run.judge.runs("freeze-acceptance");
    let gates: Vec<String> = run
        .calls()
        .into_iter()
        .filter(|id| id.starts_with("judge-freeze-acceptance-"))
        .collect();
    assert!(
        gates.len() >= 2,
        "one candidate, judged in several slots: {gates:?}"
    );
    let first = gates
        .iter()
        .find(|id| !id.contains(":occurrence:"))
        .unwrap()
        .clone();
    let old = gate_record(&run, &first);
    assert!(!old.result.data["gateEnvelope"]["operational_error"].is_null());
    run.judge.outage.store(false, Ordering::SeqCst);
    let tasks = run.authors.tasks().len();
    let args = run.upgrade(&[("new-script", "next-rev")]);
    let summary = run
        .run(FIXED_SCRIPT_SOURCE, args)
        .await
        .expect("the seeded run completes");
    assert_eq!(summary.status, WorkflowV2Status::Accepted, "{summary:?}");
    assert!(
        acceptance_tasks(&run.authors.tasks()[tasks..]).is_empty(),
        "every entry is carried"
    );
    assert_eq!(run.judge.runs("freeze-acceptance"), outages + 1);
    let now = gate_record(&run, &first);
    assert!(
        now.attempt > old.attempt && now.result.data["gateEnvelope"]["operational_error"].is_null(),
        "the first seeded gate is judged by the new runtime, not the old outage: {:?}",
        now.result.data["gateEnvelope"]
    );
    for later in gates.iter().filter(|id| **id != first) {
        assert!(
            !gate_record(&run, later).result.data["gateEnvelope"]["operational_error"].is_null(),
            "the old outages stay as evidence: {later}"
        );
    }
}

#[path = "workflow_decompose_seed_context_e2e_tests.rs"]
mod context;
