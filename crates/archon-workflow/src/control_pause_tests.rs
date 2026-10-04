//! A stall pause belongs to the generation that observed the stall.

use super::*;

fn running(store: &WorkflowStore) -> (String, u64) {
    let mut run = store
        .create_run(crate::WorkflowSpec {
            schema: crate::spec::WORKFLOW_SCHEMA.into(),
            name: "pause".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    run.status = RunStatus::Running;
    store.save_state(&run).unwrap();
    (run.id, run.generation)
}

#[test]
fn the_owning_generation_pauses_the_run_with_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let (run_id, generation) = running(&store);
    pause_with_evidence(
        &store,
        &run_id,
        generation,
        serde_json::json!({"event": "x"}),
    )
    .unwrap()
    .unwrap();
    let run = store.load_state(&run_id).unwrap();
    assert_eq!(run.status, RunStatus::Paused);
    assert_eq!(run.generation, generation + 1);
}

#[test]
fn an_obsolete_generation_changes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let (run_id, generation) = running(&store);
    // The operator paused and resumed meanwhile.
    for action in [
        crate::LifecycleAction::Pause,
        crate::LifecycleAction::Resume,
    ] {
        crate::LifecycleController::new(store.clone())
            .apply(&run_id, action)
            .unwrap();
    }
    let owner = store.load_state(&run_id).unwrap();
    let error = pause_with_evidence(
        &store,
        &run_id,
        generation,
        serde_json::json!({"event": "x"}),
    )
    .expect_err("refused");
    assert!(
        matches!(error, WorkflowError::ControlCancelled(_)),
        "{error:?}"
    );
    let after = store.load_state(&run_id).unwrap();
    assert_eq!(after.status, RunStatus::Running);
    assert_eq!(after.generation, owner.generation);
}
