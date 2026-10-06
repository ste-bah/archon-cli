//! Issue 337: a validated terminal stop outranks every pause a call of its
//! generation would take; an operator's lifecycle edit, and a run end after
//! the script settled, keep their pause.

use super::*;

fn running(store: &WorkflowStore) -> (String, u64) {
    let mut run = store
        .create_run(crate::WorkflowSpec {
            schema: crate::spec::WORKFLOW_SCHEMA.into(),
            name: "terminal-stop".into(),
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

/// The record the script host writes under the run lock when it validates a
/// deliberate stop (written raw here: the format is the contract).
fn stop_at(store: &WorkflowStore, run_id: &str, generation: u64) {
    store
        .write_run_json(
            run_id,
            "v2/terminal-stop.json",
            &serde_json::json!({"generation": generation, "reason": "deliberate gate refusal"}),
        )
        .unwrap();
}

fn operator(store: &WorkflowStore, run_id: &str, actions: &[crate::LifecycleAction]) {
    for action in actions {
        crate::LifecycleController::new(store.clone())
            .apply(run_id, action.clone())
            .unwrap();
    }
}

#[test]
fn a_call_of_the_stopped_generation_never_pauses_the_run() {
    for unfenced in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(temp.path());
        let (run_id, generation) = running(&store);
        stop_at(&store, &run_id, generation);
        let owner = if unfenced {
            PauseOwner::Unfenced
        } else {
            PauseOwner::Generation(generation)
        };

        let refused = pause_owned(&store, &run_id, owner, serde_json::json!({"event": "x"}));

        assert!(
            matches!(&refused, Err(WorkflowError::ControlCancelled(message)) if message.contains("stopped terminally")),
            "{owner:?}: {refused:?}"
        );
        let after = store.load_state(&run_id).unwrap();
        assert_eq!(
            (after.status, after.generation),
            (RunStatus::Running, generation)
        );
        let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap_or_default();
        assert!(
            !events
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .any(|event| event["kind"] == "paused"),
            "no pause evidence: {events}"
        );
    }
}

#[test]
fn a_stale_stop_or_a_run_end_pause_is_not_refused() {
    // The stopped generation's own calls are refused (the defect)...
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let (run_id, generation) = running(&store);
    stop_at(&store, &run_id, generation);
    assert!(pause_with_evidence(&store, &run_id, generation, serde_json::json!({})).is_err());
    // ...a run end of the executor, after its script settled, still pauses.
    pause_owned(
        &store,
        &run_id,
        PauseOwner::Executor(generation),
        serde_json::json!({}),
    )
    .expect("a run-end pause is not a sibling of the stop")
    .unwrap();
    assert_eq!(store.load_state(&run_id).unwrap().status, RunStatus::Paused);

    // An operator's pause and resume moved the generation: the stop is stale.
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let (run_id, generation) = running(&store);
    stop_at(&store, &run_id, generation);
    operator(
        &store,
        &run_id,
        &[
            crate::LifecycleAction::Pause,
            crate::LifecycleAction::Resume,
        ],
    );
    let resumed = store.load_state(&run_id).unwrap().generation;
    pause_with_evidence(&store, &run_id, resumed, serde_json::json!({}))
        .expect("a stale stop refuses nothing")
        .unwrap();

    // An unreadable record is evidence lost: it refuses nothing.
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let (run_id, generation) = running(&store);
    std::fs::create_dir_all(store.run_dir(&run_id).join("v2")).unwrap();
    std::fs::write(store.run_dir(&run_id).join("v2/terminal-stop.json"), b"{").unwrap();
    pause_with_evidence(&store, &run_id, generation, serde_json::json!({}))
        .expect("an unreadable stop record refuses nothing")
        .unwrap();
}
