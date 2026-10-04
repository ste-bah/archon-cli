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

/// `workflow resume`: the real lifecycle transition, which advances the
/// generation the resumed executor owns.
fn resume(fixture: &Fixture) -> u64 {
    archon_workflow::LifecycleController::new(fixture.store.clone())
        .apply(&fixture.run_id, archon_workflow::LifecycleAction::Resume)
        .unwrap()
        .generation
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

    // Resumed after a fix: the generation advances, the interrupted call is
    // replayed (the same call id, as the next attempt of round 3) and
    // progresses.
    let resumed = resume(&fixture);
    assert_eq!(
        resumed,
        generation + 2,
        "pause, then resume, each advance it"
    );
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Running
    );
    std::fs::write(fixture.repo.path().join("missing"), "x").unwrap();
    let again = run(&fixture, &execution(3, 3, &["REQ-2"])).await.unwrap();
    assert_eq!(again.status, WorkflowV2Status::Accepted);
    assert_eq!(again.data["final"], false);
    assert_eq!(
        again.data["attempt"], 2,
        "the replayed call is the next attempt"
    );
    std::fs::write(fixture.repo.path().join("also-missing"), "x").unwrap();
    let clean = run(&fixture, &execution(4, 3, &["REQ-9"])).await.unwrap();
    assert_eq!(clean.status, WorkflowV2Status::Accepted);
    assert_eq!(clean.data["final"], true);
    assert!(failing_ids(&clean).is_empty());
}

/// Round 2 (P1): a round that started under one generation never pauses a
/// newer one. Here the operator pauses and resumes while round 3 runs (its
/// check advances the generation, as those transitions do); the round still
/// stalls, and the obsolete stage stops instead of pausing the new owner.
#[tokio::test]
async fn a_stalled_round_never_pauses_a_newer_generation() {
    // The hook lives outside both live roots, so it is no source of the
    // check: only what it does to the run matters.
    let hook = tempfile::tempdir().unwrap();
    let script = hook.path().join("bump.sh");
    let fixture = fixture_with(
        true,
        &format!("sh '{}' 2>/dev/null; test -f missing", script.display()),
    );
    run(&fixture, &execution(1, 3, &[])).await.unwrap();
    run(&fixture, &execution(2, 3, &[])).await.unwrap();
    let state_path = fixture.store.state_path(&fixture.run_id);
    std::fs::write(
        &script,
        format!(
            "perl -0pi -e 's/\"generation\": (\\d+)/q(\"generation\": ).($1+2)/e' '{}'\n",
            state_path.display()
        ),
    )
    .unwrap();
    let before = fixture.store.load_state(&fixture.run_id).unwrap();

    let error = run(&fixture, &execution(3, 3, &[]))
        .await
        .expect_err("the obsolete round stops");

    assert!(
        matches!(error, WorkflowError::ControlCancelled(_)),
        "never a pause of the newer generation: {error:?}"
    );
    let after = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(after.generation, before.generation + 2, "the check ran");
    assert_ne!(after.status, RunStatus::Paused);
    assert!(
        !events(&fixture)
            .iter()
            .any(|event| event["detail"]["event"] == "acceptance_stall_pause"),
        "no pause evidence for a pause that never happened"
    );
}

/// Round 3 (decision A): the failing sets a run reached, and how many rounds
/// in a row revisited one, are kept across a pause and resume. A paused
/// round's first attempt counts: its replay that revisits a set reached
/// before is the second revisit in a row, and pauses.
#[tokio::test]
async fn revisits_are_counted_across_attempts_of_a_round() {
    let fixture = fixture(true);
    let missing = fixture.repo.path().join("missing");
    // Round 1: REQ-2 and REQ-9 fail. Round 2: only REQ-9.
    run(&fixture, &execution(1, 3, &[])).await.unwrap();
    std::fs::write(&missing, "x").unwrap();
    run(&fixture, &execution(2, 3, &[])).await.unwrap();
    // Round 3 revisits round 1's set: the first revisit escalates.
    std::fs::remove_file(&missing).unwrap();
    let first = run(&fixture, &execution(3, 3, &[])).await.unwrap();
    assert_eq!(first.data["escalate"], true, "{first:#?}");
    // Its replay revisits round 2's set: the second revisit in a row.
    std::fs::write(&missing, "x").unwrap();
    let error = run(&fixture, &execution(3, 3, &[]))
        .await
        .expect_err("two revisits in a row are a stall");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
}
