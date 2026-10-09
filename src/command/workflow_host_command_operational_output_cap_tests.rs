use super::*;

#[tokio::test]
async fn output_cap_stops_the_executor_with_a_resumable_pause_and_evidence() {
    let fixture = fixture(vec![Scripted::Operational(
        "host command 'task-set-lint' stdout output cap exceeded: cap=256 bytes, observed=300 bytes; resumable operational pause",
    )]);
    let error = fixture
        .executor
        .execute(lint(), Some(fixture.generation))
        .await
        .expect_err("output cap is an operational pause");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(run.status, RunStatus::Paused);
    assert_eq!(run.stages["host-call"].status, StageStatus::Paused);
    let emitted = events(&fixture);
    let pauses = events_named(&emitted, "host_command_registration_pause");
    assert_eq!(pauses.len(), 1);
    let detail = &pauses[0]["detail"];
    assert_eq!(detail["cause_reason"], detail["evidence"]);
    assert!(
        detail["cause_reason"]
            .as_str()
            .unwrap()
            .contains("cap=256 bytes")
    );
    assert!(
        detail["call_id"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "{detail}"
    );
}

#[test]
fn round3_growing_operational_progress_has_no_total_attempt_limit() {
    let mut history: Vec<_> = (1..=130).map(|n| attempt(n, Some(u64::from(n)))).collect();
    assert_eq!(next_step(&history), NextStep::Retry);
    history.push(attempt(131, Some(130)));
    assert_eq!(next_step(&history), NextStep::Pause("no_progress"));
}
