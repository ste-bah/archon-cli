//! Repeated same-phase publication must leave the pin bound to the last commit.
//!
//! The decomposition's phase loop now re-authors and re-publishes when a
//! committed artifact still carries a repairable finding, so a phase can commit
//! more than once. Everything downstream reads the pin, so if the pin bound to
//! an earlier publication the repaired contract would be published and then
//! ignored — a silent regression of the repair loop itself.

use super::*;

#[tokio::test]
async fn a_second_freeze_rebinds_the_lock_and_pin_to_the_latest_contract() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, _original) = seed(&temp);

    let first = freeze_acceptance(
        temp.path(),
        &tasks,
        &prd,
        Arc::new(JudgeClient {
            result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"first attempt","reason":"first reason"}]}"#.into()),
        }),
    )
    .await
    .unwrap();

    let path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let changed = std::fs::read_to_string(&path).unwrap().replace(
        "jq -e '.valid == true' out.json", "jq -e '.valid == true and .count > 0' out.json");
    std::fs::write(&path, changed).unwrap();

    let second = freeze_acceptance(
        temp.path(),
        &tasks,
        &prd,
        Arc::new(JudgeClient {
            result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"second attempt","reason":"second reason"}]}"#.into()),
        }),
    )
    .await
    .unwrap();

    assert_ne!(
        first.acceptance_digest, second.acceptance_digest,
        "the two publications must differ, or this proves nothing"
    );

    let frozen = std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    assert_eq!(
        archon_workflow::task_set_contract::content_digest(&frozen),
        second.acceptance_digest,
        "the contract on disk must be the last one published"
    );

    let lock: archon_workflow::task_set_contract::AcceptanceLock =
        serde_json::from_slice(&std::fs::read(tasks.join(ACCEPTANCE_LOCK_FILE)).unwrap()).unwrap();
    assert_eq!(
        lock.digest, second.acceptance_digest,
        "the lock must bind to the last publication, not the first"
    );

    let pin: AcceptancePin =
        serde_json::from_slice(&std::fs::read(acceptance_pin_path(temp.path(), &tasks)).unwrap())
            .unwrap();
    assert_eq!(
        pin.acceptance_digest, second.acceptance_digest,
        "the pin must bind to the last publication, not the first"
    );
    assert_eq!(
        pin.freeze_event_id, second.freeze_event_id,
        "the pin must name the freeze event that produced the contract on disk"
    );

    let text = String::from_utf8_lossy(&frozen);
    assert!(
        text.contains("second reason") && !text.contains("first reason"),
        "the repaired contract must replace the earlier one outright: {text}"
    );
}
