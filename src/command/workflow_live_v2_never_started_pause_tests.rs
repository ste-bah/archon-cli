//! Issue 263: consecutive dispatches that never start pause the run with
//! evidence; they never stop it.

use archon_workflow::{RunStatus, StageStatus, WorkflowError, WorkflowStore};

use super::pause_never_started;

fn running_run(store: &WorkflowStore) -> String {
    let mut run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "never-started".into(),
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
    let mut stage = archon_workflow::run::StageState::pending("call-2");
    stage.status = StageStatus::Running;
    run.stages.insert("call-2".into(), stage);
    store.save_state(&run).unwrap();
    run.id
}

#[test]
fn a_never_started_streak_pauses_the_run_with_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run_id = running_run(&store);
    let generation = store.load_state(&run_id).unwrap().generation;
    let fault = WorkflowError::StateCorrupt("results/record.json: missing field".into());

    let error = pause_never_started(&store, &run_id, generation, "call-2", &fault, 2)
        .expect("the run is paused, not stopped");

    let WorkflowError::ControlPaused(message) = &error else {
        panic!("a never-started streak pauses: {error:?}");
    };
    assert!(message.contains("missing field"), "{message}");
    assert!(message.contains(&run_id), "{message}");
    let run = store.load_state(&run_id).unwrap();
    assert_eq!(run.status, RunStatus::Paused);
    assert_eq!(run.generation, generation + 1);
    assert_eq!(run.stages["call-2"].status, StageStatus::Paused);
    let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap();
    let pause: serde_json::Value = events
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["detail"]["event"] == "never_started_pause")
        .unwrap_or_else(|| panic!("no pause evidence: {events}"));
    assert_eq!(pause["kind"], "paused");
    assert_eq!(pause["detail"]["call_id"], "call-2");
    assert_eq!(pause["detail"]["consecutive"], 2);
    assert!(
        pause["detail"]["error"]
            .as_str()
            .is_some_and(|text| text.contains("missing field"))
    );
}

#[test]
fn a_run_already_paused_reports_that_pause() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run_id = running_run(&store);
    let mut run = store.load_state(&run_id).unwrap();
    let generation = run.generation;
    run.status = RunStatus::Paused;
    store.save_state(&run).unwrap();
    let fault = WorkflowError::StateCorrupt("unreadable".into());

    let error =
        pause_never_started(&store, &run_id, generation, "call-2", &fault, 2).expect("paused");

    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
}

/// Round 2 (P1): a dispatch of an obsolete executor never pauses the run a
/// newer generation owns after an operator pause and resume.
#[test]
fn a_stale_streak_never_pauses_a_newer_generation() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run_id = running_run(&store);
    let generation = store.load_state(&run_id).unwrap().generation;
    for action in [
        archon_workflow::LifecycleAction::Pause,
        archon_workflow::LifecycleAction::Resume,
    ] {
        archon_workflow::LifecycleController::new(store.clone())
            .apply(&run_id, action)
            .unwrap();
    }
    let owner = store.load_state(&run_id).unwrap();
    let fault = WorkflowError::StateCorrupt("unreadable".into());

    let error = pause_never_started(&store, &run_id, generation, "call-2", &fault, 2)
        .expect("the obsolete executor stops");

    assert!(
        matches!(error, WorkflowError::ControlCancelled(_)),
        "{error:?}"
    );
    let after = store.load_state(&run_id).unwrap();
    assert_eq!(after.status, RunStatus::Running);
    assert_eq!(after.generation, owner.generation);
}
