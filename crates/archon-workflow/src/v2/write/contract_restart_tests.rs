use super::*;
use crate::v2::result_store::race_tests::on_branch_save;

#[test]
fn a_fanout_completion_in_flight_cannot_repopulate_a_restarted_branch() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let restart = store.clone().with_durable_writes();
    let runs = WorkflowStore::new(temp.path().parent().unwrap());
    let run_id = store.run_id();
    // The completion has already built its outcome when the restart interleaves.
    on_branch_save(move || {
        runs.with_run_lock(&run_id, |_| {
            restart.revoke_branch_outcome("fanout", "one")?;
            restart.bump_restart_epoch()?;
            Ok(())
        })
        .unwrap();
    });
    let saved = save_write_branch_outcome(
        &store,
        "fanout",
        "one",
        "worker",
        Some("input".into()),
        &WorkflowV2Result::accepted("completed"),
    );
    assert!(
        matches!(saved, Err(WorkflowError::ControlCancelled(_))),
        "{saved:?}"
    );
    assert!(
        store
            .load_branch_outcome("fanout", "one")
            .unwrap()
            .is_none()
    );
    assert!(store.load_superseded_branch_outcomes().is_empty());
}
