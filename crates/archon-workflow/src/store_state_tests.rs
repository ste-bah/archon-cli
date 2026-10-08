use super::*;

#[test]
fn every_state_rename_syncs_the_containing_directory() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("runs"));
    let spec = WorkflowSpec::from_yaml(
        "schema: archon.workflow.v1\nname: state-sync\ntask: state-sync\nstages:\n  - id: stage-a\n    kind: agent\n",
    )
    .unwrap();
    let run = store.create_run(spec).unwrap();
    let parent = store.state_path(&run.id).parent().unwrap().to_path_buf();

    crate::durable_io::take_synced();
    store.save_state(&run).unwrap();
    assert!(crate::durable_io::take_synced().contains(&parent));

    crate::durable_io::take_synced();
    store.save_state_preserving_control(&run).unwrap();
    assert!(crate::durable_io::take_synced().contains(&parent));

    let failed_generation = run.generation + 1;
    let mut failed = run.clone();
    failed.generation = failed_generation;
    store.save_state(&failed).unwrap();
    crate::durable_io::take_synced();
    store
        .restore_state_after_failed_transition(&run, failed_generation)
        .unwrap();
    assert!(crate::durable_io::take_synced().contains(&parent));
}
