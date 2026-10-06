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

/// What a lifecycle edit of a running run does: the executor kept, the
/// generation moved on. Returns the executor's launch generation.
fn edited_keeping_the_executor(store: &WorkflowStore, run_id: &str) -> u64 {
    let mut run = store.load_state(run_id).unwrap();
    let launch = run.generation;
    run.executor_generation = Some(launch);
    run.generation = launch + 1;
    store.save_state(&run).unwrap();
    launch
}

/// Issue 316: an executor's pause after an edit that kept it pauses the
/// run under the generation at the pause, never refused.
#[test]
fn an_executor_pauses_after_an_edit_that_kept_it() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let (run_id, _) = running(&store);
    let launch = edited_keeping_the_executor(&store, &run_id);

    pause_owned(
        &store,
        &run_id,
        PauseOwner::Executor(launch),
        serde_json::json!({"event": "x"}),
    )
    .expect("an edit that kept the executor refuses nothing")
    .unwrap();

    let run = store.load_state(&run_id).unwrap();
    assert_eq!(run.status, RunStatus::Paused);
    assert_eq!(run.generation, launch + 2);
    let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap();
    assert!(events.contains("\"paused_by_executor\""), "{events}");
}

/// A resume replaced the executor: its pause is refused and nothing changes.
#[test]
fn a_replaced_executor_never_pauses_the_new_owner() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let (run_id, launch) = running(&store);
    let mut run = store.load_state(&run_id).unwrap();
    run.generation = launch + 2;
    run.executor_generation = Some(launch + 2);
    store.save_state(&run).unwrap();

    let refused = pause_owned(
        &store,
        &run_id,
        PauseOwner::Executor(launch),
        serde_json::json!({}),
    );

    assert!(matches!(refused, Err(WorkflowError::ControlCancelled(_))));
    let after = store.load_state(&run_id).unwrap();
    assert_eq!(
        (after.status, after.generation),
        (run.status, run.generation)
    );
}

/// The exact-generation owner keeps its rule: an edit supersedes it.
#[test]
fn a_generation_owner_is_superseded_by_an_edit() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let (run_id, _) = running(&store);
    let launch = edited_keeping_the_executor(&store, &run_id);

    let refused = pause_with_evidence(&store, &run_id, launch, serde_json::json!({}));

    assert!(matches!(refused, Err(WorkflowError::ControlCancelled(_))));
    assert_eq!(
        store.load_state(&run_id).unwrap().status,
        RunStatus::Running
    );
}

/// No executor to check: the pause takes the run's generation at the pause,
/// and an operator's pause already made stands.
#[test]
fn an_unfenced_pause_takes_the_generation_at_the_pause() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let (run_id, _) = running(&store);
    let launch = edited_keeping_the_executor(&store, &run_id);

    let owner = PauseOwner::of_executor(None);
    pause_owned(&store, &run_id, owner, serde_json::json!({}))
        .unwrap()
        .unwrap();
    let again = pause_owned(&store, &run_id, owner, serde_json::json!({}));

    assert_eq!(store.load_state(&run_id).unwrap().generation, launch + 2);
    assert!(matches!(again, Err(WorkflowError::ControlPaused(_))));
}
