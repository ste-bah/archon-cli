//! Issue 261, round 5: a gate that reports an operational error (a truncated
//! judge reply, a preparation failure) PAUSES the fixed run with evidence; it
//! never fails it, so a resume can continue once the cause is repaired.
use super::*;

#[tokio::test]
async fn a_gate_operational_error_pauses_the_fixed_run_and_a_resume_continues() {
    let (temp, store, run_id, llm, host) = fixture();
    host.fixed.store(true, Ordering::SeqCst);
    host.operational.store(true, Ordering::SeqCst);
    let error = run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect_err("a persistent gate operational error pauses the run");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    assert_eq!(
        store.load_state(&run_id).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
    let paused = pause_events(&store, &run_id);
    assert_eq!(paused.len(), 1, "{paused:?}");
    let detail = &paused[0].detail;
    assert_eq!(detail["pause_id"], "pause-body-TASK-X-010-1", "{detail}");
    assert_eq!(
        detail["evidence"]["reason"], "operational_no_progress",
        "{detail}"
    );
    assert!(
        detail["evidence"]["last_findings"][0]
            .as_str()
            .is_some_and(|text| text.contains("judge response was truncated")),
        "{detail}"
    );

    host.operational.store(false, Ordering::SeqCst);
    resume(&store, &run_id);
    let summary = run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect("the resumed run continues");
    assert_eq!(summary.status, WorkflowV2Status::Accepted, "{summary:?}");
    assert_eq!(pause_events(&store, &run_id).len(), 1);
}
