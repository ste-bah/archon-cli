use archon_workflow::{
    HeuristicWorkflowPlanner, LifecycleAction, LifecycleController, StageStatus, WorkflowPlanner,
    WorkflowStore,
};

#[test]
fn force_accept_records_event_and_quality_audit_file() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = HeuristicWorkflowPlanner.plan("Research a topic").unwrap();
    let mut run = store.create_run(spec).unwrap();
    run.stage_mut("discover").unwrap().status = StageStatus::Failed;
    store.save_state(&run).unwrap();

    let updated = LifecycleController::new(store.clone())
        .apply(
            &run.id,
            LifecycleAction::ForceAcceptStage {
                stage_id: "discover".into(),
                forced_by: "test".into(),
                rationale: "known acceptable fixture".into(),
                source: "unit-test".into(),
            },
        )
        .unwrap();
    assert_eq!(
        updated.stages.get("discover").unwrap().status,
        StageStatus::ForcedAccepted
    );
    let events = std::fs::read_to_string(store.events_path(&run.id)).unwrap();
    assert!(events.contains("forced_accepted"));
    assert!(events.contains("known acceptable fixture"));

    let forced = std::fs::read_to_string(
        store
            .run_dir(&run.id)
            .join("quality")
            .join("forced")
            .join("discover.json"),
    )
    .unwrap();
    assert!(forced.contains("known acceptable fixture"));
    assert!(forced.contains("forced_accepted"));
}
