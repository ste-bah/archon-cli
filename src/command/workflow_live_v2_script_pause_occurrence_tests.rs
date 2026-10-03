//! Repeated content identifies an artifact, not the evaluation occurrence.
use super::*;

#[tokio::test]
async fn round3_distinct_evaluations_of_identical_content_replay_in_order() {
    let (temp, store, run_id, llm, host) = fixture();
    llm.repeat_body.store(true, Ordering::SeqCst);
    host.refuse.store(true, Ordering::SeqCst);
    host.varying.store(true, Ordering::SeqCst);
    run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect_err("one baseline, one novel finding and three repeats pause");
    assert_eq!(host.lands.load(Ordering::SeqCst), 5);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 7);
    host.fixed.store(true, Ordering::SeqCst);
    resume(&store, &run_id);
    let result = run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect("historical evaluations replay without changing author prompts");
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 8, "only one fresh author");
    assert!(
        record_exists(&store, &run_id, "body-TASK-X-010-author-6"),
        "the new author must follow all five historical occurrences"
    );
    let records = WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"));
    assert_eq!(
        records
            .load_call_record("body-TASK-X-010-author-2")
            .unwrap()
            .unwrap()
            .attempt,
        1,
        "replaying the first evaluation must preserve the second author prompt"
    );
    assert_eq!(
        host.lands.load(Ordering::SeqCst),
        6,
        "only one fresh evaluation"
    );
}

#[tokio::test]
async fn round3_fresh_identical_candidate_after_pause_gets_a_fresh_evaluation() {
    let (temp, store, run_id, llm, host) = fixture();
    llm.repeat_body.store(true, Ordering::SeqCst);
    host.refuse.store(true, Ordering::SeqCst);
    run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect_err("repeated findings pause");
    assert_eq!(host.lands.load(Ordering::SeqCst), 4);
    host.fixed.store(true, Ordering::SeqCst);
    resume(&store, &run_id);
    let result = run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect("a fresh occurrence must not replay the covered refusal");
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(host.lands.load(Ordering::SeqCst), 5);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 7);
    assert_eq!(pause_events(&store, &run_id).len(), 1);
}
