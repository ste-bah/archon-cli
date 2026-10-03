//! A2 / Issue 262: no round count ends the acceptance loop, and no stall
//! ends it either. The same failures seen again escalate; a second time the
//! run PAUSES with evidence (never `NeedsReview`); a clean round is final.

use super::*;

fn events(fixture: &Fixture) -> Vec<serde_json::Value> {
    std::fs::read_to_string(fixture.store.events_path(&fixture.run_id))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// What `workflow resume` does to the run's status.
fn resume(fixture: &Fixture) {
    let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
    run.status = RunStatus::Running;
    fixture.store.save_state(&run).unwrap();
}

#[tokio::test]
async fn the_loop_pauses_on_no_progress_and_ends_only_on_a_clean_round() {
    let fixture = fixture(true);
    let first = run(&fixture, &execution(1, 3, &[])).await.unwrap();
    assert_eq!(first.data["final"], false);
    let second = run(&fixture, &execution(2, 3, &[])).await.unwrap();
    assert_eq!(second.data["final"], false);
    assert_eq!(second.data["escalate"], true);
    let generation = fixture
        .store
        .load_state(&fixture.run_id)
        .unwrap()
        .generation;

    let error = run(&fixture, &execution(3, 3, &[]))
        .await
        .expect_err("a stall pauses the run, never ends it");
    let WorkflowError::ControlPaused(message) = &error else {
        panic!("a stall pauses, never fails: {error:?}");
    };
    assert!(message.contains("no progress"), "{message}");
    assert!(message.contains(&fixture.run_id), "{message}");
    let state = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(state.status, RunStatus::Paused);
    assert_eq!(state.generation, generation + 1);
    // The evidence: the round's record, and a pause event naming it.
    let (record, _) = latest_round_record(&fixture.store.run_dir(&fixture.run_id))
        .unwrap()
        .expect("the stalled round is recorded");
    assert_eq!(record.round, 3);
    assert!(!record.final_round, "the loop is not over, only paused");
    let events = events(&fixture);
    let pause = events
        .iter()
        .find(|event| event["detail"]["event"] == "acceptance_stall_pause")
        .unwrap_or_else(|| panic!("no pause evidence: {events:#?}"));
    assert_eq!(pause["kind"], "paused");
    assert_eq!(pause["detail"]["round"], 3);
    assert_eq!(pause["detail"]["cause"], "no_progress");
    assert_eq!(pause["detail"]["stalled_rounds"], 2);
    assert!(
        pause["detail"]["failing_check_ids"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|id| id == "REQ-2")),
        "{pause:#?}"
    );
    assert!(pause["detail"]["record_path"].is_string(), "{pause:#?}");

    // Resumed after a fix: the paused round runs again and progresses.
    resume(&fixture);
    std::fs::write(fixture.repo.path().join("missing"), "x").unwrap();
    let again = run(&fixture, &execution(3, 3, &["REQ-2"])).await.unwrap();
    assert_eq!(again.status, WorkflowV2Status::Accepted);
    assert_eq!(again.data["final"], false);
    std::fs::write(fixture.repo.path().join("also-missing"), "x").unwrap();
    let clean = run(&fixture, &execution(4, 3, &["REQ-9"])).await.unwrap();
    assert_eq!(clean.status, WorkflowV2Status::Accepted);
    assert_eq!(clean.data["final"], true);
    assert!(failing_ids(&clean).is_empty());
}
